use super::{convert, wasi_2023_10_18::convert_result};
use crate::sockets::{SpinSockets, SpinSocketsView};
use futures::{
    Stream as _,
    channel::{mpsc, oneshot},
};
use pin_project_lite::pin_project;
use spin_factors::anyhow;
use std::pin::Pin;
use std::task::{Context, Poll};
use wasmtime::component::{
    Access, Accessor, Destination, FutureConsumer, FutureProducer, FutureReader, HasData, Lift,
    Linker, Lower, Resource, Source, StreamConsumer, StreamProducer, StreamReader, StreamResult,
};
use wasmtime::error::Context as _;
use wasmtime::{AsContextMut, StoreContextMut};
use wasmtime_wasi::cli::{WasiCli, WasiCliCtxView};
use wasmtime_wasi::clocks::{WasiClocks, WasiClocksCtxView};
use wasmtime_wasi::filesystem::{WasiFilesystem, WasiFilesystemCtxView};
use wasmtime_wasi::p3::bindings as latest;
use wasmtime_wasi::random::{WasiRandom, WasiRandomCtx};
use wasmtime_wasi::sockets::{WasiSockets, WasiSocketsCtxView};

mod bindings {
    use super::latest;

    wasmtime::component::bindgen!({
        path: "../../wit",
        world: "wasi:cli/command@0.3.0-rc-2026-03-15",
        imports: {
            "wasi:cli/stdin": store | trappable,
            "wasi:cli/stdout": store | trappable,
            "wasi:cli/stderr": store | trappable,
            "wasi:filesystem/types.[method]descriptor.read-via-stream": store | trappable,
            "wasi:filesystem/types.[method]descriptor.write-via-stream": store | trappable,
            "wasi:filesystem/types.[method]descriptor.append-via-stream": store | trappable,
            "wasi:filesystem/types.[method]descriptor.read-directory": store | trappable,
            "wasi:sockets/types.[method]tcp-socket.bind": async | trappable,
            "wasi:sockets/types.[method]tcp-socket.listen": async | store | trappable,
            "wasi:sockets/types.[method]tcp-socket.send": store | trappable,
            "wasi:sockets/types.[method]tcp-socket.receive": store | trappable,
            "wasi:sockets/types.[method]udp-socket.bind": async | trappable,
            "wasi:sockets/types.[method]udp-socket.connect": async | trappable,
            default: trappable,
        },
        exports: { default: async | store },
        with: {
            "wasi:cli/terminal-input.terminal-input": latest::cli::terminal_input::TerminalInput,
            "wasi:cli/terminal-output.terminal-output": latest::cli::terminal_output::TerminalOutput,
            "wasi:filesystem/types.descriptor": latest::filesystem::types::Descriptor,
            "wasi:sockets/types.tcp-socket": latest::sockets::types::TcpSocket,
            "wasi:sockets/types.udp-socket": latest::sockets::types::UdpSocket,
        },
    });
}

mod wasi {
    pub use super::bindings::wasi::{
        cli0_3_0_rc_2026_03_15 as cli, clocks0_3_0_rc_2026_03_15 as clocks,
        filesystem0_3_0_rc_2026_03_15 as filesystem, random0_3_0_rc_2026_03_15 as random,
        sockets0_3_0_rc_2026_03_15 as sockets,
    };
}

pub fn add_to_linker<T>(
    linker: &mut Linker<T>,
    random_closure: fn(&mut T) -> &mut WasiRandomCtx,
    clocks_closure: fn(&mut T) -> WasiClocksCtxView<'_>,
    cli_closure: fn(&mut T) -> WasiCliCtxView<'_>,
    filesystem_closure: fn(&mut T) -> WasiFilesystemCtxView<'_>,
    sockets_closure: fn(&mut T) -> SpinSocketsView<'_, T>,
    wasi_sockets_closure: fn(&mut T) -> WasiSocketsCtxView<'_>,
) -> anyhow::Result<()>
where
    T: Send + 'static,
{
    wasi::clocks::monotonic_clock::add_to_linker::<_, WasiClocks>(linker, clocks_closure)?;
    wasi::clocks::system_clock::add_to_linker::<_, WasiClocks>(linker, clocks_closure)?;
    wasi::filesystem::types::add_to_linker::<_, WasiFilesystem>(linker, filesystem_closure)?;
    wasi::filesystem::preopens::add_to_linker::<_, WasiFilesystem>(linker, filesystem_closure)?;
    wasi::random::random::add_to_linker::<_, WasiRandom>(linker, random_closure)?;
    wasi::random::insecure::add_to_linker::<_, WasiRandom>(linker, random_closure)?;
    wasi::random::insecure_seed::add_to_linker::<_, WasiRandom>(linker, random_closure)?;
    wasi::cli::exit::add_to_linker::<_, WasiCli>(
        linker,
        &wasi::cli::exit::LinkOptions::default(),
        cli_closure,
    )?;
    wasi::cli::environment::add_to_linker::<_, WasiCli>(linker, cli_closure)?;
    wasi::cli::stdin::add_to_linker::<_, WasiCli>(linker, cli_closure)?;
    wasi::cli::stdout::add_to_linker::<_, WasiCli>(linker, cli_closure)?;
    wasi::cli::stderr::add_to_linker::<_, WasiCli>(linker, cli_closure)?;
    wasi::cli::terminal_input::add_to_linker::<_, WasiCli>(linker, cli_closure)?;
    wasi::cli::terminal_output::add_to_linker::<_, WasiCli>(linker, cli_closure)?;
    wasi::cli::terminal_stdin::add_to_linker::<_, WasiCli>(linker, cli_closure)?;
    wasi::cli::terminal_stdout::add_to_linker::<_, WasiCli>(linker, cli_closure)?;
    wasi::cli::terminal_stderr::add_to_linker::<_, WasiCli>(linker, cli_closure)?;
    wasi::sockets::types::add_to_linker::<_, SpinSockets<T>>(linker, sockets_closure)?;
    wasi::sockets::ip_name_lookup::add_to_linker::<_, WasiSockets>(linker, wasi_sockets_closure)?;
    Ok(())
}

impl wasi::clocks::types::Host for WasiClocksCtxView<'_> {}

impl wasi::clocks::system_clock::Host for WasiClocksCtxView<'_> {
    fn now(&mut self) -> wasmtime::Result<wasi::clocks::system_clock::Instant> {
        latest::clocks::system_clock::Host::now(self).map(|v| v.into())
    }

    fn get_resolution(&mut self) -> wasmtime::Result<wasi::clocks::types::Duration> {
        latest::clocks::system_clock::Host::get_resolution(self)
    }
}

impl<T> wasi::clocks::monotonic_clock::HostWithStore<T> for WasiClocks {
    async fn wait_until(
        store: &Accessor<T, Self>,
        when: wasi::clocks::monotonic_clock::Mark,
    ) -> wasmtime::Result<()> {
        latest::clocks::monotonic_clock::HostWithStore::wait_until(store, when).await
    }

    async fn wait_for(
        store: &Accessor<T, Self>,
        duration: wasi::clocks::types::Duration,
    ) -> wasmtime::Result<()> {
        latest::clocks::monotonic_clock::HostWithStore::wait_for(store, duration).await
    }
}

impl wasi::clocks::monotonic_clock::Host for WasiClocksCtxView<'_> {
    fn now(&mut self) -> wasmtime::Result<wasi::clocks::monotonic_clock::Mark> {
        latest::clocks::monotonic_clock::Host::now(self)
    }

    fn get_resolution(&mut self) -> wasmtime::Result<wasi::clocks::types::Duration> {
        latest::clocks::monotonic_clock::Host::get_resolution(self)
    }
}

impl wasi::filesystem::types::Host for WasiFilesystemCtxView<'_> {}

impl<T> wasi::filesystem::types::HostDescriptorWithStore<T> for WasiFilesystem {
    fn read_via_stream(
        mut store: Access<'_, T, Self>,
        fd: Resource<wasi::filesystem::types::Descriptor>,
        offset: wasi::filesystem::types::Filesize,
    ) -> wasmtime::Result<(
        StreamReader<u8>,
        FutureReader<Result<(), wasi::filesystem::types::ErrorCode>>,
    )> {
        latest::filesystem::types::HostDescriptorWithStore::read_via_stream(
            reborrow(&mut store),
            fd,
            offset,
        )
        .and_then(|(stream, future)| {
            Ok((stream, future.try_map(store, |v| v.map_err(|v| v.into()))?))
        })
    }

    fn write_via_stream(
        mut store: Access<'_, T, Self>,
        fd: Resource<wasi::filesystem::types::Descriptor>,
        data: StreamReader<u8>,
        offset: wasi::filesystem::types::Filesize,
    ) -> wasmtime::Result<FutureReader<Result<(), wasi::filesystem::types::ErrorCode>>> {
        latest::filesystem::types::HostDescriptorWithStore::write_via_stream(
            reborrow(&mut store),
            fd,
            data,
            offset,
        )
        .and_then(|v| v.try_map(store, |v| v.map_err(|v| v.into())))
    }

    fn append_via_stream(
        mut store: Access<'_, T, Self>,
        fd: Resource<wasi::filesystem::types::Descriptor>,
        data: StreamReader<u8>,
    ) -> wasmtime::Result<FutureReader<Result<(), wasi::filesystem::types::ErrorCode>>> {
        latest::filesystem::types::HostDescriptorWithStore::append_via_stream(
            reborrow(&mut store),
            fd,
            data,
        )
        .and_then(|v| v.try_map(store, |v| v.map_err(|v| v.into())))
    }

    async fn advise(
        store: &Accessor<T, Self>,
        fd: Resource<wasi::filesystem::types::Descriptor>,
        offset: wasi::filesystem::types::Filesize,
        length: wasi::filesystem::types::Filesize,
        advice: wasi::filesystem::types::Advice,
    ) -> wasmtime::Result<Result<(), wasi::filesystem::types::ErrorCode>> {
        convert_result(
            latest::filesystem::types::HostDescriptorWithStore::advise(
                store,
                fd,
                offset,
                length,
                advice.into(),
            )
            .await,
        )
    }

    async fn sync_data(
        store: &Accessor<T, Self>,
        fd: Resource<wasi::filesystem::types::Descriptor>,
    ) -> wasmtime::Result<Result<(), wasi::filesystem::types::ErrorCode>> {
        convert_result(
            latest::filesystem::types::HostDescriptorWithStore::sync_data(store, fd).await,
        )
    }

    async fn get_flags(
        store: &Accessor<T, Self>,
        fd: Resource<wasi::filesystem::types::Descriptor>,
    ) -> wasmtime::Result<
        Result<wasi::filesystem::types::DescriptorFlags, wasi::filesystem::types::ErrorCode>,
    > {
        convert_result(
            latest::filesystem::types::HostDescriptorWithStore::get_flags(store, fd).await,
        )
    }

    async fn get_type(
        store: &Accessor<T, Self>,
        fd: Resource<wasi::filesystem::types::Descriptor>,
    ) -> wasmtime::Result<
        Result<wasi::filesystem::types::DescriptorType, wasi::filesystem::types::ErrorCode>,
    > {
        convert_result(
            latest::filesystem::types::HostDescriptorWithStore::get_type(store, fd).await,
        )
    }

    async fn set_size(
        store: &Accessor<T, Self>,
        fd: Resource<wasi::filesystem::types::Descriptor>,
        size: wasi::filesystem::types::Filesize,
    ) -> wasmtime::Result<Result<(), wasi::filesystem::types::ErrorCode>> {
        convert_result(
            latest::filesystem::types::HostDescriptorWithStore::set_size(store, fd, size).await,
        )
    }

    async fn set_times(
        store: &Accessor<T, Self>,
        fd: Resource<wasi::filesystem::types::Descriptor>,
        data_access_timestamp: wasi::filesystem::types::NewTimestamp,
        data_modification_timestamp: wasi::filesystem::types::NewTimestamp,
    ) -> wasmtime::Result<Result<(), wasi::filesystem::types::ErrorCode>> {
        convert_result(
            latest::filesystem::types::HostDescriptorWithStore::set_times(
                store,
                fd,
                data_access_timestamp.into(),
                data_modification_timestamp.into(),
            )
            .await,
        )
    }

    fn read_directory(
        mut store: Access<'_, T, Self>,
        fd: Resource<wasi::filesystem::types::Descriptor>,
    ) -> wasmtime::Result<(
        StreamReader<wasi::filesystem::types::DirectoryEntry>,
        FutureReader<Result<(), wasi::filesystem::types::ErrorCode>>,
    )> {
        latest::filesystem::types::HostDescriptorWithStore::read_directory(reborrow(&mut store), fd)
            .and_then(|(stream, future)| {
                Ok((
                    stream.try_map(reborrow(&mut store), |v| v.into())?,
                    future.try_map(store, |v| v.map_err(|v| v.into()))?,
                ))
            })
    }

    async fn sync(
        store: &Accessor<T, Self>,
        fd: Resource<wasi::filesystem::types::Descriptor>,
    ) -> wasmtime::Result<Result<(), wasi::filesystem::types::ErrorCode>> {
        convert_result(latest::filesystem::types::HostDescriptorWithStore::sync(store, fd).await)
    }

    async fn create_directory_at(
        store: &Accessor<T, Self>,
        fd: Resource<wasi::filesystem::types::Descriptor>,
        path: String,
    ) -> wasmtime::Result<Result<(), wasi::filesystem::types::ErrorCode>> {
        convert_result(
            latest::filesystem::types::HostDescriptorWithStore::create_directory_at(
                store, fd, path,
            )
            .await,
        )
    }

    async fn stat(
        store: &Accessor<T, Self>,
        fd: Resource<wasi::filesystem::types::Descriptor>,
    ) -> wasmtime::Result<
        Result<wasi::filesystem::types::DescriptorStat, wasi::filesystem::types::ErrorCode>,
    > {
        convert_result(latest::filesystem::types::HostDescriptorWithStore::stat(store, fd).await)
    }

    async fn stat_at(
        store: &Accessor<T, Self>,
        fd: Resource<wasi::filesystem::types::Descriptor>,
        path_flags: wasi::filesystem::types::PathFlags,
        path: String,
    ) -> wasmtime::Result<
        Result<wasi::filesystem::types::DescriptorStat, wasi::filesystem::types::ErrorCode>,
    > {
        convert_result(
            latest::filesystem::types::HostDescriptorWithStore::stat_at(
                store,
                fd,
                path_flags.into(),
                path,
            )
            .await,
        )
    }

    async fn set_times_at(
        store: &Accessor<T, Self>,
        fd: Resource<wasi::filesystem::types::Descriptor>,
        path_flags: wasi::filesystem::types::PathFlags,
        path: String,
        data_access_timestamp: wasi::filesystem::types::NewTimestamp,
        data_modification_timestamp: wasi::filesystem::types::NewTimestamp,
    ) -> wasmtime::Result<Result<(), wasi::filesystem::types::ErrorCode>> {
        convert_result(
            latest::filesystem::types::HostDescriptorWithStore::set_times_at(
                store,
                fd,
                path_flags.into(),
                path,
                data_access_timestamp.into(),
                data_modification_timestamp.into(),
            )
            .await,
        )
    }

    async fn link_at(
        store: &Accessor<T, Self>,
        fd: Resource<wasi::filesystem::types::Descriptor>,
        old_path_flags: wasi::filesystem::types::PathFlags,
        old_path: String,
        new_fd: Resource<wasi::filesystem::types::Descriptor>,
        new_path: String,
    ) -> wasmtime::Result<Result<(), wasi::filesystem::types::ErrorCode>> {
        convert_result(
            latest::filesystem::types::HostDescriptorWithStore::link_at(
                store,
                fd,
                old_path_flags.into(),
                old_path,
                new_fd,
                new_path,
            )
            .await,
        )
    }

    async fn open_at(
        store: &Accessor<T, Self>,
        fd: Resource<wasi::filesystem::types::Descriptor>,
        path_flags: wasi::filesystem::types::PathFlags,
        path: String,
        open_flags: wasi::filesystem::types::OpenFlags,
        flags: wasi::filesystem::types::DescriptorFlags,
    ) -> wasmtime::Result<
        Result<Resource<wasi::filesystem::types::Descriptor>, wasi::filesystem::types::ErrorCode>,
    > {
        convert_result(
            latest::filesystem::types::HostDescriptorWithStore::open_at(
                store,
                fd,
                path_flags.into(),
                path,
                open_flags.into(),
                flags.into(),
            )
            .await,
        )
    }

    async fn readlink_at(
        store: &Accessor<T, Self>,
        fd: Resource<wasi::filesystem::types::Descriptor>,
        path: String,
    ) -> wasmtime::Result<Result<String, wasi::filesystem::types::ErrorCode>> {
        convert_result(
            latest::filesystem::types::HostDescriptorWithStore::readlink_at(store, fd, path).await,
        )
    }

    async fn remove_directory_at(
        store: &Accessor<T, Self>,
        fd: Resource<wasi::filesystem::types::Descriptor>,
        path: String,
    ) -> wasmtime::Result<Result<(), wasi::filesystem::types::ErrorCode>> {
        convert_result(
            latest::filesystem::types::HostDescriptorWithStore::remove_directory_at(
                store, fd, path,
            )
            .await,
        )
    }

    async fn rename_at(
        store: &Accessor<T, Self>,
        fd: Resource<wasi::filesystem::types::Descriptor>,
        old_path: String,
        new_fd: Resource<wasi::filesystem::types::Descriptor>,
        new_path: String,
    ) -> wasmtime::Result<Result<(), wasi::filesystem::types::ErrorCode>> {
        convert_result(
            latest::filesystem::types::HostDescriptorWithStore::rename_at(
                store, fd, old_path, new_fd, new_path,
            )
            .await,
        )
    }

    async fn symlink_at(
        store: &Accessor<T, Self>,
        fd: Resource<wasi::filesystem::types::Descriptor>,
        old_path: String,
        new_path: String,
    ) -> wasmtime::Result<Result<(), wasi::filesystem::types::ErrorCode>> {
        convert_result(
            latest::filesystem::types::HostDescriptorWithStore::symlink_at(
                store, fd, old_path, new_path,
            )
            .await,
        )
    }

    async fn unlink_file_at(
        store: &Accessor<T, Self>,
        fd: Resource<wasi::filesystem::types::Descriptor>,
        path: String,
    ) -> wasmtime::Result<Result<(), wasi::filesystem::types::ErrorCode>> {
        convert_result(
            latest::filesystem::types::HostDescriptorWithStore::unlink_file_at(store, fd, path)
                .await,
        )
    }

    async fn is_same_object(
        store: &Accessor<T, Self>,
        fd: Resource<wasi::filesystem::types::Descriptor>,
        other: Resource<wasi::filesystem::types::Descriptor>,
    ) -> wasmtime::Result<bool> {
        latest::filesystem::types::HostDescriptorWithStore::is_same_object(store, fd, other).await
    }

    async fn metadata_hash(
        store: &Accessor<T, Self>,
        fd: Resource<wasi::filesystem::types::Descriptor>,
    ) -> wasmtime::Result<
        Result<wasi::filesystem::types::MetadataHashValue, wasi::filesystem::types::ErrorCode>,
    > {
        convert_result(
            latest::filesystem::types::HostDescriptorWithStore::metadata_hash(store, fd).await,
        )
    }

    async fn metadata_hash_at(
        store: &Accessor<T, Self>,
        fd: Resource<wasi::filesystem::types::Descriptor>,
        path_flags: wasi::filesystem::types::PathFlags,
        path: String,
    ) -> wasmtime::Result<
        Result<wasi::filesystem::types::MetadataHashValue, wasi::filesystem::types::ErrorCode>,
    > {
        convert_result(
            latest::filesystem::types::HostDescriptorWithStore::metadata_hash_at(
                store,
                fd,
                path_flags.into(),
                path,
            )
            .await,
        )
    }
}

impl wasi::filesystem::types::HostDescriptor for WasiFilesystemCtxView<'_> {
    fn drop(&mut self, fd: Resource<wasi::filesystem::types::Descriptor>) -> wasmtime::Result<()> {
        latest::filesystem::types::HostDescriptor::drop(self, fd)
    }
}

impl wasi::filesystem::preopens::Host for WasiFilesystemCtxView<'_> {
    fn get_directories(
        &mut self,
    ) -> wasmtime::Result<Vec<(Resource<wasi::filesystem::types::Descriptor>, String)>> {
        latest::filesystem::preopens::Host::get_directories(self)
    }
}

impl wasi::random::random::Host for WasiRandomCtx {
    fn get_random_bytes(&mut self, len: u64) -> wasmtime::Result<Vec<u8>> {
        latest::random::random::Host::get_random_bytes(self, len)
    }

    fn get_random_u64(&mut self) -> wasmtime::Result<u64> {
        latest::random::random::Host::get_random_u64(self)
    }
}

impl wasi::random::insecure::Host for WasiRandomCtx {
    fn get_insecure_random_bytes(&mut self, len: u64) -> wasmtime::Result<Vec<u8>> {
        latest::random::insecure::Host::get_insecure_random_bytes(self, len)
    }

    fn get_insecure_random_u64(&mut self) -> wasmtime::Result<u64> {
        latest::random::insecure::Host::get_insecure_random_u64(self)
    }
}

impl wasi::random::insecure_seed::Host for WasiRandomCtx {
    fn get_insecure_seed(&mut self) -> wasmtime::Result<(u64, u64)> {
        latest::random::insecure_seed::Host::get_insecure_seed(self)
    }
}

impl wasi::cli::terminal_input::Host for WasiCliCtxView<'_> {}
impl wasi::cli::terminal_output::Host for WasiCliCtxView<'_> {}

impl wasi::cli::terminal_input::HostTerminalInput for WasiCliCtxView<'_> {
    fn drop(
        &mut self,
        rep: Resource<wasi::cli::terminal_input::TerminalInput>,
    ) -> wasmtime::Result<()> {
        latest::cli::terminal_input::HostTerminalInput::drop(self, rep)
    }
}

impl wasi::cli::terminal_output::HostTerminalOutput for WasiCliCtxView<'_> {
    fn drop(
        &mut self,
        rep: Resource<wasi::cli::terminal_output::TerminalOutput>,
    ) -> wasmtime::Result<()> {
        latest::cli::terminal_output::HostTerminalOutput::drop(self, rep)
    }
}

impl wasi::cli::terminal_stdin::Host for WasiCliCtxView<'_> {
    fn get_terminal_stdin(
        &mut self,
    ) -> wasmtime::Result<Option<Resource<wasi::cli::terminal_input::TerminalInput>>> {
        latest::cli::terminal_stdin::Host::get_terminal_stdin(self)
    }
}

impl wasi::cli::terminal_stdout::Host for WasiCliCtxView<'_> {
    fn get_terminal_stdout(
        &mut self,
    ) -> wasmtime::Result<Option<Resource<wasi::cli::terminal_output::TerminalOutput>>> {
        latest::cli::terminal_stdout::Host::get_terminal_stdout(self)
    }
}

impl wasi::cli::terminal_stderr::Host for WasiCliCtxView<'_> {
    fn get_terminal_stderr(
        &mut self,
    ) -> wasmtime::Result<Option<Resource<wasi::cli::terminal_output::TerminalOutput>>> {
        latest::cli::terminal_stderr::Host::get_terminal_stderr(self)
    }
}

impl<T> wasi::cli::stdin::HostWithStore<T> for WasiCli {
    fn read_via_stream(
        mut store: Access<T, Self>,
    ) -> wasmtime::Result<(
        StreamReader<u8>,
        FutureReader<Result<(), wasi::cli::types::ErrorCode>>,
    )> {
        latest::cli::stdin::HostWithStore::read_via_stream(reborrow(&mut store)).and_then(
            |(stream, future)| Ok((stream, future.try_map(store, |v| v.map_err(|v| v.into()))?)),
        )
    }
}

impl wasi::cli::stdin::Host for WasiCliCtxView<'_> {}

impl<T> wasi::cli::stdout::HostWithStore<T> for WasiCli {
    fn write_via_stream(
        mut store: Access<'_, T, Self>,
        data: StreamReader<u8>,
    ) -> wasmtime::Result<FutureReader<Result<(), wasi::cli::types::ErrorCode>>> {
        latest::cli::stdout::HostWithStore::write_via_stream(reborrow(&mut store), data)
            .and_then(|v| v.try_map(store, |v| v.map_err(|v| v.into())))
    }
}

impl wasi::cli::stdout::Host for WasiCliCtxView<'_> {}

impl<T> wasi::cli::stderr::HostWithStore<T> for WasiCli {
    fn write_via_stream(
        mut store: Access<'_, T, Self>,
        data: StreamReader<u8>,
    ) -> wasmtime::Result<FutureReader<Result<(), wasi::cli::types::ErrorCode>>> {
        latest::cli::stderr::HostWithStore::write_via_stream(reborrow(&mut store), data)
            .and_then(|v| v.try_map(store, |v| v.map_err(|v| v.into())))
    }
}

impl wasi::cli::stderr::Host for WasiCliCtxView<'_> {}

impl wasi::cli::environment::Host for WasiCliCtxView<'_> {
    fn get_environment(&mut self) -> wasmtime::Result<Vec<(String, String)>> {
        latest::cli::environment::Host::get_environment(self)
    }

    fn get_arguments(&mut self) -> wasmtime::Result<Vec<String>> {
        latest::cli::environment::Host::get_arguments(self)
    }

    fn get_initial_cwd(&mut self) -> wasmtime::Result<Option<String>> {
        latest::cli::environment::Host::get_initial_cwd(self)
    }
}

impl wasi::cli::exit::Host for WasiCliCtxView<'_> {
    fn exit(&mut self, status: Result<(), ()>) -> wasmtime::Result<()> {
        latest::cli::exit::Host::exit(self, status)
    }

    fn exit_with_code(&mut self, status_code: u8) -> wasmtime::Result<()> {
        latest::cli::exit::Host::exit_with_code(self, status_code)
    }
}

impl<T> wasi::sockets::types::Host for SpinSocketsView<'_, T> {}

impl<T: 'static> wasi::sockets::types::HostUdpSocketWithStore<T> for SpinSockets<T> {
    async fn send(
        store: &Accessor<T, Self>,
        socket: Resource<wasi::sockets::types::UdpSocket>,
        data: Vec<u8>,
        remote_address: Option<wasi::sockets::types::IpSocketAddress>,
    ) -> wasmtime::Result<Result<(), wasi::sockets::types::ErrorCode>> {
        convert_result(
            latest::sockets::types::HostUdpSocketWithStore::send(
                store,
                socket,
                data,
                remote_address.map(|v| v.into()),
            )
            .await,
        )
    }

    async fn receive(
        store: &Accessor<T, Self>,
        socket: Resource<wasi::sockets::types::UdpSocket>,
    ) -> wasmtime::Result<
        Result<(Vec<u8>, wasi::sockets::types::IpSocketAddress), wasi::sockets::types::ErrorCode>,
    > {
        convert_result(
            latest::sockets::types::HostUdpSocketWithStore::receive(store, socket)
                .await
                .map(|(a, b)| (a, b.into())),
        )
    }
}

impl<T> wasi::sockets::types::HostUdpSocket for SpinSocketsView<'_, T> {
    async fn bind(
        &mut self,
        socket: Resource<wasi::sockets::types::UdpSocket>,
        local_address: wasi::sockets::types::IpSocketAddress,
    ) -> wasmtime::Result<Result<(), wasi::sockets::types::ErrorCode>> {
        convert_result(
            latest::sockets::types::HostUdpSocket::bind(self, socket, local_address.into()).await,
        )
    }

    async fn connect(
        &mut self,
        socket: Resource<wasi::sockets::types::UdpSocket>,
        remote_address: wasi::sockets::types::IpSocketAddress,
    ) -> wasmtime::Result<Result<(), wasi::sockets::types::ErrorCode>> {
        convert_result(
            latest::sockets::types::HostUdpSocket::connect(self, socket, remote_address.into())
                .await,
        )
    }

    fn create(
        &mut self,
        address_family: wasi::sockets::types::IpAddressFamily,
    ) -> wasmtime::Result<
        Result<Resource<wasi::sockets::types::UdpSocket>, wasi::sockets::types::ErrorCode>,
    > {
        convert_result(latest::sockets::types::HostUdpSocket::create(
            self,
            address_family.into(),
        ))
    }

    fn disconnect(
        &mut self,
        socket: Resource<wasi::sockets::types::UdpSocket>,
    ) -> wasmtime::Result<Result<(), wasi::sockets::types::ErrorCode>> {
        convert_result(latest::sockets::types::HostUdpSocket::disconnect(
            self, socket,
        ))
    }

    fn get_local_address(
        &mut self,
        socket: Resource<wasi::sockets::types::UdpSocket>,
    ) -> wasmtime::Result<
        Result<wasi::sockets::types::IpSocketAddress, wasi::sockets::types::ErrorCode>,
    > {
        convert_result(latest::sockets::types::HostUdpSocket::get_local_address(
            self, socket,
        ))
    }

    fn get_remote_address(
        &mut self,
        socket: Resource<wasi::sockets::types::UdpSocket>,
    ) -> wasmtime::Result<
        Result<wasi::sockets::types::IpSocketAddress, wasi::sockets::types::ErrorCode>,
    > {
        convert_result(latest::sockets::types::HostUdpSocket::get_remote_address(
            self, socket,
        ))
    }

    fn get_address_family(
        &mut self,
        socket: Resource<wasi::sockets::types::UdpSocket>,
    ) -> wasmtime::Result<wasi::sockets::types::IpAddressFamily> {
        latest::sockets::types::HostUdpSocket::get_address_family(self, socket).map(|v| v.into())
    }

    fn get_unicast_hop_limit(
        &mut self,
        socket: Resource<wasi::sockets::types::UdpSocket>,
    ) -> wasmtime::Result<Result<u8, wasi::sockets::types::ErrorCode>> {
        convert_result(latest::sockets::types::HostUdpSocket::get_unicast_hop_limit(self, socket))
    }

    fn set_unicast_hop_limit(
        &mut self,
        socket: Resource<wasi::sockets::types::UdpSocket>,
        value: u8,
    ) -> wasmtime::Result<Result<(), wasi::sockets::types::ErrorCode>> {
        convert_result(
            latest::sockets::types::HostUdpSocket::set_unicast_hop_limit(self, socket, value),
        )
    }

    fn get_receive_buffer_size(
        &mut self,
        socket: Resource<wasi::sockets::types::UdpSocket>,
    ) -> wasmtime::Result<Result<u64, wasi::sockets::types::ErrorCode>> {
        convert_result(latest::sockets::types::HostUdpSocket::get_receive_buffer_size(self, socket))
    }

    fn set_receive_buffer_size(
        &mut self,
        socket: Resource<wasi::sockets::types::UdpSocket>,
        value: u64,
    ) -> wasmtime::Result<Result<(), wasi::sockets::types::ErrorCode>> {
        convert_result(
            latest::sockets::types::HostUdpSocket::set_receive_buffer_size(self, socket, value),
        )
    }

    fn get_send_buffer_size(
        &mut self,
        socket: Resource<wasi::sockets::types::UdpSocket>,
    ) -> wasmtime::Result<Result<u64, wasi::sockets::types::ErrorCode>> {
        convert_result(latest::sockets::types::HostUdpSocket::get_send_buffer_size(
            self, socket,
        ))
    }

    fn set_send_buffer_size(
        &mut self,
        socket: Resource<wasi::sockets::types::UdpSocket>,
        value: u64,
    ) -> wasmtime::Result<Result<(), wasi::sockets::types::ErrorCode>> {
        convert_result(latest::sockets::types::HostUdpSocket::set_send_buffer_size(
            self, socket, value,
        ))
    }

    fn drop(&mut self, sock: Resource<wasi::sockets::types::UdpSocket>) -> wasmtime::Result<()> {
        latest::sockets::types::HostUdpSocket::drop(self, sock)
    }
}

impl<T: Send + 'static> wasi::sockets::types::HostTcpSocketWithStore<T> for SpinSockets<T> {
    async fn connect(
        store: &Accessor<T, Self>,
        socket: Resource<wasi::sockets::types::TcpSocket>,
        remote_address: wasi::sockets::types::IpSocketAddress,
    ) -> wasmtime::Result<Result<(), wasi::sockets::types::ErrorCode>> {
        convert_result(
            latest::sockets::types::HostTcpSocketWithStore::connect(
                store,
                socket,
                remote_address.into(),
            )
            .await,
        )
    }

    async fn listen(
        store: Access<'_, T, Self>,
        socket: Resource<wasi::sockets::types::TcpSocket>,
    ) -> wasmtime::Result<
        Result<
            StreamReader<Resource<wasi::sockets::types::TcpSocket>>,
            wasi::sockets::types::ErrorCode,
        >,
    > {
        convert_result(latest::sockets::types::HostTcpSocketWithStore::listen(store, socket).await)
    }

    fn send(
        mut store: Access<'_, T, Self>,
        socket: Resource<wasi::sockets::types::TcpSocket>,
        data: StreamReader<u8>,
    ) -> wasmtime::Result<FutureReader<Result<(), wasi::sockets::types::ErrorCode>>> {
        latest::sockets::types::HostTcpSocketWithStore::send(reborrow(&mut store), socket, data)
            .and_then(|v| v.try_map(store, |v| v.map_err(|v| v.into())))
    }

    fn receive(
        mut store: Access<'_, T, Self>,
        socket: Resource<wasi::sockets::types::TcpSocket>,
    ) -> wasmtime::Result<(
        StreamReader<u8>,
        FutureReader<Result<(), wasi::sockets::types::ErrorCode>>,
    )> {
        latest::sockets::types::HostTcpSocketWithStore::receive(reborrow(&mut store), socket)
            .and_then(|(stream, future)| {
                Ok((stream, future.try_map(store, |v| v.map_err(|v| v.into()))?))
            })
    }
}

impl<T> wasi::sockets::types::HostTcpSocket for SpinSocketsView<'_, T> {
    async fn bind(
        &mut self,
        socket: Resource<wasi::sockets::types::TcpSocket>,
        local_address: wasi::sockets::types::IpSocketAddress,
    ) -> wasmtime::Result<Result<(), wasi::sockets::types::ErrorCode>> {
        convert_result(
            latest::sockets::types::HostTcpSocket::bind(self, socket, local_address.into()).await,
        )
    }

    fn create(
        &mut self,
        address_family: wasi::sockets::types::IpAddressFamily,
    ) -> wasmtime::Result<
        Result<Resource<wasi::sockets::types::TcpSocket>, wasi::sockets::types::ErrorCode>,
    > {
        convert_result(latest::sockets::types::HostTcpSocket::create(
            self,
            address_family.into(),
        ))
    }

    fn get_local_address(
        &mut self,
        socket: Resource<wasi::sockets::types::TcpSocket>,
    ) -> wasmtime::Result<
        Result<wasi::sockets::types::IpSocketAddress, wasi::sockets::types::ErrorCode>,
    > {
        convert_result(latest::sockets::types::HostTcpSocket::get_local_address(
            self, socket,
        ))
    }

    fn get_remote_address(
        &mut self,
        socket: Resource<wasi::sockets::types::TcpSocket>,
    ) -> wasmtime::Result<
        Result<wasi::sockets::types::IpSocketAddress, wasi::sockets::types::ErrorCode>,
    > {
        convert_result(latest::sockets::types::HostTcpSocket::get_remote_address(
            self, socket,
        ))
    }

    fn get_is_listening(
        &mut self,
        socket: Resource<wasi::sockets::types::TcpSocket>,
    ) -> wasmtime::Result<bool> {
        latest::sockets::types::HostTcpSocket::get_is_listening(self, socket)
    }

    fn get_address_family(
        &mut self,
        socket: Resource<wasi::sockets::types::TcpSocket>,
    ) -> wasmtime::Result<wasi::sockets::types::IpAddressFamily> {
        latest::sockets::types::HostTcpSocket::get_address_family(self, socket).map(|v| v.into())
    }

    fn set_listen_backlog_size(
        &mut self,
        socket: Resource<wasi::sockets::types::TcpSocket>,
        value: u64,
    ) -> wasmtime::Result<Result<(), wasi::sockets::types::ErrorCode>> {
        convert_result(
            latest::sockets::types::HostTcpSocket::set_listen_backlog_size(self, socket, value),
        )
    }

    fn get_keep_alive_enabled(
        &mut self,
        socket: Resource<wasi::sockets::types::TcpSocket>,
    ) -> wasmtime::Result<Result<bool, wasi::sockets::types::ErrorCode>> {
        convert_result(latest::sockets::types::HostTcpSocket::get_keep_alive_enabled(self, socket))
    }

    fn set_keep_alive_enabled(
        &mut self,
        socket: Resource<wasi::sockets::types::TcpSocket>,
        value: bool,
    ) -> wasmtime::Result<Result<(), wasi::sockets::types::ErrorCode>> {
        convert_result(
            latest::sockets::types::HostTcpSocket::set_keep_alive_enabled(self, socket, value),
        )
    }

    fn get_keep_alive_idle_time(
        &mut self,
        socket: Resource<wasi::sockets::types::TcpSocket>,
    ) -> wasmtime::Result<Result<wasi::sockets::types::Duration, wasi::sockets::types::ErrorCode>>
    {
        convert_result(
            latest::sockets::types::HostTcpSocket::get_keep_alive_idle_time(self, socket),
        )
    }

    fn set_keep_alive_idle_time(
        &mut self,
        socket: Resource<wasi::sockets::types::TcpSocket>,
        value: wasi::sockets::types::Duration,
    ) -> wasmtime::Result<Result<(), wasi::sockets::types::ErrorCode>> {
        convert_result(
            latest::sockets::types::HostTcpSocket::set_keep_alive_idle_time(self, socket, value),
        )
    }

    fn get_keep_alive_interval(
        &mut self,
        socket: Resource<wasi::sockets::types::TcpSocket>,
    ) -> wasmtime::Result<Result<wasi::sockets::types::Duration, wasi::sockets::types::ErrorCode>>
    {
        convert_result(latest::sockets::types::HostTcpSocket::get_keep_alive_interval(self, socket))
    }

    fn set_keep_alive_interval(
        &mut self,
        socket: Resource<wasi::sockets::types::TcpSocket>,
        value: wasi::sockets::types::Duration,
    ) -> wasmtime::Result<Result<(), wasi::sockets::types::ErrorCode>> {
        convert_result(
            latest::sockets::types::HostTcpSocket::set_keep_alive_interval(self, socket, value),
        )
    }

    fn get_keep_alive_count(
        &mut self,
        socket: Resource<wasi::sockets::types::TcpSocket>,
    ) -> wasmtime::Result<Result<u32, wasi::sockets::types::ErrorCode>> {
        convert_result(latest::sockets::types::HostTcpSocket::get_keep_alive_count(
            self, socket,
        ))
    }

    fn set_keep_alive_count(
        &mut self,
        socket: Resource<wasi::sockets::types::TcpSocket>,
        value: u32,
    ) -> wasmtime::Result<Result<(), wasi::sockets::types::ErrorCode>> {
        convert_result(latest::sockets::types::HostTcpSocket::set_keep_alive_count(
            self, socket, value,
        ))
    }

    fn get_hop_limit(
        &mut self,
        socket: Resource<wasi::sockets::types::TcpSocket>,
    ) -> wasmtime::Result<Result<u8, wasi::sockets::types::ErrorCode>> {
        convert_result(latest::sockets::types::HostTcpSocket::get_hop_limit(
            self, socket,
        ))
    }

    fn set_hop_limit(
        &mut self,
        socket: Resource<wasi::sockets::types::TcpSocket>,
        value: u8,
    ) -> wasmtime::Result<Result<(), wasi::sockets::types::ErrorCode>> {
        convert_result(latest::sockets::types::HostTcpSocket::set_hop_limit(
            self, socket, value,
        ))
    }

    fn get_receive_buffer_size(
        &mut self,
        socket: Resource<wasi::sockets::types::TcpSocket>,
    ) -> wasmtime::Result<Result<u64, wasi::sockets::types::ErrorCode>> {
        convert_result(latest::sockets::types::HostTcpSocket::get_receive_buffer_size(self, socket))
    }

    fn set_receive_buffer_size(
        &mut self,
        socket: Resource<wasi::sockets::types::TcpSocket>,
        value: u64,
    ) -> wasmtime::Result<Result<(), wasi::sockets::types::ErrorCode>> {
        convert_result(
            latest::sockets::types::HostTcpSocket::set_receive_buffer_size(self, socket, value),
        )
    }

    fn get_send_buffer_size(
        &mut self,
        socket: Resource<wasi::sockets::types::TcpSocket>,
    ) -> wasmtime::Result<Result<u64, wasi::sockets::types::ErrorCode>> {
        convert_result(latest::sockets::types::HostTcpSocket::get_send_buffer_size(
            self, socket,
        ))
    }

    fn set_send_buffer_size(
        &mut self,
        socket: Resource<wasi::sockets::types::TcpSocket>,
        value: u64,
    ) -> wasmtime::Result<Result<(), wasi::sockets::types::ErrorCode>> {
        convert_result(latest::sockets::types::HostTcpSocket::set_send_buffer_size(
            self, socket, value,
        ))
    }

    fn drop(&mut self, sock: Resource<wasi::sockets::types::TcpSocket>) -> wasmtime::Result<()> {
        latest::sockets::types::HostTcpSocket::drop(self, sock)
    }
}

impl<T> wasi::sockets::ip_name_lookup::HostWithStore<T> for WasiSockets {
    async fn resolve_addresses(
        store: &Accessor<T, Self>,
        name: String,
    ) -> wasmtime::Result<
        Result<Vec<wasi::sockets::types::IpAddress>, wasi::sockets::ip_name_lookup::ErrorCode>,
    > {
        latest::sockets::ip_name_lookup::HostWithStore::resolve_addresses(store, name)
            .await
            .map(|v| {
                v.map(|v| v.into_iter().map(|v| v.into()).collect())
                    .map_err(|e| e.into())
            })
    }
}

impl wasi::sockets::ip_name_lookup::Host for WasiSocketsCtxView<'_> {}

convert! {
    struct latest::clocks::system_clock::Instant [<=>] wasi::clocks::system_clock::Instant {
        seconds,
        nanoseconds,
    }

    enum latest::cli::types::ErrorCode => wasi::cli::types::ErrorCode {
        Io,
        IllegalByteSequence,
        Pipe,
    }

    enum latest::filesystem::types::ErrorCode => wasi::filesystem::types::ErrorCode {
        Access,
        Already,
        BadDescriptor,
        Busy,
        Deadlock,
        Quota,
        Exist,
        FileTooLarge,
        IllegalByteSequence,
        InProgress,
        Interrupted,
        Invalid,
        Io,
        IsDirectory,
        Loop,
        TooManyLinks,
        MessageSize,
        NameTooLong,
        NoDevice,
        NoEntry,
        NoLock,
        InsufficientMemory,
        InsufficientSpace,
        NotDirectory,
        NotEmpty,
        NotRecoverable,
        Unsupported,
        NoTty,
        NoSuchDevice,
        Overflow,
        NotPermitted,
        Pipe,
        ReadOnly,
        InvalidSeek,
        TextFileBusy,
        CrossDevice,
        Other(v),
    }

    enum wasi::filesystem::types::Advice => latest::filesystem::types::Advice {
        Normal,
        Sequential,
        Random,
        WillNeed,
        DontNeed,
        NoReuse,
    }

    flags wasi::filesystem::types::DescriptorFlags [<=>] latest::filesystem::types::DescriptorFlags {
        READ,
        WRITE,
        FILE_INTEGRITY_SYNC,
        DATA_INTEGRITY_SYNC,
        REQUESTED_WRITE_SYNC,
        MUTATE_DIRECTORY,
    }

    enum wasi::filesystem::types::DescriptorType [<=>] latest::filesystem::types::DescriptorType {
        BlockDevice,
        CharacterDevice,
        Directory,
        Fifo,
        SymbolicLink,
        RegularFile,
        Socket,
        Other(v),
    }

    enum wasi::filesystem::types::NewTimestamp => latest::filesystem::types::NewTimestamp {
        NoChange,
        Now,
        Timestamp(e),
    }

    flags wasi::filesystem::types::PathFlags => latest::filesystem::types::PathFlags {
        SYMLINK_FOLLOW,
    }

    flags wasi::filesystem::types::OpenFlags => latest::filesystem::types::OpenFlags {
        CREATE,
        DIRECTORY,
        EXCLUSIVE,
        TRUNCATE,
    }

    struct latest::filesystem::types::MetadataHashValue => wasi::filesystem::types::MetadataHashValue {
        lower,
        upper,
    }

    struct latest::filesystem::types::DirectoryEntry => wasi::filesystem::types::DirectoryEntry {
        type_,
        name,
    }

    enum latest::sockets::types::ErrorCode => wasi::sockets::types::ErrorCode {
        AccessDenied,
        NotSupported,
        InvalidArgument,
        OutOfMemory,
        Timeout,
        InvalidState,
        AddressNotBindable,
        AddressInUse,
        RemoteUnreachable,
        ConnectionRefused,
        ConnectionBroken,
        ConnectionReset,
        ConnectionAborted,
        DatagramTooLarge,
        Other(v),
    }

    enum latest::sockets::types::IpAddress [<=>] wasi::sockets::types::IpAddress {
        Ipv4(e),
        Ipv6(e),
    }

    enum latest::sockets::types::IpSocketAddress [<=>] wasi::sockets::types::IpSocketAddress {
        Ipv4(e),
        Ipv6(e),
    }

    struct latest::sockets::types::Ipv4SocketAddress [<=>] wasi::sockets::types::Ipv4SocketAddress {
        port,
        address,
    }

    struct latest::sockets::types::Ipv6SocketAddress [<=>] wasi::sockets::types::Ipv6SocketAddress {
        port,
        flow_info,
        scope_id,
        address,
    }

    enum latest::sockets::types::IpAddressFamily [<=>] wasi::sockets::types::IpAddressFamily {
        Ipv4,
        Ipv6,
    }

    enum latest::sockets::ip_name_lookup::ErrorCode => wasi::sockets::ip_name_lookup::ErrorCode {
        AccessDenied,
        InvalidArgument,
        NameUnresolvable,
        TemporaryResolverFailure,
        PermanentResolverFailure,
        Other(v),
    }
}

impl From<latest::filesystem::types::DescriptorStat> for wasi::filesystem::types::DescriptorStat {
    fn from(
        e: latest::filesystem::types::DescriptorStat,
    ) -> wasi::filesystem::types::DescriptorStat {
        wasi::filesystem::types::DescriptorStat {
            type_: e.type_.into(),
            link_count: e.link_count,
            size: e.size,
            data_access_timestamp: e.data_access_timestamp.map(|e| e.into()),
            data_modification_timestamp: e.data_modification_timestamp.map(|e| e.into()),
            status_change_timestamp: e.status_change_timestamp.map(|e| e.into()),
        }
    }
}

pub trait FutureReaderExt<T> {
    fn try_map<U: Lower + Lift + 'static>(
        self,
        store: impl AsContextMut,
        fun: impl FnOnce(T) -> U + Send + 'static,
    ) -> wasmtime::Result<FutureReader<U>>;
}

impl<T: Lift + Send + 'static> FutureReaderExt<T> for FutureReader<T> {
    fn try_map<U: Lower + Lift + 'static>(
        self,
        mut store: impl AsContextMut,
        fun: impl FnOnce(T) -> U + Send + 'static,
    ) -> wasmtime::Result<FutureReader<U>> {
        pin_project! {
            struct Producer<T, F> {
                #[pin]
                rx: oneshot::Receiver<T>,
                fun: Option<F>,
            }
        }

        impl<D, T: Send + 'static, U, F: FnOnce(T) -> U + Send + 'static> FutureProducer<D>
            for Producer<T, F>
        {
            type Item = U;

            fn poll_produce(
                self: Pin<&mut Self>,
                cx: &mut Context<'_>,
                _store: StoreContextMut<D>,
                finish: bool,
            ) -> Poll<wasmtime::Result<Option<Self::Item>>> {
                let me = self.project();

                match me.rx.poll(cx) {
                    Poll::Pending if finish => Poll::Ready(Ok(None)),
                    Poll::Pending => Poll::Pending,
                    Poll::Ready(result) => {
                        Poll::Ready(result.map_err(wasmtime::Error::from).and_then(|value| {
                            Ok(Some((me
                                .fun
                                .take()
                                .context("oneshot channel yielded more than one value")?)(
                                value,
                            )))
                        }))
                    }
                }
            }
        }

        struct Consumer<T> {
            tx: Option<oneshot::Sender<T>>,
        }

        impl<D, T: Lift + Send + 'static> FutureConsumer<D> for Consumer<T> {
            type Item = T;

            fn poll_consume(
                mut self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
                store: StoreContextMut<D>,
                mut source: Source<'_, Self::Item>,
                _finish: bool,
            ) -> Poll<wasmtime::Result<()>> {
                let mut result = None;
                source
                    .read(store, &mut result)
                    .context("failed to read result")?;
                let result = result.context("result value missing")?;
                let tx = self.tx.take().context("polled after returning `Ready`")?;
                _ = tx.send(result);
                Poll::Ready(Ok(()))
            }
        }

        let (tx, rx) = oneshot::channel();
        let mapped = FutureReader::new(store.as_context_mut(), Producer { rx, fun: Some(fun) })?;
        self.pipe(store, Consumer { tx: Some(tx) })?;
        Ok(mapped)
    }
}

pub trait StreamReaderExt<T> {
    fn try_map<U: Lower + Lift + Send + Sync + 'static>(
        self,
        store: impl AsContextMut,
        fun: impl Fn(T) -> U + Send + 'static,
    ) -> wasmtime::Result<StreamReader<U>>;
}

impl<T: Lift + Send + 'static> StreamReaderExt<T> for StreamReader<T> {
    fn try_map<U: Lower + Lift + Send + Sync + 'static>(
        self,
        mut store: impl AsContextMut,
        fun: impl Fn(T) -> U + Send + 'static,
    ) -> wasmtime::Result<StreamReader<U>> {
        pin_project! {
            struct Producer<T, F> {
                #[pin]
                rx: mpsc::Receiver<T>,
                fun: F,
            }
        }

        impl<D, T: Send + 'static, U: Send + Sync + 'static, F: Fn(T) -> U + Send + 'static>
            StreamProducer<D> for Producer<T, F>
        {
            type Item = U;
            type Buffer = Option<U>;

            fn poll_produce<'a>(
                self: Pin<&mut Self>,
                cx: &mut Context<'_>,
                _store: StoreContextMut<'a, D>,
                mut destination: Destination<'a, Self::Item, Self::Buffer>,
                finish: bool,
            ) -> Poll<wasmtime::Result<StreamResult>> {
                let me = self.project();

                match me.rx.poll_next(cx) {
                    Poll::Pending if finish => Poll::Ready(Ok(StreamResult::Cancelled)),
                    Poll::Pending => Poll::Pending,
                    Poll::Ready(Some(result)) => {
                        destination.set_buffer(Some((me.fun)(result)));
                        Poll::Ready(Ok(StreamResult::Completed))
                    }
                    Poll::Ready(None) => Poll::Ready(Ok(StreamResult::Dropped)),
                }
            }
        }

        struct Consumer<T> {
            tx: mpsc::Sender<T>,
        }

        impl<D, T: Lift + Send + 'static> StreamConsumer<D> for Consumer<T> {
            type Item = T;

            fn poll_consume(
                mut self: Pin<&mut Self>,
                cx: &mut Context<'_>,
                store: StoreContextMut<D>,
                mut source: Source<'_, Self::Item>,
                finish: bool,
            ) -> Poll<wasmtime::Result<StreamResult>> {
                match self.tx.poll_ready(cx) {
                    Poll::Pending if finish => Poll::Ready(Ok(StreamResult::Cancelled)),
                    Poll::Pending => Poll::Pending,
                    Poll::Ready(Ok(())) => {
                        let mut result = None;
                        source
                            .read(store, &mut result)
                            .context("failed to read result")?;
                        let result = result.context("result value missing")?;
                        self.tx.start_send(result)?;
                        Poll::Ready(Ok(StreamResult::Completed))
                    }
                    Poll::Ready(Err(error)) if error.is_disconnected() => {
                        Poll::Ready(Ok(StreamResult::Dropped))
                    }
                    Poll::Ready(Err(error)) => Poll::Ready(Err(error.into())),
                }
            }
        }

        let (tx, rx) = mpsc::channel(1);
        let mapped = StreamReader::new(store.as_context_mut(), Producer { rx, fun })?;
        self.pipe(store, Consumer { tx })?;
        Ok(mapped)
    }
}

pub fn reborrow<'a, T: 'static, D: HasData + ?Sized>(
    access: &'a mut Access<'_, T, D>,
) -> Access<'a, T, D> {
    let getter = access.getter();
    Access::new(access.as_context_mut(), getter)
}
