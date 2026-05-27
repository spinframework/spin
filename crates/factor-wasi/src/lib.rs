mod io;
pub mod spin;
mod wasi_2023_10_18;
mod wasi_2023_11_10;

use std::{
    collections::HashMap,
    future::Future,
    io::{Read, Write},
    net::SocketAddr,
    path::Path,
    sync::{Arc, Mutex},
};

use io::{PipeReadStream, PipedWriteStream};
use spin_factors::{
    AppComponent, Factor, FactorInstanceBuilder, InitContext, PrepareContext, RuntimeFactors,
    RuntimeFactorsInstanceState, anyhow,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use wasmtime::component::{HasData, Resource};
use wasmtime_wasi::cli::{StdinStream, StdoutStream, WasiCli, WasiCliCtxView};
use wasmtime_wasi::clocks::{WasiClocks, WasiClocksCtxView};
use wasmtime_wasi::filesystem::{WasiFilesystem, WasiFilesystemCtxView};
use wasmtime_wasi::p2::bindings::sockets::network::{
    ErrorCode as SocketErrorCode, Host as NetworkHost, Network,
};
use wasmtime_wasi::p2::bindings::sockets::tcp::{self as p2_tcp, IpSocketAddress, ShutdownType};
use wasmtime_wasi::p2::bindings::sockets::tcp_create_socket as p2_tcp_create;
use wasmtime_wasi::p2::bindings::sockets::udp as p2_udp;
use wasmtime_wasi::p2::bindings::sockets::udp_create_socket as p2_udp_create;
use wasmtime_wasi::p2::{DynInputStream, DynOutputStream, DynPollable};
use wasmtime_wasi::random::{WasiRandom, WasiRandomCtx};
use wasmtime_wasi::sockets::{TcpSocket, UdpSocket, WasiSockets, WasiSocketsCtxView};
use wasmtime_wasi::{DirPerms, FilePerms, ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView};

pub use wasmtime_wasi::sockets::SocketAddrUse;

/// Shared state for tracking per-socket semaphore permits. Permits are
/// acquired when a socket is allocated (at `start_connect` for TCP, at
/// `create_udp_socket` for UDP) and released when the socket resource is dropped.
pub struct SocketPermitState {
    semaphore: Arc<Semaphore>,
    /// Active permits keyed by socket resource rep.
    ///
    /// Permits are removed (and the permit released) when the WASI socket resource is dropped.
    active: Mutex<HashMap<u32, OwnedSemaphorePermit>>,
}

impl SocketPermitState {
    pub fn new(semaphore: Arc<Semaphore>) -> Arc<Self> {
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
            // If we have a permit state, we need to acquire a permit before allowing the connection to proceed.
            let socket_rep = this.rep();
            let Ok(permit) = Arc::clone(&state.semaphore).try_acquire_owned() else {
                // wasi has no "quota exceeded" error code. ConnectionRefused is the closest available.
                return Err(SocketErrorCode::ConnectionRefused.into());
            };
            p2_tcp::HostTcpSocket::start_connect(&mut self.inner, this, network, remote_address)
                .await?;
            // If the connection was successfully initiated, store the permit so it can be released when the socket is dropped.
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
        // Release the permit before dropping the socket resource.
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
        // Release the permit before dropping the socket resource.
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
            // If we have a permit state, we need to acquire a permit before allowing the socket creation to proceed.
            let state = Arc::clone(state);
            let permit = Arc::clone(&state.semaphore)
                .try_acquire_owned()
                .map_err(|_| SocketErrorCode::ConnectionRefused)?;
            let sock = p2_udp_create::Host::create_udp_socket(&mut self.inner, address_family)?;
            // If the socket was successfully created, store the permit so it can be released when the socket is dropped.
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

pub struct WasiFactor {
    files_mounter: Box<dyn FilesMounter>,
}

impl WasiFactor {
    pub fn new(files_mounter: impl FilesMounter + 'static) -> Self {
        Self {
            files_mounter: Box::new(files_mounter),
        }
    }

    pub fn get_wasi_impl(
        runtime_instance_state: &mut impl RuntimeFactorsInstanceState,
    ) -> Option<WasiCtxView<'_>> {
        let (state, table) = runtime_instance_state.get_with_table::<WasiFactor>()?;
        Some(WasiCtxView {
            ctx: &mut state.ctx,
            table,
        })
    }

    pub fn get_cli_impl(
        runtime_instance_state: &mut impl RuntimeFactorsInstanceState,
    ) -> Option<WasiCliCtxView<'_>> {
        let (state, table) = runtime_instance_state.get_with_table::<WasiFactor>()?;
        Some(WasiCliCtxView {
            ctx: state.ctx.cli(),
            table,
        })
    }

    pub fn get_sockets_impl(
        runtime_instance_state: &mut impl RuntimeFactorsInstanceState,
    ) -> Option<SpinSocketsView<'_>> {
        let (state, table) = runtime_instance_state.get_with_table::<WasiFactor>()?;
        Some(SpinSocketsView {
            inner: WasiSocketsCtxView {
                ctx: state.ctx.sockets(),
                table,
            },
            permit_state: state.socket_permit_state.clone(),
        })
    }
}

/// Helper trait to extend `InitContext` with some more `link_*_bindings`
/// methods related to `wasmtime-wasi` and `wasmtime-wasi-io`-specific
/// signatures.
#[allow(clippy::type_complexity, reason = "sorry, blame alex")]
trait InitContextExt: InitContext<WasiFactor> {
    fn get_table(data: &mut Self::StoreData) -> &mut ResourceTable {
        let (_state, table) = Self::get_data_with_table(data);
        table
    }

    fn get_clocks(data: &mut Self::StoreData) -> WasiClocksCtxView<'_> {
        let (state, table) = Self::get_data_with_table(data);
        WasiClocksCtxView {
            ctx: state.ctx.clocks(),
            table,
        }
    }

    fn get_random(data: &mut Self::StoreData) -> &mut WasiRandomCtx {
        let (state, _) = Self::get_data_with_table(data);
        state.ctx.random()
    }

    fn link_clocks_bindings(
        &mut self,
        add_to_linker: fn(
            &mut wasmtime::component::Linker<Self::StoreData>,
            fn(&mut Self::StoreData) -> WasiClocksCtxView<'_>,
        ) -> wasmtime::Result<()>,
    ) -> wasmtime::Result<()> {
        add_to_linker(self.linker(), Self::get_clocks)
    }

    fn get_cli(data: &mut Self::StoreData) -> WasiCliCtxView<'_> {
        let (state, table) = Self::get_data_with_table(data);
        WasiCliCtxView {
            ctx: state.ctx.cli(),
            table,
        }
    }

    fn link_cli_bindings(
        &mut self,
        add_to_linker: fn(
            &mut wasmtime::component::Linker<Self::StoreData>,
            fn(&mut Self::StoreData) -> WasiCliCtxView<'_>,
        ) -> wasmtime::Result<()>,
    ) -> wasmtime::Result<()> {
        add_to_linker(self.linker(), Self::get_cli)
    }

    fn link_cli_default_bindings<O: Default>(
        &mut self,
        add_to_linker: fn(
            &mut wasmtime::component::Linker<Self::StoreData>,
            &O,
            fn(&mut Self::StoreData) -> WasiCliCtxView<'_>,
        ) -> wasmtime::Result<()>,
    ) -> wasmtime::Result<()> {
        add_to_linker(self.linker(), &O::default(), Self::get_cli)
    }

    fn get_filesystem(data: &mut Self::StoreData) -> WasiFilesystemCtxView<'_> {
        let (state, table) = Self::get_data_with_table(data);
        WasiFilesystemCtxView {
            ctx: state.ctx.filesystem(),
            table,
        }
    }

    fn link_filesystem_bindings(
        &mut self,
        add_to_linker: fn(
            &mut wasmtime::component::Linker<Self::StoreData>,
            fn(&mut Self::StoreData) -> WasiFilesystemCtxView<'_>,
        ) -> wasmtime::Result<()>,
    ) -> wasmtime::Result<()> {
        add_to_linker(self.linker(), Self::get_filesystem)
    }

    fn get_sockets(data: &mut Self::StoreData) -> WasiSocketsCtxView<'_> {
        let (state, table) = Self::get_data_with_table(data);
        WasiSocketsCtxView {
            ctx: state.ctx.sockets(),
            table,
        }
    }

    fn link_sockets_bindings(
        &mut self,
        add_to_linker: fn(
            &mut wasmtime::component::Linker<Self::StoreData>,
            fn(&mut Self::StoreData) -> WasiSocketsCtxView<'_>,
        ) -> wasmtime::Result<()>,
    ) -> wasmtime::Result<()> {
        add_to_linker(self.linker(), Self::get_sockets)
    }

    fn link_sockets_default_bindings<O: Default>(
        &mut self,
        add_to_linker: fn(
            &mut wasmtime::component::Linker<Self::StoreData>,
            &O,
            fn(&mut Self::StoreData) -> WasiSocketsCtxView<'_>,
        ) -> wasmtime::Result<()>,
    ) -> wasmtime::Result<()> {
        add_to_linker(self.linker(), &O::default(), Self::get_sockets)
    }

    fn get_spin_sockets(data: &mut Self::StoreData) -> SpinSocketsView<'_> {
        let (state, table) = Self::get_data_with_table(data);
        SpinSocketsView {
            inner: WasiSocketsCtxView {
                ctx: state.ctx.sockets(),
                table,
            },
            permit_state: state.socket_permit_state.clone(),
        }
    }

    fn link_spin_sockets_bindings(
        &mut self,
        add_to_linker: fn(
            &mut wasmtime::component::Linker<Self::StoreData>,
            fn(&mut Self::StoreData) -> SpinSocketsView<'_>,
        ) -> wasmtime::Result<()>,
    ) -> wasmtime::Result<()> {
        add_to_linker(self.linker(), Self::get_spin_sockets)
    }

    fn link_io_bindings(
        &mut self,
        add_to_linker: fn(
            &mut wasmtime::component::Linker<Self::StoreData>,
            fn(&mut Self::StoreData) -> &mut ResourceTable,
        ) -> wasmtime::Result<()>,
    ) -> wasmtime::Result<()> {
        add_to_linker(self.linker(), Self::get_table)
    }

    fn link_random_bindings(
        &mut self,
        add_to_linker: fn(
            &mut wasmtime::component::Linker<Self::StoreData>,
            fn(&mut Self::StoreData) -> &mut WasiRandomCtx,
        ) -> wasmtime::Result<()>,
    ) -> wasmtime::Result<()> {
        add_to_linker(self.linker(), |data| {
            let (state, _table) = Self::get_data_with_table(data);
            state.ctx.random()
        })
    }

    fn link_all_bindings(
        &mut self,
        add_to_linker: fn(
            &mut wasmtime::component::Linker<Self::StoreData>,
            fn(&mut Self::StoreData) -> &mut ResourceTable,
            fn(&mut Self::StoreData) -> &mut WasiRandomCtx,
            fn(&mut Self::StoreData) -> WasiClocksCtxView<'_>,
            fn(&mut Self::StoreData) -> WasiCliCtxView<'_>,
            fn(&mut Self::StoreData) -> WasiFilesystemCtxView<'_>,
            fn(&mut Self::StoreData) -> WasiSocketsCtxView<'_>,
        ) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        add_to_linker(
            self.linker(),
            Self::get_table,
            Self::get_random,
            Self::get_clocks,
            Self::get_cli,
            Self::get_filesystem,
            Self::get_sockets,
        )
    }
}

impl<T> InitContextExt for T where T: InitContext<WasiFactor> {}

struct HasIo;

impl HasData for HasIo {
    type Data<'a> = &'a mut ResourceTable;
}

impl Factor for WasiFactor {
    type RuntimeConfig = ();
    type AppState = ();
    type InstanceBuilder = InstanceBuilder;

    fn init(&mut self, ctx: &mut impl InitContext<Self>) -> anyhow::Result<()> {
        use wasmtime_wasi::{p2, p3};

        ctx.link_clocks_bindings(p2::bindings::clocks::wall_clock::add_to_linker::<_, WasiClocks>)?;
        ctx.link_clocks_bindings(
            p3::bindings::clocks::system_clock::add_to_linker::<_, WasiClocks>,
        )?;
        ctx.link_clocks_bindings(
            p2::bindings::clocks::monotonic_clock::add_to_linker::<_, WasiClocks>,
        )?;
        ctx.link_clocks_bindings(
            p3::bindings::clocks::monotonic_clock::add_to_linker::<_, WasiClocks>,
        )?;
        ctx.link_filesystem_bindings(
            p2::bindings::filesystem::types::add_to_linker::<_, WasiFilesystem>,
        )?;
        ctx.link_filesystem_bindings(
            p3::bindings::filesystem::types::add_to_linker::<_, WasiFilesystem>,
        )?;
        ctx.link_filesystem_bindings(
            p2::bindings::filesystem::preopens::add_to_linker::<_, WasiFilesystem>,
        )?;
        ctx.link_filesystem_bindings(
            p3::bindings::filesystem::preopens::add_to_linker::<_, WasiFilesystem>,
        )?;
        ctx.link_io_bindings(p2::bindings::io::error::add_to_linker::<_, HasIo>)?;
        ctx.link_io_bindings(p2::bindings::io::poll::add_to_linker::<_, HasIo>)?;
        ctx.link_io_bindings(p2::bindings::io::streams::add_to_linker::<_, HasIo>)?;
        ctx.link_random_bindings(p2::bindings::random::random::add_to_linker::<_, WasiRandom>)?;
        ctx.link_random_bindings(p3::bindings::random::random::add_to_linker::<_, WasiRandom>)?;
        ctx.link_random_bindings(p2::bindings::random::insecure::add_to_linker::<_, WasiRandom>)?;
        ctx.link_random_bindings(p3::bindings::random::insecure::add_to_linker::<_, WasiRandom>)?;
        ctx.link_random_bindings(
            p2::bindings::random::insecure_seed::add_to_linker::<_, WasiRandom>,
        )?;
        ctx.link_random_bindings(
            p3::bindings::random::insecure_seed::add_to_linker::<_, WasiRandom>,
        )?;
        ctx.link_cli_default_bindings(p2::bindings::cli::exit::add_to_linker::<_, WasiCli>)?;
        ctx.link_cli_default_bindings(p3::bindings::cli::exit::add_to_linker::<_, WasiCli>)?;
        ctx.link_cli_bindings(p2::bindings::cli::environment::add_to_linker::<_, WasiCli>)?;
        ctx.link_cli_bindings(p3::bindings::cli::environment::add_to_linker::<_, WasiCli>)?;
        ctx.link_cli_bindings(p2::bindings::cli::stdin::add_to_linker::<_, WasiCli>)?;
        ctx.link_cli_bindings(p3::bindings::cli::stdin::add_to_linker::<_, WasiCli>)?;
        ctx.link_cli_bindings(p2::bindings::cli::stdout::add_to_linker::<_, WasiCli>)?;
        ctx.link_cli_bindings(p3::bindings::cli::stdout::add_to_linker::<_, WasiCli>)?;
        ctx.link_cli_bindings(p2::bindings::cli::stderr::add_to_linker::<_, WasiCli>)?;
        ctx.link_cli_bindings(p3::bindings::cli::stderr::add_to_linker::<_, WasiCli>)?;
        ctx.link_cli_bindings(p2::bindings::cli::terminal_input::add_to_linker::<_, WasiCli>)?;
        ctx.link_cli_bindings(p3::bindings::cli::terminal_input::add_to_linker::<_, WasiCli>)?;
        ctx.link_cli_bindings(p2::bindings::cli::terminal_output::add_to_linker::<_, WasiCli>)?;
        ctx.link_cli_bindings(p3::bindings::cli::terminal_output::add_to_linker::<_, WasiCli>)?;
        ctx.link_cli_bindings(p2::bindings::cli::terminal_stdin::add_to_linker::<_, WasiCli>)?;
        ctx.link_cli_bindings(p3::bindings::cli::terminal_stdin::add_to_linker::<_, WasiCli>)?;
        ctx.link_cli_bindings(p2::bindings::cli::terminal_stdout::add_to_linker::<_, WasiCli>)?;
        ctx.link_cli_bindings(p3::bindings::cli::terminal_stdout::add_to_linker::<_, WasiCli>)?;
        ctx.link_cli_bindings(p2::bindings::cli::terminal_stderr::add_to_linker::<_, WasiCli>)?;
        ctx.link_cli_bindings(p3::bindings::cli::terminal_stderr::add_to_linker::<_, WasiCli>)?;
        ctx.link_spin_sockets_bindings(
            p2::bindings::sockets::tcp::add_to_linker::<_, SpinSockets>,
        )?;
        ctx.link_spin_sockets_bindings(
            p2::bindings::sockets::tcp_create_socket::add_to_linker::<_, SpinSockets>,
        )?;
        ctx.link_spin_sockets_bindings(
            p2::bindings::sockets::udp::add_to_linker::<_, SpinSockets>,
        )?;
        ctx.link_spin_sockets_bindings(
            p2::bindings::sockets::udp_create_socket::add_to_linker::<_, SpinSockets>,
        )?;
        ctx.link_sockets_bindings(
            p2::bindings::sockets::instance_network::add_to_linker::<_, WasiSockets>,
        )?;
        ctx.link_sockets_default_bindings(
            p2::bindings::sockets::network::add_to_linker::<_, WasiSockets>,
        )?;
        ctx.link_sockets_bindings(
            p2::bindings::sockets::ip_name_lookup::add_to_linker::<_, WasiSockets>,
        )?;
        ctx.link_sockets_bindings(
            p3::bindings::sockets::ip_name_lookup::add_to_linker::<_, WasiSockets>,
        )?;
        ctx.link_sockets_bindings(p3::bindings::sockets::types::add_to_linker::<_, WasiSockets>)?;

        ctx.link_all_bindings(wasi_2023_10_18::add_to_linker)?;
        ctx.link_all_bindings(wasi_2023_11_10::add_to_linker)?;
        Ok(())
    }

    fn configure_app<T: RuntimeFactors>(
        &self,
        _ctx: spin_factors::ConfigureAppContext<T, Self>,
    ) -> anyhow::Result<Self::AppState> {
        Ok(())
    }

    fn prepare<T: RuntimeFactors>(
        &self,
        ctx: PrepareContext<T, Self>,
    ) -> anyhow::Result<InstanceBuilder> {
        let mut wasi_ctx = WasiCtxBuilder::new();

        // Mount files
        let mount_ctx = MountFilesContext { ctx: &mut wasi_ctx };
        self.files_mounter
            .mount_files(ctx.app_component(), mount_ctx)?;

        let mut builder = InstanceBuilder {
            ctx: wasi_ctx,
            socket_permit_state: None,
        };

        // Apply environment variables
        builder.env(ctx.app_component().environment());

        Ok(builder)
    }
}

pub trait FilesMounter: Send + Sync {
    fn mount_files(
        &self,
        app_component: &AppComponent,
        ctx: MountFilesContext,
    ) -> anyhow::Result<()>;
}

pub struct DummyFilesMounter;

impl FilesMounter for DummyFilesMounter {
    fn mount_files(
        &self,
        app_component: &AppComponent,
        _ctx: MountFilesContext,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            app_component.files().next().is_none(),
            "DummyFilesMounter can't actually mount files"
        );
        Ok(())
    }
}

pub struct MountFilesContext<'a> {
    ctx: &'a mut WasiCtxBuilder,
}

impl MountFilesContext<'_> {
    pub fn preopened_dir(
        &mut self,
        host_path: impl AsRef<Path>,
        guest_path: impl AsRef<str>,
        writable: bool,
    ) -> anyhow::Result<()> {
        let (dir_perms, file_perms) = if writable {
            (DirPerms::all(), FilePerms::all())
        } else {
            (DirPerms::READ, FilePerms::READ)
        };
        self.ctx
            .preopened_dir(host_path, guest_path, dir_perms, file_perms)?;
        Ok(())
    }
}

pub struct InstanceBuilder {
    ctx: WasiCtxBuilder,
    socket_permit_state: Option<Arc<SocketPermitState>>,
}

impl InstanceBuilder {
    /// Sets the WASI `stdin` descriptor to the given [`StdinStream`].
    pub fn stdin(&mut self, stdin: impl StdinStream + 'static) {
        self.ctx.stdin(stdin);
    }

    /// Sets the WASI `stdin` descriptor to the given [`Read`]er.
    pub fn stdin_pipe(&mut self, r: impl Read + Send + Sync + Unpin + 'static) {
        self.stdin(PipeReadStream::new(r));
    }

    /// Sets the WASI `stdout` descriptor to the given [`StdoutStream`].
    pub fn stdout(&mut self, stdout: impl StdoutStream + 'static) {
        self.ctx.stdout(stdout);
    }

    /// Sets the WASI `stdout` descriptor to the given [`Write`]r.
    pub fn stdout_pipe(&mut self, w: impl Write + Send + Sync + Unpin + 'static) {
        self.stdout(PipedWriteStream::new(w));
    }

    /// Sets the WASI `stderr` descriptor to the given [`StdoutStream`].
    pub fn stderr(&mut self, stderr: impl StdoutStream + 'static) {
        self.ctx.stderr(stderr);
    }

    /// Sets the WASI `stderr` descriptor to the given [`Write`]r.
    pub fn stderr_pipe(&mut self, w: impl Write + Send + Sync + Unpin + 'static) {
        self.stderr(PipedWriteStream::new(w));
    }

    /// Appends the given strings to the WASI 'args'.
    pub fn args(&mut self, args: impl IntoIterator<Item = impl AsRef<str>>) {
        for arg in args {
            self.ctx.arg(arg);
        }
    }

    /// Sets the given key/value string entries on the WASI 'env'.
    pub fn env(&mut self, vars: impl IntoIterator<Item = (impl AsRef<str>, impl AsRef<str>)>) {
        for (k, v) in vars {
            self.ctx.env(k, v);
        }
    }

    /// "Mounts" the given `host_path` into the WASI filesystem at the given
    /// `guest_path`.
    pub fn preopened_dir(
        &mut self,
        host_path: impl AsRef<Path>,
        guest_path: impl AsRef<str>,
        writable: bool,
    ) -> anyhow::Result<()> {
        let (dir_perms, file_perms) = if writable {
            (DirPerms::all(), FilePerms::all())
        } else {
            (DirPerms::READ, FilePerms::READ)
        };
        self.ctx
            .preopened_dir(host_path, guest_path, dir_perms, file_perms)?;
        Ok(())
    }
}

impl FactorInstanceBuilder for InstanceBuilder {
    type InstanceState = InstanceState;

    fn build(self) -> anyhow::Result<Self::InstanceState> {
        let InstanceBuilder {
            ctx: mut wasi_ctx,
            socket_permit_state,
        } = self;
        Ok(InstanceState {
            ctx: wasi_ctx.build(),
            socket_permit_state,
        })
    }
}

impl InstanceBuilder {
    /// Sets the socket permit state for per-connection quota tracking.
    pub fn set_socket_permit_state(&mut self, state: Arc<SocketPermitState>) {
        self.socket_permit_state = Some(state);
    }

    pub fn outbound_socket_addr_check<F, Fut>(&mut self, check: F)
    where
        F: Fn(SocketAddr, SocketAddrUse) -> Fut + Send + Sync + Clone + 'static,
        Fut: Future<Output = bool> + Send + Sync,
    {
        self.ctx.socket_addr_check(move |addr, addr_use| {
            let check = check.clone();
            Box::pin(async move {
                match addr_use {
                    SocketAddrUse::TcpBind => false,
                    SocketAddrUse::TcpConnect
                    | SocketAddrUse::UdpBind
                    | SocketAddrUse::UdpConnect
                    | SocketAddrUse::UdpOutgoingDatagram => check(addr, addr_use).await,
                }
            })
        });
    }
}

pub struct InstanceState {
    ctx: WasiCtx,
    socket_permit_state: Option<Arc<SocketPermitState>>,
}
