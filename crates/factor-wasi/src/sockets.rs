//! Socket quota tracking and WASI socket host implementations.
//!
//! This module provides [`SocketPermitState`], [`SpinSocketsView`], and
//! [`SpinSockets`] — the types needed to intercept WASI TCP/UDP socket
//! creation and enforce a per-app cap on the number of concurrently open
//! sockets.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use spin_connection_semaphore::{ConnectionPermit, ConnectionSemaphore};
use wasmtime::component::{HasData, Resource};
use wasmtime_wasi::p2::bindings::sockets::network::{
    ErrorCode as SocketErrorCode, Host as NetworkHost, Network,
};
use wasmtime_wasi::p2::bindings::sockets::tcp::{self as p2_tcp, IpSocketAddress, ShutdownType};
use wasmtime_wasi::p2::bindings::sockets::tcp_create_socket as p2_tcp_create;
use wasmtime_wasi::p2::bindings::sockets::udp as p2_udp;
use wasmtime_wasi::p2::bindings::sockets::udp_create_socket as p2_udp_create;
use wasmtime_wasi::p2::{DynInputStream, DynOutputStream, DynPollable};
use wasmtime_wasi::sockets::{TcpSocket, UdpSocket, WasiSocketsCtxView};

/// Shared state for tracking per-socket semaphore permits. Permits are
/// acquired when a socket is allocated (at `start_connect` for TCP, at
/// `create_udp_socket` for UDP) and released when the socket resource is dropped.
pub struct SocketPermitState {
    semaphore: ConnectionSemaphore,
    /// Active permits keyed by socket resource rep, released when the resource is dropped.
    active: Mutex<HashMap<u32, ConnectionPermit>>,
}

impl SocketPermitState {
    pub fn new(semaphore: ConnectionSemaphore) -> Arc<Self> {
        Arc::new(Self {
            semaphore,
            active: Mutex::new(HashMap::new()),
        })
    }
}

/// A view over WASI socket state that carries an optional per-instance socket
/// permit store, enabling per-connection quota tracking.
pub struct SpinSocketsView<'a> {
    pub(crate) inner: WasiSocketsCtxView<'a>,
    pub(crate) permit_state: Option<Arc<SocketPermitState>>,
}

impl<'a> std::ops::Deref for SpinSocketsView<'a> {
    type Target = WasiSocketsCtxView<'a>;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl std::ops::DerefMut for SpinSocketsView<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

/// [`HasData`] accessor for [`SpinSocketsView`], used in place of [`WasiSockets`]
/// when registering TCP socket bindings so that `start_connect` and `drop` can
/// participate in socket quota tracking.
pub struct SpinSockets;

impl HasData for SpinSockets {
    type Data<'a> = SpinSocketsView<'a>;
}

impl SpinSocketsView<'_> {
    /// Attempts to acquire a connection permit from the semaphore.
    ///
    /// Returns `Ok(None)` when no quota is configured, `Ok(Some(permit))` on
    /// success, or `Err(())` when the quota is exhausted.
    ///
    /// The returned permit is unregistered — call [`Self::register_permit`] once
    /// the socket resource rep is known to tie its lifetime to the socket.
    pub(crate) fn try_acquire(&self) -> Result<Option<ConnectionPermit>, ()> {
        let Some(state) = &self.permit_state else {
            return Ok(None);
        };
        state.semaphore.try_acquire().map(Some).ok_or(())
    }

    /// Registers `permit` under `socket_rep` so it is held until the socket is
    /// dropped. No-op when `permit` is `None` (no quota configured).
    pub(crate) fn register_permit(&self, socket_rep: u32, permit: Option<ConnectionPermit>) {
        let (Some(state), Some(permit)) = (&self.permit_state, permit) else {
            return;
        };
        state
            .active
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(socket_rep, permit);
    }

    /// Releases the connection permit for `socket_rep`, if any.
    pub(crate) fn release_permit(&self, socket_rep: u32) {
        if let Some(state) = &self.permit_state {
            state
                .active
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&socket_rep);
        }
    }
}

impl p2_tcp::Host for SpinSocketsView<'_> {}

impl p2_tcp::HostTcpSocket for SpinSocketsView<'_> {
    async fn start_bind(
        &mut self,
        this: Resource<TcpSocket>,
        network: Resource<Network>,
        local_address: IpSocketAddress,
    ) -> wasmtime_wasi::p2::SocketResult<()> {
        p2_tcp::HostTcpSocket::start_bind(&mut self.inner, this, network, local_address).await
    }

    fn finish_bind(&mut self, this: Resource<TcpSocket>) -> wasmtime_wasi::p2::SocketResult<()> {
        p2_tcp::HostTcpSocket::finish_bind(&mut self.inner, this)
    }

    async fn start_connect(
        &mut self,
        this: Resource<TcpSocket>,
        network: Resource<Network>,
        remote_address: IpSocketAddress,
    ) -> wasmtime_wasi::p2::SocketResult<()> {
        let socket_rep = this.rep();
        // Unlike outbound HTTP (which queues when its permit pool is exhausted),
        // sockets fail immediately. Waiting would risk deadlock if a component
        // holds sockets open across async yield points, and raw-socket callers
        // are better positioned to implement their own retry logic.
        let Ok(permit) = self.try_acquire() else {
            tracing::warn!("TCP socket connection refused: connection quota exhausted");
            return Err(SocketErrorCode::NewSocketLimit.into());
        };
        let result =
            p2_tcp::HostTcpSocket::start_connect(&mut self.inner, this, network, remote_address)
                .await;
        if result.is_ok() {
            self.register_permit(socket_rep, permit);
        }
        // On error, `permit` is dropped here, automatically releasing the semaphore slot.
        result
    }

    fn finish_connect(
        &mut self,
        this: Resource<TcpSocket>,
    ) -> wasmtime_wasi::p2::SocketResult<(Resource<DynInputStream>, Resource<DynOutputStream>)>
    {
        p2_tcp::HostTcpSocket::finish_connect(&mut self.inner, this)
    }

    fn start_listen(&mut self, this: Resource<TcpSocket>) -> wasmtime_wasi::p2::SocketResult<()> {
        p2_tcp::HostTcpSocket::start_listen(&mut self.inner, this)
    }

    fn finish_listen(&mut self, this: Resource<TcpSocket>) -> wasmtime_wasi::p2::SocketResult<()> {
        p2_tcp::HostTcpSocket::finish_listen(&mut self.inner, this)
    }

    fn accept(
        &mut self,
        this: Resource<TcpSocket>,
    ) -> wasmtime_wasi::p2::SocketResult<(
        Resource<TcpSocket>,
        Resource<DynInputStream>,
        Resource<DynOutputStream>,
    )> {
        p2_tcp::HostTcpSocket::accept(&mut self.inner, this)
    }

    fn local_address(
        &mut self,
        this: Resource<TcpSocket>,
    ) -> wasmtime_wasi::p2::SocketResult<IpSocketAddress> {
        p2_tcp::HostTcpSocket::local_address(&mut self.inner, this)
    }

    fn remote_address(
        &mut self,
        this: Resource<TcpSocket>,
    ) -> wasmtime_wasi::p2::SocketResult<IpSocketAddress> {
        p2_tcp::HostTcpSocket::remote_address(&mut self.inner, this)
    }

    fn is_listening(&mut self, this: Resource<TcpSocket>) -> wasmtime::Result<bool> {
        p2_tcp::HostTcpSocket::is_listening(&mut self.inner, this)
    }

    fn address_family(
        &mut self,
        this: Resource<TcpSocket>,
    ) -> wasmtime::Result<wasmtime_wasi::p2::bindings::sockets::network::IpAddressFamily> {
        p2_tcp::HostTcpSocket::address_family(&mut self.inner, this)
    }

    fn set_listen_backlog_size(
        &mut self,
        this: Resource<TcpSocket>,
        value: u64,
    ) -> wasmtime_wasi::p2::SocketResult<()> {
        p2_tcp::HostTcpSocket::set_listen_backlog_size(&mut self.inner, this, value)
    }

    fn keep_alive_enabled(
        &mut self,
        this: Resource<TcpSocket>,
    ) -> wasmtime_wasi::p2::SocketResult<bool> {
        p2_tcp::HostTcpSocket::keep_alive_enabled(&mut self.inner, this)
    }

    fn set_keep_alive_enabled(
        &mut self,
        this: Resource<TcpSocket>,
        value: bool,
    ) -> wasmtime_wasi::p2::SocketResult<()> {
        p2_tcp::HostTcpSocket::set_keep_alive_enabled(&mut self.inner, this, value)
    }

    fn keep_alive_idle_time(
        &mut self,
        this: Resource<TcpSocket>,
    ) -> wasmtime_wasi::p2::SocketResult<u64> {
        p2_tcp::HostTcpSocket::keep_alive_idle_time(&mut self.inner, this)
    }

    fn set_keep_alive_idle_time(
        &mut self,
        this: Resource<TcpSocket>,
        value: u64,
    ) -> wasmtime_wasi::p2::SocketResult<()> {
        p2_tcp::HostTcpSocket::set_keep_alive_idle_time(&mut self.inner, this, value)
    }

    fn keep_alive_interval(
        &mut self,
        this: Resource<TcpSocket>,
    ) -> wasmtime_wasi::p2::SocketResult<u64> {
        p2_tcp::HostTcpSocket::keep_alive_interval(&mut self.inner, this)
    }

    fn set_keep_alive_interval(
        &mut self,
        this: Resource<TcpSocket>,
        value: u64,
    ) -> wasmtime_wasi::p2::SocketResult<()> {
        p2_tcp::HostTcpSocket::set_keep_alive_interval(&mut self.inner, this, value)
    }

    fn keep_alive_count(
        &mut self,
        this: Resource<TcpSocket>,
    ) -> wasmtime_wasi::p2::SocketResult<u32> {
        p2_tcp::HostTcpSocket::keep_alive_count(&mut self.inner, this)
    }

    fn set_keep_alive_count(
        &mut self,
        this: Resource<TcpSocket>,
        value: u32,
    ) -> wasmtime_wasi::p2::SocketResult<()> {
        p2_tcp::HostTcpSocket::set_keep_alive_count(&mut self.inner, this, value)
    }

    fn hop_limit(&mut self, this: Resource<TcpSocket>) -> wasmtime_wasi::p2::SocketResult<u8> {
        p2_tcp::HostTcpSocket::hop_limit(&mut self.inner, this)
    }

    fn set_hop_limit(
        &mut self,
        this: Resource<TcpSocket>,
        value: u8,
    ) -> wasmtime_wasi::p2::SocketResult<()> {
        p2_tcp::HostTcpSocket::set_hop_limit(&mut self.inner, this, value)
    }

    fn receive_buffer_size(
        &mut self,
        this: Resource<TcpSocket>,
    ) -> wasmtime_wasi::p2::SocketResult<u64> {
        p2_tcp::HostTcpSocket::receive_buffer_size(&mut self.inner, this)
    }

    fn set_receive_buffer_size(
        &mut self,
        this: Resource<TcpSocket>,
        value: u64,
    ) -> wasmtime_wasi::p2::SocketResult<()> {
        p2_tcp::HostTcpSocket::set_receive_buffer_size(&mut self.inner, this, value)
    }

    fn send_buffer_size(
        &mut self,
        this: Resource<TcpSocket>,
    ) -> wasmtime_wasi::p2::SocketResult<u64> {
        p2_tcp::HostTcpSocket::send_buffer_size(&mut self.inner, this)
    }

    fn set_send_buffer_size(
        &mut self,
        this: Resource<TcpSocket>,
        value: u64,
    ) -> wasmtime_wasi::p2::SocketResult<()> {
        p2_tcp::HostTcpSocket::set_send_buffer_size(&mut self.inner, this, value)
    }

    fn subscribe(&mut self, this: Resource<TcpSocket>) -> wasmtime::Result<Resource<DynPollable>> {
        p2_tcp::HostTcpSocket::subscribe(&mut self.inner, this)
    }

    fn shutdown(
        &mut self,
        this: Resource<TcpSocket>,
        shutdown_type: ShutdownType,
    ) -> wasmtime_wasi::p2::SocketResult<()> {
        p2_tcp::HostTcpSocket::shutdown(&mut self.inner, this, shutdown_type)
    }

    fn drop(&mut self, this: Resource<TcpSocket>) -> wasmtime::Result<()> {
        self.release_permit(this.rep());
        p2_tcp::HostTcpSocket::drop(&mut self.inner, this)
    }
}

impl NetworkHost for SpinSocketsView<'_> {
    fn convert_error_code(
        &mut self,
        error: wasmtime_wasi::p2::SocketError,
    ) -> wasmtime::Result<wasmtime_wasi::p2::bindings::sockets::network::ErrorCode> {
        NetworkHost::convert_error_code(&mut self.inner, error)
    }

    fn network_error_code(
        &mut self,
        err: Resource<wasmtime::Error>,
    ) -> wasmtime::Result<Option<wasmtime_wasi::p2::bindings::sockets::network::ErrorCode>> {
        NetworkHost::network_error_code(&mut self.inner, err)
    }
}

impl wasmtime_wasi::p2::bindings::sockets::network::HostNetwork for SpinSocketsView<'_> {
    fn drop(&mut self, this: Resource<Network>) -> wasmtime::Result<()> {
        wasmtime_wasi::p2::bindings::sockets::network::HostNetwork::drop(&mut self.inner, this)
    }
}

impl p2_tcp_create::Host for SpinSocketsView<'_> {
    fn create_tcp_socket(
        &mut self,
        address_family: wasmtime_wasi::p2::bindings::sockets::network::IpAddressFamily,
    ) -> wasmtime_wasi::p2::SocketResult<Resource<TcpSocket>> {
        p2_tcp_create::Host::create_tcp_socket(&mut self.inner, address_family)
    }
}

impl p2_udp::Host for SpinSocketsView<'_> {}

impl p2_udp::HostUdpSocket for SpinSocketsView<'_> {
    async fn start_bind(
        &mut self,
        this: Resource<p2_udp::UdpSocket>,
        network: Resource<p2_udp::Network>,
        local_address: p2_udp::IpSocketAddress,
    ) -> wasmtime_wasi::p2::SocketResult<()> {
        p2_udp::HostUdpSocket::start_bind(&mut self.inner, this, network, local_address).await
    }

    fn finish_bind(
        &mut self,
        this: Resource<p2_udp::UdpSocket>,
    ) -> wasmtime_wasi::p2::SocketResult<()> {
        p2_udp::HostUdpSocket::finish_bind(&mut self.inner, this)
    }

    async fn stream(
        &mut self,
        this: Resource<p2_udp::UdpSocket>,
        remote_address: Option<p2_udp::IpSocketAddress>,
    ) -> wasmtime_wasi::p2::SocketResult<(
        Resource<p2_udp::IncomingDatagramStream>,
        Resource<p2_udp::OutgoingDatagramStream>,
    )> {
        p2_udp::HostUdpSocket::stream(&mut self.inner, this, remote_address).await
    }

    fn local_address(
        &mut self,
        this: Resource<p2_udp::UdpSocket>,
    ) -> wasmtime_wasi::p2::SocketResult<p2_udp::IpSocketAddress> {
        p2_udp::HostUdpSocket::local_address(&mut self.inner, this)
    }

    fn remote_address(
        &mut self,
        this: Resource<p2_udp::UdpSocket>,
    ) -> wasmtime_wasi::p2::SocketResult<p2_udp::IpSocketAddress> {
        p2_udp::HostUdpSocket::remote_address(&mut self.inner, this)
    }

    fn address_family(
        &mut self,
        this: Resource<p2_udp::UdpSocket>,
    ) -> wasmtime::Result<p2_udp::IpAddressFamily> {
        p2_udp::HostUdpSocket::address_family(&mut self.inner, this)
    }

    fn unicast_hop_limit(
        &mut self,
        this: Resource<p2_udp::UdpSocket>,
    ) -> wasmtime_wasi::p2::SocketResult<u8> {
        p2_udp::HostUdpSocket::unicast_hop_limit(&mut self.inner, this)
    }

    fn set_unicast_hop_limit(
        &mut self,
        this: Resource<p2_udp::UdpSocket>,
        value: u8,
    ) -> wasmtime_wasi::p2::SocketResult<()> {
        p2_udp::HostUdpSocket::set_unicast_hop_limit(&mut self.inner, this, value)
    }

    fn receive_buffer_size(
        &mut self,
        this: Resource<p2_udp::UdpSocket>,
    ) -> wasmtime_wasi::p2::SocketResult<u64> {
        p2_udp::HostUdpSocket::receive_buffer_size(&mut self.inner, this)
    }

    fn set_receive_buffer_size(
        &mut self,
        this: Resource<p2_udp::UdpSocket>,
        value: u64,
    ) -> wasmtime_wasi::p2::SocketResult<()> {
        p2_udp::HostUdpSocket::set_receive_buffer_size(&mut self.inner, this, value)
    }

    fn send_buffer_size(
        &mut self,
        this: Resource<p2_udp::UdpSocket>,
    ) -> wasmtime_wasi::p2::SocketResult<u64> {
        p2_udp::HostUdpSocket::send_buffer_size(&mut self.inner, this)
    }

    fn set_send_buffer_size(
        &mut self,
        this: Resource<p2_udp::UdpSocket>,
        value: u64,
    ) -> wasmtime_wasi::p2::SocketResult<()> {
        p2_udp::HostUdpSocket::set_send_buffer_size(&mut self.inner, this, value)
    }

    fn subscribe(
        &mut self,
        this: Resource<p2_udp::UdpSocket>,
    ) -> wasmtime::Result<Resource<DynPollable>> {
        p2_udp::HostUdpSocket::subscribe(&mut self.inner, this)
    }

    fn drop(&mut self, this: Resource<p2_udp::UdpSocket>) -> wasmtime::Result<()> {
        self.release_permit(this.rep());
        p2_udp::HostUdpSocket::drop(&mut self.inner, this)
    }
}

impl p2_udp::HostIncomingDatagramStream for SpinSocketsView<'_> {
    fn receive(
        &mut self,
        this: Resource<p2_udp::IncomingDatagramStream>,
        max_results: u64,
    ) -> wasmtime_wasi::p2::SocketResult<Vec<p2_udp::IncomingDatagram>> {
        p2_udp::HostIncomingDatagramStream::receive(&mut self.inner, this, max_results)
    }

    fn subscribe(
        &mut self,
        this: Resource<p2_udp::IncomingDatagramStream>,
    ) -> wasmtime::Result<Resource<DynPollable>> {
        p2_udp::HostIncomingDatagramStream::subscribe(&mut self.inner, this)
    }

    fn drop(&mut self, this: Resource<p2_udp::IncomingDatagramStream>) -> wasmtime::Result<()> {
        p2_udp::HostIncomingDatagramStream::drop(&mut self.inner, this)
    }
}

impl p2_udp::HostOutgoingDatagramStream for SpinSocketsView<'_> {
    fn check_send(
        &mut self,
        this: Resource<p2_udp::OutgoingDatagramStream>,
    ) -> wasmtime_wasi::p2::SocketResult<u64> {
        p2_udp::HostOutgoingDatagramStream::check_send(&mut self.inner, this)
    }

    async fn send(
        &mut self,
        this: Resource<p2_udp::OutgoingDatagramStream>,
        datagrams: Vec<p2_udp::OutgoingDatagram>,
    ) -> wasmtime_wasi::p2::SocketResult<u64> {
        p2_udp::HostOutgoingDatagramStream::send(&mut self.inner, this, datagrams).await
    }

    fn subscribe(
        &mut self,
        this: Resource<p2_udp::OutgoingDatagramStream>,
    ) -> wasmtime::Result<Resource<DynPollable>> {
        p2_udp::HostOutgoingDatagramStream::subscribe(&mut self.inner, this)
    }

    fn drop(&mut self, this: Resource<p2_udp::OutgoingDatagramStream>) -> wasmtime::Result<()> {
        p2_udp::HostOutgoingDatagramStream::drop(&mut self.inner, this)
    }
}

impl p2_udp_create::Host for SpinSocketsView<'_> {
    fn create_udp_socket(
        &mut self,
        address_family: wasmtime_wasi::p2::bindings::sockets::network::IpAddressFamily,
    ) -> wasmtime_wasi::p2::SocketResult<Resource<UdpSocket>> {
        // Check quota before allocating the socket resource.
        // See the analogous comment in `start_connect` for why we fail
        // immediately rather than waiting (as outbound HTTP does).
        let Ok(permit) = self.try_acquire() else {
            tracing::warn!("UDP socket creation refused: connection quota exhausted");
            return Err(SocketErrorCode::NewSocketLimit.into());
        };
        let sock = p2_udp_create::Host::create_udp_socket(&mut self.inner, address_family)?;
        self.register_permit(sock.rep(), permit);
        Ok(sock)
    }
}

// ── p3 socket bindings ────────────────────────────────────────────────────────

use wasmtime_wasi::p3::bindings::sockets::types as p3_types;
use wasmtime_wasi::p3::sockets::{SocketError as P3SocketError, SocketResult as P3SocketResult};

impl p3_types::Host for SpinSocketsView<'_> {
    fn convert_error_code(
        &mut self,
        error: P3SocketError,
    ) -> wasmtime::Result<p3_types::ErrorCode> {
        error.downcast()
    }
}

impl p3_types::HostTcpSocket for SpinSocketsView<'_> {
    async fn bind(
        &mut self,
        socket: Resource<TcpSocket>,
        local_address: p3_types::IpSocketAddress,
    ) -> P3SocketResult<()> {
        p3_types::HostTcpSocket::bind(&mut self.inner, socket, local_address).await
    }

    fn create(
        &mut self,
        address_family: p3_types::IpAddressFamily,
    ) -> P3SocketResult<Resource<TcpSocket>> {
        p3_types::HostTcpSocket::create(&mut self.inner, address_family)
    }

    fn get_local_address(
        &mut self,
        socket: Resource<TcpSocket>,
    ) -> P3SocketResult<p3_types::IpSocketAddress> {
        p3_types::HostTcpSocket::get_local_address(&mut self.inner, socket)
    }

    fn get_remote_address(
        &mut self,
        socket: Resource<TcpSocket>,
    ) -> P3SocketResult<p3_types::IpSocketAddress> {
        p3_types::HostTcpSocket::get_remote_address(&mut self.inner, socket)
    }

    fn get_is_listening(&mut self, socket: Resource<TcpSocket>) -> wasmtime::Result<bool> {
        p3_types::HostTcpSocket::get_is_listening(&mut self.inner, socket)
    }

    fn get_address_family(
        &mut self,
        socket: Resource<TcpSocket>,
    ) -> wasmtime::Result<p3_types::IpAddressFamily> {
        p3_types::HostTcpSocket::get_address_family(&mut self.inner, socket)
    }

    fn set_listen_backlog_size(
        &mut self,
        socket: Resource<TcpSocket>,
        value: u64,
    ) -> P3SocketResult<()> {
        p3_types::HostTcpSocket::set_listen_backlog_size(&mut self.inner, socket, value)
    }

    fn get_keep_alive_enabled(
        &mut self,
        socket: Resource<TcpSocket>,
    ) -> P3SocketResult<bool> {
        p3_types::HostTcpSocket::get_keep_alive_enabled(&mut self.inner, socket)
    }

    fn set_keep_alive_enabled(
        &mut self,
        socket: Resource<TcpSocket>,
        value: bool,
    ) -> P3SocketResult<()> {
        p3_types::HostTcpSocket::set_keep_alive_enabled(&mut self.inner, socket, value)
    }

    fn get_keep_alive_idle_time(
        &mut self,
        socket: Resource<TcpSocket>,
    ) -> P3SocketResult<p3_types::Duration> {
        p3_types::HostTcpSocket::get_keep_alive_idle_time(&mut self.inner, socket)
    }

    fn set_keep_alive_idle_time(
        &mut self,
        socket: Resource<TcpSocket>,
        value: p3_types::Duration,
    ) -> P3SocketResult<()> {
        p3_types::HostTcpSocket::set_keep_alive_idle_time(&mut self.inner, socket, value)
    }

    fn get_keep_alive_interval(
        &mut self,
        socket: Resource<TcpSocket>,
    ) -> P3SocketResult<p3_types::Duration> {
        p3_types::HostTcpSocket::get_keep_alive_interval(&mut self.inner, socket)
    }

    fn set_keep_alive_interval(
        &mut self,
        socket: Resource<TcpSocket>,
        value: p3_types::Duration,
    ) -> P3SocketResult<()> {
        p3_types::HostTcpSocket::set_keep_alive_interval(&mut self.inner, socket, value)
    }

    fn get_keep_alive_count(
        &mut self,
        socket: Resource<TcpSocket>,
    ) -> P3SocketResult<u32> {
        p3_types::HostTcpSocket::get_keep_alive_count(&mut self.inner, socket)
    }

    fn set_keep_alive_count(
        &mut self,
        socket: Resource<TcpSocket>,
        value: u32,
    ) -> P3SocketResult<()> {
        p3_types::HostTcpSocket::set_keep_alive_count(&mut self.inner, socket, value)
    }

    fn get_hop_limit(&mut self, socket: Resource<TcpSocket>) -> P3SocketResult<u8> {
        p3_types::HostTcpSocket::get_hop_limit(&mut self.inner, socket)
    }

    fn set_hop_limit(
        &mut self,
        socket: Resource<TcpSocket>,
        value: u8,
    ) -> P3SocketResult<()> {
        p3_types::HostTcpSocket::set_hop_limit(&mut self.inner, socket, value)
    }

    fn get_receive_buffer_size(
        &mut self,
        socket: Resource<TcpSocket>,
    ) -> P3SocketResult<u64> {
        p3_types::HostTcpSocket::get_receive_buffer_size(&mut self.inner, socket)
    }

    fn set_receive_buffer_size(
        &mut self,
        socket: Resource<TcpSocket>,
        value: u64,
    ) -> P3SocketResult<()> {
        p3_types::HostTcpSocket::set_receive_buffer_size(&mut self.inner, socket, value)
    }

    fn get_send_buffer_size(
        &mut self,
        socket: Resource<TcpSocket>,
    ) -> P3SocketResult<u64> {
        p3_types::HostTcpSocket::get_send_buffer_size(&mut self.inner, socket)
    }

    fn set_send_buffer_size(
        &mut self,
        socket: Resource<TcpSocket>,
        value: u64,
    ) -> P3SocketResult<()> {
        p3_types::HostTcpSocket::set_send_buffer_size(&mut self.inner, socket, value)
    }

    fn drop(&mut self, sock: Resource<TcpSocket>) -> wasmtime::Result<()> {
        self.release_permit(sock.rep());
        p3_types::HostTcpSocket::drop(&mut self.inner, sock)
    }
}

impl p3_types::HostUdpSocket for SpinSocketsView<'_> {
    async fn bind(
        &mut self,
        socket: Resource<UdpSocket>,
        local_address: p3_types::IpSocketAddress,
    ) -> P3SocketResult<()> {
        p3_types::HostUdpSocket::bind(&mut self.inner, socket, local_address).await
    }

    async fn connect(
        &mut self,
        socket: Resource<UdpSocket>,
        remote_address: p3_types::IpSocketAddress,
    ) -> P3SocketResult<()> {
        p3_types::HostUdpSocket::connect(&mut self.inner, socket, remote_address).await
    }

    fn create(
        &mut self,
        address_family: p3_types::IpAddressFamily,
    ) -> P3SocketResult<Resource<UdpSocket>> {
        let Ok(permit) = self.try_acquire() else {
            tracing::warn!("UDP socket creation refused: connection quota exhausted");
            return Err(p3_types::ErrorCode::AccessDenied.into());
        };
        let sock = p3_types::HostUdpSocket::create(&mut self.inner, address_family)?;
        self.register_permit(sock.rep(), permit);
        Ok(sock)
    }

    fn disconnect(&mut self, socket: Resource<UdpSocket>) -> P3SocketResult<()> {
        p3_types::HostUdpSocket::disconnect(&mut self.inner, socket)
    }

    fn get_local_address(
        &mut self,
        socket: Resource<UdpSocket>,
    ) -> P3SocketResult<p3_types::IpSocketAddress> {
        p3_types::HostUdpSocket::get_local_address(&mut self.inner, socket)
    }

    fn get_remote_address(
        &mut self,
        socket: Resource<UdpSocket>,
    ) -> P3SocketResult<p3_types::IpSocketAddress> {
        p3_types::HostUdpSocket::get_remote_address(&mut self.inner, socket)
    }

    fn get_address_family(
        &mut self,
        socket: Resource<UdpSocket>,
    ) -> wasmtime::Result<p3_types::IpAddressFamily> {
        p3_types::HostUdpSocket::get_address_family(&mut self.inner, socket)
    }

    fn get_unicast_hop_limit(
        &mut self,
        socket: Resource<UdpSocket>,
    ) -> P3SocketResult<u8> {
        p3_types::HostUdpSocket::get_unicast_hop_limit(&mut self.inner, socket)
    }

    fn set_unicast_hop_limit(
        &mut self,
        socket: Resource<UdpSocket>,
        value: u8,
    ) -> P3SocketResult<()> {
        p3_types::HostUdpSocket::set_unicast_hop_limit(&mut self.inner, socket, value)
    }

    fn get_receive_buffer_size(
        &mut self,
        socket: Resource<UdpSocket>,
    ) -> P3SocketResult<u64> {
        p3_types::HostUdpSocket::get_receive_buffer_size(&mut self.inner, socket)
    }

    fn set_receive_buffer_size(
        &mut self,
        socket: Resource<UdpSocket>,
        value: u64,
    ) -> P3SocketResult<()> {
        p3_types::HostUdpSocket::set_receive_buffer_size(&mut self.inner, socket, value)
    }

    fn get_send_buffer_size(
        &mut self,
        socket: Resource<UdpSocket>,
    ) -> P3SocketResult<u64> {
        p3_types::HostUdpSocket::get_send_buffer_size(&mut self.inner, socket)
    }

    fn set_send_buffer_size(
        &mut self,
        socket: Resource<UdpSocket>,
        value: u64,
    ) -> P3SocketResult<()> {
        p3_types::HostUdpSocket::set_send_buffer_size(&mut self.inner, socket, value)
    }

    fn drop(&mut self, sock: Resource<UdpSocket>) -> wasmtime::Result<()> {
        self.release_permit(sock.rep());
        p3_types::HostUdpSocket::drop(&mut self.inner, sock)
    }
}

// ── p3 WithStore impls ────────────────────────────────────────────────────────

use wasmtime::component::{Access, Accessor, FutureReader, StreamReader};
use wasmtime_wasi::sockets::WasiSockets;

impl p3_types::HostTcpSocketWithStore for SpinSockets {
    async fn connect<T: Send>(
        store: &Accessor<T, Self>,
        socket: Resource<TcpSocket>,
        remote_address: p3_types::IpSocketAddress,
    ) -> P3SocketResult<()> {
        let socket_rep = socket.rep();
        let Ok(permit) = store.with(|mut view| Ok(view.get().try_acquire()))? else {
            tracing::warn!("TCP socket connection refused: connection quota exhausted");
            return Err(p3_types::ErrorCode::AccessDenied.into());
        };
        let wasi_accessor: Accessor<T, WasiSockets> = store.with_getter(|t|{ todo!()});
        let result = WasiSockets::connect(&wasi_accessor, socket, remote_address).await;
        if result.is_ok() {
            store.with(|mut view| Ok(view.get().register_permit(socket_rep, permit)))?;
        }
        result
    }

    fn listen<T: 'static>(
        mut store: Access<'_, T, Self>,
        socket: Resource<TcpSocket>,
    ) -> P3SocketResult<StreamReader<Resource<TcpSocket>>> {
        // BLOCKED: same getter problem — we have fn(&mut T) -> SpinSocketsView<'_>
        // but WasiSockets::listen needs fn(&mut T) -> WasiSocketsCtxView<'_>.
        // Also needs listen_p3/tcp_listener_arc/non_inherited_options, all pub(crate).
        use wasmtime::AsContextMut;
        let wasi_store: Access<'_, T, WasiSockets> =
            Access::new(store.as_context_mut(), todo!());
        WasiSockets::listen(wasi_store, socket)
    }

    fn send<T: 'static>(
        mut store: Access<'_, T, Self>,
        socket: Resource<TcpSocket>,
        data: StreamReader<u8>,
    ) -> wasmtime::Result<FutureReader<Result<(), p3_types::ErrorCode>>> {
        // BLOCKED: same getter problem; also needs TcpSocket::take_send_stream (pub(crate)).
        use wasmtime::AsContextMut;
        let wasi_store: Access<'_, T, WasiSockets> =
            Access::new(store.as_context_mut(), todo!());
        WasiSockets::send(wasi_store, socket, data)
    }

    fn receive<T: 'static>(
        mut store: Access<T, Self>,
        socket: Resource<TcpSocket>,
    ) -> wasmtime::Result<(
        StreamReader<u8>,
        FutureReader<Result<(), p3_types::ErrorCode>>,
    )> {
        // BLOCKED: same getter problem; also needs TcpSocket::take_receive_stream (pub(crate)).
        use wasmtime::AsContextMut;
        let wasi_store: Access<'_, T, WasiSockets> =
            Access::new(store.as_context_mut(), todo!());
        WasiSockets::receive(wasi_store, socket)
    }
}

impl p3_types::HostUdpSocketWithStore for SpinSockets {
    async fn send<T>(
        store: &Accessor<T, Self>,
        socket: Resource<UdpSocket>,
        data: Vec<u8>,
        remote_address: Option<p3_types::IpSocketAddress>,
    ) -> P3SocketResult<()> {
        // BLOCKED: same getter problem; also needs UdpSocket::send_p3 (pub(crate)).
        let wasi_accessor: Accessor<T, WasiSockets> = store.with_getter(todo!());
        WasiSockets::send(&wasi_accessor, socket, data, remote_address).await
    }

    async fn receive<T>(
        store: &Accessor<T, Self>,
        socket: Resource<UdpSocket>,
    ) -> P3SocketResult<(Vec<u8>, p3_types::IpSocketAddress)> {
        // BLOCKED: same getter problem; also needs UdpSocket::receive_p3 (pub(crate)).
        let wasi_accessor: Accessor<T, WasiSockets> = store.with_getter(todo!());
        WasiSockets::receive(&wasi_accessor, socket).await
    }
}
