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
use wasmtime_wasi::cli::{StdinStream, StdoutStream, WasiCliCtxView};
use wasmtime_wasi::clocks::WasiClocksCtxView;
use wasmtime_wasi::filesystem::WasiFilesystemCtxView;
use wasmtime_wasi::random::WasiRandomCtx;
use wasmtime_wasi::sockets::WasiSocketsCtxView;
use wasmtime_wasi::{DirPerms, FilePerms, ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView};

pub use wasmtime_wasi::sockets::SocketAddrUse;

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
    ) -> Option<WasiSocketsCtxView<'_>> {
        let (state, table) = runtime_instance_state.get_with_table::<WasiFactor>()?;
        Some(WasiSocketsCtxView {
            ctx: state.ctx.sockets(),
            table,
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

    fn link_clocks_bindings(
        &mut self,
        add_to_linker: fn(
            &mut wasmtime::component::Linker<Self::StoreData>,
            fn(&mut Self::StoreData) -> WasiClocksCtxView<'_>,
        ) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
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
        ) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        add_to_linker(self.linker(), Self::get_cli)
    }

    fn link_cli_default_bindings<O: Default>(
        &mut self,
        add_to_linker: fn(
            &mut wasmtime::component::Linker<Self::StoreData>,
            &O,
            fn(&mut Self::StoreData) -> WasiCliCtxView<'_>,
        ) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
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
        ) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
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
        ) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        add_to_linker(self.linker(), Self::get_sockets)
    }

    fn link_sockets_default_bindings<O: Default>(
        &mut self,
        add_to_linker: fn(
            &mut wasmtime::component::Linker<Self::StoreData>,
            &O,
            fn(&mut Self::StoreData) -> WasiSocketsCtxView<'_>,
        ) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        add_to_linker(self.linker(), &O::default(), Self::get_sockets)
    }

    fn link_io_bindings(
        &mut self,
        add_to_linker: fn(
            &mut wasmtime::component::Linker<Self::StoreData>,
            fn(&mut Self::StoreData) -> &mut ResourceTable,
        ) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        add_to_linker(self.linker(), Self::get_table)
    }

    fn get_wasi(data: &mut Self::StoreData) -> WasiCtxView<'_> {
        let (state, table) = Self::get_data_with_table(data);
        WasiCtxView {
            ctx: &mut state.ctx,
            table,
        }
    }

    fn link_random_bindings(
        &mut self,
        add_to_linker: fn(
            &mut wasmtime::component::Linker<Self::StoreData>,
            fn(&mut Self::StoreData) -> &mut WasiRandomCtx,
        ) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        add_to_linker(self.linker(), |data| {
            let (state, _table) = Self::get_data_with_table(data);
            state.ctx.random()
        })
    }

    fn link_all_bindings(
        &mut self,
        add_to_linker: fn(
            &mut wasmtime::component::Linker<Self::StoreData>,
            fn(&mut Self::StoreData) -> WasiCtxView<'_>,
            fn(&mut Self::StoreData) -> WasiClocksCtxView<'_>,
            fn(&mut Self::StoreData) -> WasiCliCtxView<'_>,
            fn(&mut Self::StoreData) -> WasiFilesystemCtxView<'_>,
            fn(&mut Self::StoreData) -> WasiSocketsCtxView<'_>,
        ) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        add_to_linker(
            self.linker(),
            Self::get_wasi,
            Self::get_clocks,
            Self::get_cli,
            Self::get_filesystem,
            Self::get_sockets,
        )
    }
}

impl<T> InitContextExt for T where T: InitContext<WasiFactor> {}

struct HasWasi;

impl HasData for HasWasi {
    type Data<'a> = WasiCtxView<'a>;
}

struct HasClocks;

impl HasData for HasClocks {
    type Data<'a> = WasiClocksCtxView<'a>;
}

struct HasCli;

impl HasData for HasCli {
    type Data<'a> = WasiCliCtxView<'a>;
}

struct HasFilesystem;

impl HasData for HasFilesystem {
    type Data<'a> = WasiFilesystemCtxView<'a>;
}

struct HasSockets;

impl HasData for HasSockets {
    type Data<'a> = WasiSocketsCtxView<'a>;
}

struct HasIo;

impl HasData for HasIo {
    type Data<'a> = &'a mut ResourceTable;
}

struct HasRandom;

impl HasData for HasRandom {
    type Data<'a> = &'a mut WasiRandomCtx;
}

impl Factor for WasiFactor {
    type RuntimeConfig = ();
    type AppState = ();
    type InstanceBuilder = InstanceBuilder;

    fn init(&mut self, ctx: &mut impl InitContext<Self>) -> anyhow::Result<()> {
        use wasmtime_wasi::p2::bindings;

        ctx.link_clocks_bindings(bindings::clocks::wall_clock::add_to_linker::<_, HasClocks>)?;
        ctx.link_clocks_bindings(bindings::clocks::monotonic_clock::add_to_linker::<_, HasClocks>)?;
        ctx.link_filesystem_bindings(
            bindings::filesystem::types::add_to_linker::<_, HasFilesystem>,
        )?;
        ctx.link_filesystem_bindings(
            bindings::filesystem::preopens::add_to_linker::<_, HasFilesystem>,
        )?;
        ctx.link_io_bindings(bindings::io::error::add_to_linker::<_, HasIo>)?;
        ctx.link_io_bindings(bindings::io::poll::add_to_linker::<_, HasIo>)?;
        ctx.link_io_bindings(bindings::io::streams::add_to_linker::<_, HasIo>)?;
        ctx.link_random_bindings(bindings::random::random::add_to_linker::<_, HasRandom>)?;
        ctx.link_random_bindings(bindings::random::insecure::add_to_linker::<_, HasRandom>)?;
        ctx.link_random_bindings(bindings::random::insecure_seed::add_to_linker::<_, HasRandom>)?;
        ctx.link_cli_default_bindings(bindings::cli::exit::add_to_linker::<_, HasCli>)?;
        ctx.link_cli_bindings(bindings::cli::environment::add_to_linker::<_, HasCli>)?;
        ctx.link_cli_bindings(bindings::cli::stdin::add_to_linker::<_, HasCli>)?;
        ctx.link_cli_bindings(bindings::cli::stdout::add_to_linker::<_, HasCli>)?;
        ctx.link_cli_bindings(bindings::cli::stderr::add_to_linker::<_, HasCli>)?;
        ctx.link_cli_bindings(bindings::cli::terminal_input::add_to_linker::<_, HasCli>)?;
        ctx.link_cli_bindings(bindings::cli::terminal_output::add_to_linker::<_, HasCli>)?;
        ctx.link_cli_bindings(bindings::cli::terminal_stdin::add_to_linker::<_, HasCli>)?;
        ctx.link_cli_bindings(bindings::cli::terminal_stdout::add_to_linker::<_, HasCli>)?;
        ctx.link_cli_bindings(bindings::cli::terminal_stderr::add_to_linker::<_, HasCli>)?;
        ctx.link_sockets_bindings(bindings::sockets::tcp::add_to_linker::<_, HasSockets>)?;
        ctx.link_sockets_bindings(
            bindings::sockets::tcp_create_socket::add_to_linker::<_, HasSockets>,
        )?;
        ctx.link_sockets_bindings(bindings::sockets::udp::add_to_linker::<_, HasSockets>)?;
        ctx.link_sockets_bindings(
            bindings::sockets::udp_create_socket::add_to_linker::<_, HasSockets>,
        )?;
        ctx.link_sockets_bindings(
            bindings::sockets::instance_network::add_to_linker::<_, HasSockets>,
        )?;
        ctx.link_sockets_default_bindings(
            bindings::sockets::network::add_to_linker::<_, HasSockets>,
        )?;
        ctx.link_sockets_bindings(
            bindings::sockets::ip_name_lookup::add_to_linker::<_, HasSockets>,
        )?;

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
}
