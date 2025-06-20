mod io;
pub mod spin;
mod wasi_2023_10_18;
mod wasi_2023_11_10;

use std::{
    future::Future,
    io::{Read, Write},
    net::SocketAddr,
    path::Path,
};

use io::{PipeReadStream, PipedWriteStream};
use spin_factors::{
    anyhow, AppComponent, Factor, FactorInstanceBuilder, InitContext, PrepareContext,
    RuntimeFactors, RuntimeFactorsInstanceState,
};
use wasmtime::component::HasData;
use wasmtime_wasi::p2::{
    IoImpl, IoView, StdinStream, StdoutStream, WasiCtx, WasiCtxBuilder, WasiImpl, WasiView,
};
use wasmtime_wasi::{DirPerms, FilePerms, ResourceTable};

pub use wasmtime_wasi::SocketAddrUse;

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
    ) -> Option<WasiImpl<impl WasiView + '_>> {
        let (state, table) = runtime_instance_state.get_with_table::<WasiFactor>()?;
        Some(WasiImpl(IoImpl(WasiImplInner {
            ctx: &mut state.ctx,
            table,
        })))
    }
}

/// Helper trait to extend `InitContext` with some more `link_*_bindings`
/// methods related to `wasmtime-wasi` and `wasmtime-wasi-io`-specific
/// signatures.
#[allow(clippy::type_complexity, reason = "sorry, blame alex")]
trait InitContextExt: InitContext<WasiFactor> {
    fn get_io(data: &mut Self::StoreData) -> IoImpl<WasiImplInner<'_>> {
        let (state, table) = Self::get_data_with_table(data);
        IoImpl(WasiImplInner {
            ctx: &mut state.ctx,
            table,
        })
    }

    fn link_io_bindings(
        &mut self,
        add_to_linker: fn(
            &mut wasmtime::component::Linker<Self::StoreData>,
            fn(&mut Self::StoreData) -> IoImpl<WasiImplInner<'_>>,
        ) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        add_to_linker(self.linker(), Self::get_io)
    }

    fn get_wasi(data: &mut Self::StoreData) -> WasiImpl<WasiImplInner<'_>> {
        WasiImpl(Self::get_io(data))
    }

    fn link_wasi_bindings(
        &mut self,
        add_to_linker: fn(
            &mut wasmtime::component::Linker<Self::StoreData>,
            fn(&mut Self::StoreData) -> WasiImpl<WasiImplInner<'_>>,
        ) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        add_to_linker(self.linker(), Self::get_wasi)
    }

    fn link_wasi_default_bindings<O>(
        &mut self,
        add_to_linker: fn(
            &mut wasmtime::component::Linker<Self::StoreData>,
            &O,
            fn(&mut Self::StoreData) -> WasiImpl<WasiImplInner<'_>>,
        ) -> anyhow::Result<()>,
    ) -> anyhow::Result<()>
    where
        O: Default,
    {
        add_to_linker(self.linker(), &O::default(), Self::get_wasi)
    }
}

impl<T> InitContextExt for T where T: InitContext<WasiFactor> {}

struct HasWasi;

impl HasData for HasWasi {
    type Data<'a> = WasiImpl<WasiImplInner<'a>>;
}

struct HasIo;

impl HasData for HasIo {
    type Data<'a> = IoImpl<WasiImplInner<'a>>;
}

impl Factor for WasiFactor {
    type RuntimeConfig = ();
    type AppState = ();
    type InstanceBuilder = InstanceBuilder;

    fn init(&mut self, ctx: &mut impl InitContext<Self>) -> anyhow::Result<()> {
        use wasmtime_wasi::p2::bindings;

        ctx.link_wasi_bindings(bindings::clocks::wall_clock::add_to_linker::<_, HasWasi>)?;
        ctx.link_wasi_bindings(bindings::clocks::monotonic_clock::add_to_linker::<_, HasWasi>)?;
        ctx.link_wasi_bindings(bindings::filesystem::types::add_to_linker::<_, HasWasi>)?;
        ctx.link_wasi_bindings(bindings::filesystem::preopens::add_to_linker::<_, HasWasi>)?;
        ctx.link_io_bindings(bindings::io::error::add_to_linker::<_, HasIo>)?;
        ctx.link_io_bindings(bindings::io::poll::add_to_linker::<_, HasIo>)?;
        ctx.link_io_bindings(bindings::io::streams::add_to_linker::<_, HasIo>)?;
        ctx.link_wasi_bindings(bindings::random::random::add_to_linker::<_, HasWasi>)?;
        ctx.link_wasi_bindings(bindings::random::insecure::add_to_linker::<_, HasWasi>)?;
        ctx.link_wasi_bindings(bindings::random::insecure_seed::add_to_linker::<_, HasWasi>)?;
        ctx.link_wasi_default_bindings(bindings::cli::exit::add_to_linker::<_, HasWasi>)?;
        ctx.link_wasi_bindings(bindings::cli::environment::add_to_linker::<_, HasWasi>)?;
        ctx.link_wasi_bindings(bindings::cli::stdin::add_to_linker::<_, HasWasi>)?;
        ctx.link_wasi_bindings(bindings::cli::stdout::add_to_linker::<_, HasWasi>)?;
        ctx.link_wasi_bindings(bindings::cli::stderr::add_to_linker::<_, HasWasi>)?;
        ctx.link_wasi_bindings(bindings::cli::terminal_input::add_to_linker::<_, HasWasi>)?;
        ctx.link_wasi_bindings(bindings::cli::terminal_output::add_to_linker::<_, HasWasi>)?;
        ctx.link_wasi_bindings(bindings::cli::terminal_stdin::add_to_linker::<_, HasWasi>)?;
        ctx.link_wasi_bindings(bindings::cli::terminal_stdout::add_to_linker::<_, HasWasi>)?;
        ctx.link_wasi_bindings(bindings::cli::terminal_stderr::add_to_linker::<_, HasWasi>)?;
        ctx.link_wasi_bindings(bindings::sockets::tcp::add_to_linker::<_, HasWasi>)?;
        ctx.link_wasi_bindings(bindings::sockets::tcp_create_socket::add_to_linker::<_, HasWasi>)?;
        ctx.link_wasi_bindings(bindings::sockets::udp::add_to_linker::<_, HasWasi>)?;
        ctx.link_wasi_bindings(bindings::sockets::udp_create_socket::add_to_linker::<_, HasWasi>)?;
        ctx.link_wasi_bindings(bindings::sockets::instance_network::add_to_linker::<_, HasWasi>)?;
        ctx.link_wasi_default_bindings(bindings::sockets::network::add_to_linker::<_, HasWasi>)?;
        ctx.link_wasi_bindings(bindings::sockets::ip_name_lookup::add_to_linker::<_, HasWasi>)?;

        ctx.link_wasi_bindings(wasi_2023_10_18::add_to_linker)?;
        ctx.link_wasi_bindings(wasi_2023_11_10::add_to_linker)?;
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

        let mut builder = InstanceBuilder { ctx: wasi_ctx };

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
        let InstanceBuilder { ctx: mut wasi_ctx } = self;
        Ok(InstanceState {
            ctx: wasi_ctx.build(),
        })
    }
}

impl InstanceBuilder {
    pub fn outbound_socket_addr_check<F, Fut>(&mut self, check: F)
    where
        F: Fn(SocketAddr, SocketAddrUse) -> Fut + Send + Sync + Clone + 'static,
        Fut: Future<Output = bool> + Send + Sync,
    {
        self.ctx.socket_addr_check(move |addr, addr_use| {
            let check = check.clone();
            Box::pin(async move {
                match addr_use {
                    wasmtime_wasi::SocketAddrUse::TcpBind => false,
                    wasmtime_wasi::SocketAddrUse::TcpConnect
                    | wasmtime_wasi::SocketAddrUse::UdpBind
                    | wasmtime_wasi::SocketAddrUse::UdpConnect
                    | wasmtime_wasi::SocketAddrUse::UdpOutgoingDatagram => {
                        check(addr, addr_use).await
                    }
                }
            })
        });
    }
}

pub struct InstanceState {
    ctx: WasiCtx,
}

struct WasiImplInner<'a> {
    ctx: &'a mut WasiCtx,
    table: &'a mut ResourceTable,
}

impl WasiView for WasiImplInner<'_> {
    fn ctx(&mut self) -> &mut WasiCtx {
        self.ctx
    }
}

impl IoView for WasiImplInner<'_> {
    fn table(&mut self) -> &mut ResourceTable {
        self.table
    }
}
