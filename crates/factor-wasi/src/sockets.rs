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
        if let Some(state) = &self.permit_state {
            let socket_rep = this.rep();
            // Unlike outbound HTTP (which queues when its permit pool is exhausted),
            // sockets fail immediately. Waiting would risk deadlock if a component
            // holds sockets open across async yield points, and raw-socket callers
            // are better positioned to implement their own retry logic.
            let Some(permit) = state.semaphore.try_acquire() else {
                tracing::warn!("TCP socket connection refused: connection quota exhausted");
                return Err(SocketErrorCode::NewSocketLimit.into());
            };
            p2_tcp::HostTcpSocket::start_connect(&mut self.inner, this, network, remote_address)
                .await?;
            state
                .active
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(socket_rep, permit);
            Ok(())
        } else {
            p2_tcp::HostTcpSocket::start_connect(&mut self.inner, this, network, remote_address)
                .await
        }
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
        // Release both permits before dropping the socket resource.
        if let Some(state) = &self.permit_state {
            state
                .active
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&this.rep());
        }
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
        // Release both permits before dropping the socket resource.
        if let Some(state) = &self.permit_state {
            state
                .active
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&this.rep());
        }
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
        if let Some(state) = &self.permit_state {
            let state = Arc::clone(state);
            // See the analogous comment in `start_connect` for why we fail
            // immediately rather than waiting (as outbound HTTP does).
            let Some(permit) = state.semaphore.try_acquire() else {
                tracing::warn!("UDP socket creation refused: connection quota exhausted");
                return Err(SocketErrorCode::NewSocketLimit.into());
            };
            let sock = p2_udp_create::Host::create_udp_socket(&mut self.inner, address_family)?;
            state
                .active
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(sock.rep(), permit);
            Ok(sock)
        } else {
            p2_udp_create::Host::create_udp_socket(&mut self.inner, address_family)
        }
    }
}
