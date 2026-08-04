mod io;
pub mod sockets;
pub mod spin;
mod wasi_2023_10_18;
mod wasi_2023_11_10;
mod wasi_2026_03_15;

use std::{
    collections::HashMap,
    future::Future,
    io::{Read, Write},
    net::SocketAddr,
    path::Path,
    sync::Arc,
};

use io::{PipeReadStream, PipedWriteStream};
use spin_factors::{
    Factor, FactorField, FactorInstanceBuilder, InitContext, PrepareContext, RuntimeFactors,
    RuntimeFactorsInstanceState, anyhow,
};
use spin_serde::CapabilitySetKey;
use wasmtime::component::HasData;
use wasmtime_wasi::cli::{StdinStream, StdoutStream, WasiCli, WasiCliCtxView};
use wasmtime_wasi::clocks::{WasiClocks, WasiClocksCtxView};
use wasmtime_wasi::filesystem::{WasiFilesystem, WasiFilesystemCtxView};
use wasmtime_wasi::random::{WasiRandom, WasiRandomCtx};
use wasmtime_wasi::sockets::{WasiSockets, WasiSocketsCtxView};
use wasmtime_wasi::{FsPerms, ResourceTable, WasiCtx, WasiCtxBuilder, WasiCtxView};

pub use sockets::{SocketPermitState, SpinSockets, SpinSocketsView};
pub use wasi_2023_10_18::convert_result;
pub use wasi_2026_03_15::{FutureReaderExt, StreamReaderExt, reborrow};
pub use wasmtime_wasi::sockets::SocketAddrUse;

pub struct WasiFactor {
    files_mounter: Box<dyn FilesMounter>,
    cap_set_id: std::sync::RwLock<HashMap<CapabilitySetKey, wasmtime_wasi::NamedId>>,
    id_to_csk: Arc<std::sync::RwLock<Vec<CapabilitySetKey>>>,
}

impl WasiFactor {
    pub fn new(files_mounter: impl FilesMounter + 'static) -> Self {
        Self {
            files_mounter: Box::new(files_mounter),
            cap_set_id: Default::default(),
            id_to_csk: Default::default(),
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

    pub fn get_sockets_impl<T>(
        runtime_instance_state: &mut impl RuntimeFactorsInstanceState,
        getter: fn(&mut T) -> WasiSocketsCtxView<'_>,
    ) -> Option<SpinSocketsView<'_, T>> {
        let (state, table) = runtime_instance_state.get_with_table::<WasiFactor>()?;
        Some(SpinSocketsView {
            inner: WasiSocketsCtxView {
                ctx: state.ctx.sockets(),
                table,
            },
            permit_state: state.socket_permit_state.clone(),
            getter,
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

    fn get_spin_sockets(data: &mut Self::StoreData) -> SpinSocketsView<'_, Self::StoreData> {
        let (state, table) = Self::get_data_with_table(data);
        SpinSocketsView {
            inner: WasiSocketsCtxView {
                ctx: state.ctx.sockets(),
                table,
            },
            permit_state: state.socket_permit_state.clone(),
            getter: Self::get_wasi_sockets,
        }
    }

    fn get_wasi_sockets(data: &mut Self::StoreData) -> WasiSocketsCtxView<'_> {
        let (state, table) = Self::get_data_with_table(data);
        WasiSocketsCtxView {
            ctx: state.ctx.sockets(),
            table,
        }
    }

    fn link_spin_sockets_bindings(
        &mut self,
        add_to_linker: fn(
            &mut wasmtime::component::Linker<Self::StoreData>,
            fn(&mut Self::StoreData) -> SpinSocketsView<'_, Self::StoreData>,
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
            fn(&mut Self::StoreData) -> SpinSocketsView<'_, Self::StoreData>,
        ) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        add_to_linker(
            self.linker(),
            Self::get_table,
            Self::get_random,
            Self::get_clocks,
            Self::get_cli,
            Self::get_filesystem,
            Self::get_spin_sockets,
        )
    }

    fn link_all_p3_bindings(
        &mut self,
        add_to_linker: fn(
            &mut wasmtime::component::Linker<Self::StoreData>,
            fn(&mut Self::StoreData) -> &mut WasiRandomCtx,
            fn(&mut Self::StoreData) -> WasiClocksCtxView<'_>,
            fn(&mut Self::StoreData) -> WasiCliCtxView<'_>,
            fn(&mut Self::StoreData) -> WasiFilesystemCtxView<'_>,
            fn(&mut Self::StoreData) -> SpinSocketsView<'_, Self::StoreData>,
            fn(&mut Self::StoreData) -> WasiSocketsCtxView<'_>,
        ) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        add_to_linker(
            self.linker(),
            Self::get_random,
            Self::get_clocks,
            Self::get_cli,
            Self::get_filesystem,
            Self::get_spin_sockets,
            Self::get_wasi_sockets,
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

    fn init<T: InitContext<Self>>(&mut self, ctx: &mut T) -> anyhow::Result<()> {
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
        ctx.link_cli_bindings(p2::bindings::cli::exit::add_to_linker::<_, WasiCli>)?;
        ctx.link_cli_bindings(p3::bindings::cli::exit::add_to_linker::<_, WasiCli>)?;
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
            p2::bindings::sockets::tcp::add_to_linker::<_, SpinSockets<T::StoreData>>,
        )?;
        ctx.link_spin_sockets_bindings(
            p2::bindings::sockets::tcp_create_socket::add_to_linker::<_, SpinSockets<T::StoreData>>,
        )?;
        ctx.link_spin_sockets_bindings(
            p2::bindings::sockets::udp::add_to_linker::<_, SpinSockets<T::StoreData>>,
        )?;
        ctx.link_spin_sockets_bindings(
            p2::bindings::sockets::udp_create_socket::add_to_linker::<_, SpinSockets<T::StoreData>>,
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
        ctx.link_spin_sockets_bindings(
            p3::bindings::sockets::types::add_to_linker::<_, SpinSockets<T::StoreData>>,
        )?;

        ctx.link_all_bindings(wasi_2023_10_18::add_to_linker)?;
        ctx.link_all_bindings(wasi_2023_11_10::add_to_linker)?;
        ctx.link_all_p3_bindings(wasi_2026_03_15::add_to_linker)?;
        Ok(())
    }

    fn configure_app<T: RuntimeFactors>(
        &self,
        ctx: spin_factors::ConfigureAppContext<T, Self>,
    ) -> anyhow::Result<Self::AppState> {
        for component in ctx.app().components() {
            for caps in component.locked.dependency_capabilities() {
                self.register_capability_set(&caps.capabilities_key)?;
            }
        }

        Ok(())
    }

    fn register_named_imports<T: InitContext<Self>>(
        &self,
        ctx: &mut T,
        component: &wasmtime::component::Component,
    ) -> anyhow::Result<()> {
        use wasmtime_wasi::{p2, p3};

        p2::bindings::named_imports::wasi::cli::environment::add_to_linker::<
            _,
            wasmtime_wasi::cli::WasiCliNamed<_>,
        >(
            ctx.linker(),
            component,
            |name| self.lookup_named_import(name),
            |x| wasmtime_wasi::WasiCtxNamedView(WasiStoreDataWrapper::<T::Field>::from_mut(x)),
        )?;
        p2::bindings::named_imports::wasi::filesystem::types::add_to_linker::<
            _,
            wasmtime_wasi::filesystem::WasiFilesystemNamed<_>,
        >(
            ctx.linker(),
            component,
            |name| self.lookup_named_import(name),
            |x| wasmtime_wasi::WasiCtxNamedView(WasiStoreDataWrapper::<T::Field>::from_mut(x)),
        )?;
        p2::bindings::named_imports::wasi::filesystem::preopens::add_to_linker::<
            _,
            wasmtime_wasi::filesystem::WasiFilesystemNamed<_>,
        >(
            ctx.linker(),
            component,
            |name| self.lookup_named_import(name),
            |x| wasmtime_wasi::WasiCtxNamedView(WasiStoreDataWrapper::<T::Field>::from_mut(x)),
        )?;

        p3::bindings::named_imports::wasi::cli::environment::add_to_linker::<
            _,
            wasmtime_wasi::cli::WasiCliNamed<_>,
        >(
            ctx.linker(),
            component,
            |name| self.lookup_named_import(name),
            |x| wasmtime_wasi::WasiCtxNamedView(WasiStoreDataWrapper::<T::Field>::from_mut(x)),
        )?;
        p3::bindings::named_imports::wasi::filesystem::types::add_to_linker::<
            _,
            wasmtime_wasi::filesystem::WasiFilesystemNamed<_>,
        >(
            ctx.linker(),
            component,
            |name| self.lookup_named_import(name),
            |x| wasmtime_wasi::WasiCtxNamedView(WasiStoreDataWrapper::<T::Field>::from_mut(x)),
        )?;
        p3::bindings::named_imports::wasi::filesystem::preopens::add_to_linker::<
            _,
            wasmtime_wasi::filesystem::WasiFilesystemNamed<_>,
        >(
            ctx.linker(),
            component,
            |name| self.lookup_named_import(name),
            |x| wasmtime_wasi::WasiCtxNamedView(WasiStoreDataWrapper::<T::Field>::from_mut(x)),
        )?;

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
            .mount_files(ctx.app_component().files(), mount_ctx)?;

        let mut dependency_capabilities = std::collections::HashMap::new();
        for caps in ctx.app_component().locked.dependency_capabilities() {
            let mut dep_wasi_ctx = WasiCtxBuilder::new();
            let dep_mount_ctx = MountFilesContext {
                ctx: &mut dep_wasi_ctx,
            };
            self.files_mounter
                .mount_files(caps.files.iter(), dep_mount_ctx)?;
            for (k, v) in &caps.environment {
                dep_wasi_ctx.env(k, v);
            }
            // TODO: AOH key?
            dependency_capabilities.insert(self.named_id_of(&caps.capabilities_key)?, dep_wasi_ctx);
        }
        let dependency_capabilities = SpinNamedWasiCtxBuilder {
            ids: dependency_capabilities,
        };

        let mut builder = InstanceBuilder {
            ctx: wasi_ctx,
            socket_permit_state: None,
            dependency_capabilities,
            id_to_csk: Arc::new(self.id_to_csk.read().unwrap().clone()),
        };

        // Apply environment variables
        builder.env(ctx.app_component().environment());

        Ok(builder)
    }
}

impl WasiFactor {
    fn lookup_named_import(
        &self,
        named_import_name: &str,
    ) -> wasmtime::Result<wasmtime_wasi::NamedId> {
        let named_import_key = spin_serde::NamedImportKey::try_from(named_import_name)
            .map_err(wasmtime::Error::from_anyhow)?;
        self.named_id_of(named_import_key.capability_set())
    }

    fn named_id_of(&self, csk: &CapabilitySetKey) -> wasmtime::Result<wasmtime_wasi::NamedId> {
        let named_id_opt = self
            .cap_set_id
            .read()
            .map_err(|e| wasmtime::Error::msg(e.to_string()))?
            .get(csk)
            .cloned();
        named_id_opt.ok_or_else(|| wasmtime::Error::msg("named import ID not found"))
    }

    fn register_capability_set(
        &self,
        cap_set_key: &CapabilitySetKey,
    ) -> wasmtime::Result<wasmtime_wasi::NamedId> {
        if let Ok(id) = self.named_id_of(cap_set_key) {
            return Ok(id);
        }
        // TODO: maybe this should be a single helper struct
        let mut cap_set_id = self
            .cap_set_id
            .write()
            .map_err(|e| wasmtime::Error::msg(e.to_string()))?;
        let mut id_to_csk = self
            .id_to_csk
            .write()
            .map_err(|e| wasmtime::Error::msg(e.to_string()))?;
        let id = wasmtime_wasi::NamedId(id_to_csk.len());
        id_to_csk.push(cap_set_key.clone());
        cap_set_id.insert(cap_set_key.clone(), id);
        Ok(id)
    }
}

struct SpinNamedWasiCtxBuilder {
    ids: std::collections::HashMap<wasmtime_wasi::NamedId, WasiCtxBuilder>,
}

pub trait FilesMounter: Send + Sync {
    fn mount_files(
        &self,
        files: std::slice::Iter<'_, spin_factors::ContentPath>,
        // app_component: &AppComponent,
        ctx: MountFilesContext,
    ) -> anyhow::Result<()>;
}

pub struct DummyFilesMounter;

impl FilesMounter for DummyFilesMounter {
    fn mount_files(
        &self,
        mut files: std::slice::Iter<'_, spin_factors::ContentPath>,
        // app_component: &AppComponent,
        _ctx: MountFilesContext,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            files.next().is_none(),
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
        let perms = if writable {
            FsPerms::ReadWrite
        } else {
            FsPerms::ReadOnly
        };
        self.ctx.preopened_dir(host_path, guest_path, perms)?;
        Ok(())
    }
}

pub struct InstanceBuilder {
    ctx: WasiCtxBuilder,
    socket_permit_state: Option<Arc<SocketPermitState>>,
    dependency_capabilities: SpinNamedWasiCtxBuilder,
    id_to_csk: Arc<Vec<CapabilitySetKey>>,
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
        let perms = if writable {
            FsPerms::ReadWrite
        } else {
            FsPerms::ReadOnly
        };
        self.ctx.preopened_dir(host_path, guest_path, perms)?;
        Ok(())
    }

    pub fn allow_tcp_udp(&mut self, enable: bool) {
        self.ctx.allow_tcp(enable).allow_udp(enable);
    }
}

impl FactorInstanceBuilder for InstanceBuilder {
    type InstanceState = InstanceState;

    fn build(self) -> anyhow::Result<Self::InstanceState> {
        let InstanceBuilder {
            ctx: mut wasi_ctx,
            socket_permit_state,
            dependency_capabilities,
            id_to_csk: _,
        } = self;
        Ok(InstanceState {
            ctx: wasi_ctx.build(),
            socket_permit_state,
            dependency_ctx: dependency_capabilities
                .ids
                .into_iter()
                .map(|(id, mut builder)| (id, builder.build()))
                .collect(),
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
        F: Fn(Option<CapabilitySetKey>, SocketAddr, SocketAddrUse) -> Fut
            + Send
            + Sync
            + Clone
            + 'static,
        Fut: Future<Output = bool> + Send + Sync,
    {
        let check2 = check.clone();
        self.ctx.socket_addr_check(move |addr, addr_use| {
            let check = check2.clone();
            Box::pin(async move {
                match addr_use {
                    SocketAddrUse::TcpListen
                    | SocketAddrUse::TcpAccept
                    | SocketAddrUse::UdpReceive => false,
                    SocketAddrUse::TcpBind
                    | SocketAddrUse::TcpConnect
                    | SocketAddrUse::UdpBind
                    | SocketAddrUse::UdpSend => check(None, addr, addr_use).await,
                }
            })
        });
        for (id, ctx) in &mut self.dependency_capabilities.ids {
            let Some(csk) = self.id_to_csk.get(id.0) else {
                continue; // TODO: error?
            };
            let csk = csk.clone();
            let check2 = check.clone();
            ctx.socket_addr_check(move |addr, addr_use| {
                let check = check2.clone();
                let csk = csk.clone();
                Box::pin(async move {
                    match addr_use {
                        SocketAddrUse::TcpListen
                        | SocketAddrUse::TcpAccept
                        | SocketAddrUse::UdpReceive => false,
                        SocketAddrUse::TcpBind
                        | SocketAddrUse::TcpConnect
                        | SocketAddrUse::UdpBind
                        | SocketAddrUse::UdpSend => check(Some(csk), addr, addr_use).await,
                    }
                })
            });
        }
    }
}

pub struct InstanceState {
    ctx: WasiCtx,
    socket_permit_state: Option<Arc<SocketPermitState>>,
    dependency_ctx: std::collections::HashMap<wasmtime_wasi::NamedId, WasiCtx>,
}

impl InstanceState {
    pub fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.ctx
    }
}

#[repr(transparent)]
struct WasiStoreDataWrapper<T>
where
    T: FactorField<Factor = WasiFactor>,
{
    data: T::State,
}

impl<T> WasiStoreDataWrapper<T>
where
    T: FactorField<Factor = WasiFactor>,
{
    fn from_mut(data: &mut T::State) -> &mut Self {
        // SAFETY: TODO (aka repr(transparent), one field, etc)
        unsafe { &mut *(data as *mut T::State as *mut Self) }
    }
}

impl<T> wasmtime_wasi::WasiNamedView for WasiStoreDataWrapper<T>
where
    T: FactorField<Factor = WasiFactor>,
{
    fn ctx(&mut self, id: wasmtime_wasi::NamedId) -> WasiCtxView<'_> {
        let (st, table) = T::get(&mut self.data);
        WasiCtxView {
            ctx: st.dependency_ctx.get_mut(&id).unwrap(),
            table,
        }
    }
}
