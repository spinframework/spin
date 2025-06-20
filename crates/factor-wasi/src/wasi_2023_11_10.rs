use super::wasi_2023_10_18::{convert, convert_result};
use spin_factors::anyhow::{self, Result};
use wasmtime::component::{Linker, Resource};
use wasmtime_wasi::p2::{WasiImpl, WasiView};

mod latest {
    pub use wasmtime_wasi::p2::bindings::*;
}

mod bindings {
    use super::latest;

    wasmtime::component::bindgen!({
        path: "../../wit",
        world: "wasi:cli/reactor@0.2.0-rc-2023-11-10",
        async: {
            only_imports: [
                "[drop]input-stream",
                "[drop]output-stream",
                "[method]descriptor.advise",
                "[method]descriptor.create-directory-at",
                "[method]descriptor.get-flags",
                "[method]descriptor.get-type",
                "[method]descriptor.is-same-object",
                "[method]descriptor.link-at",
                "[method]descriptor.metadata-hash",
                "[method]descriptor.metadata-hash-at",
                "[method]descriptor.open-at",
                "[method]descriptor.read",
                "[method]descriptor.read-directory",
                "[method]descriptor.readlink-at",
                "[method]descriptor.remove-directory-at",
                "[method]descriptor.rename-at",
                "[method]descriptor.set-size",
                "[method]descriptor.set-times",
                "[method]descriptor.set-times-at",
                "[method]descriptor.stat",
                "[method]descriptor.stat-at",
                "[method]descriptor.symlink-at",
                "[method]descriptor.sync",
                "[method]descriptor.sync-data",
                "[method]descriptor.unlink-file-at",
                "[method]descriptor.write",
                "[method]input-stream.blocking-read",
                "[method]input-stream.blocking-skip",
                "[method]output-stream.blocking-splice",
                "[method]output-stream.blocking-flush",
                "[method]output-stream.blocking-write",
                "[method]output-stream.blocking-write-and-flush",
                "[method]output-stream.blocking-write-zeroes-and-flush",
                "[method]directory-entry-stream.read-directory-entry",
                "[method]pollable.block",
                "[method]pollable.ready",
                "poll",

                "[method]tcp-socket.start-bind",
                "[method]tcp-socket.start-connect",
                "[method]udp-socket.start-bind",
                "[method]udp-socket.stream",
                "[method]outgoing-datagram-stream.send",
            ]
        },
        with: {
            "wasi:io/poll/pollable": latest::io::poll::Pollable,
            "wasi:io/streams/input-stream": latest::io::streams::InputStream,
            "wasi:io/streams/output-stream": latest::io::streams::OutputStream,
            "wasi:io/error/error": latest::io::error::Error,
            "wasi:filesystem/types/directory-entry-stream": latest::filesystem::types::DirectoryEntryStream,
            "wasi:filesystem/types/descriptor": latest::filesystem::types::Descriptor,
            "wasi:cli/terminal-input/terminal-input": latest::cli::terminal_input::TerminalInput,
            "wasi:cli/terminal-output/terminal-output": latest::cli::terminal_output::TerminalOutput,
            "wasi:sockets/tcp/tcp-socket": latest::sockets::tcp::TcpSocket,
            "wasi:sockets/udp/udp-socket": latest::sockets::udp::UdpSocket,
            "wasi:sockets/udp/outgoing-datagram-stream": latest::sockets::udp::OutgoingDatagramStream,
            "wasi:sockets/udp/incoming-datagram-stream": latest::sockets::udp::IncomingDatagramStream,
            "wasi:sockets/network/network": latest::sockets::network::Network,
            "wasi:sockets/ip-name-lookup/resolve-address-stream": latest::sockets::ip_name_lookup::ResolveAddressStream,
        },
        trappable_imports: true,
    });
}

mod wasi {
    pub use super::bindings::wasi::{
        cli0_2_0_rc_2023_11_10 as cli, clocks0_2_0_rc_2023_11_10 as clocks,
        filesystem0_2_0_rc_2023_11_10 as filesystem, io0_2_0_rc_2023_11_10 as io,
        random0_2_0_rc_2023_11_10 as random, sockets0_2_0_rc_2023_11_10 as sockets,
    };
}

use wasi::cli::terminal_input::TerminalInput;
use wasi::cli::terminal_output::TerminalOutput;
use wasi::clocks::monotonic_clock::{Duration, Instant};
use wasi::clocks::wall_clock::Datetime;
use wasi::filesystem::types::{
    Advice, Descriptor, DescriptorFlags, DescriptorStat, DescriptorType, DirectoryEntry,
    DirectoryEntryStream, ErrorCode as FsErrorCode, Filesize, MetadataHashValue, NewTimestamp,
    OpenFlags, PathFlags,
};
use wasi::io::poll::Pollable;
use wasi::io::streams::{Error as IoError, InputStream, OutputStream, StreamError};
use wasi::sockets::ip_name_lookup::{IpAddress, ResolveAddressStream};
use wasi::sockets::network::{Ipv4SocketAddress, Ipv6SocketAddress};
use wasi::sockets::tcp::{
    ErrorCode as SocketErrorCode, IpAddressFamily, IpSocketAddress, Network, ShutdownType,
    TcpSocket,
};
use wasi::sockets::udp::{
    IncomingDatagram, IncomingDatagramStream, OutgoingDatagram, OutgoingDatagramStream, UdpSocket,
};

use crate::{HasWasi, WasiImplInner};

pub fn add_to_linker<T>(
    linker: &mut Linker<T>,
    closure: fn(&mut T) -> WasiImpl<WasiImplInner<'_>>,
) -> Result<()>
where
    T: Send + 'static,
{
    wasi::clocks::monotonic_clock::add_to_linker::<_, HasWasi>(linker, closure)?;
    wasi::clocks::wall_clock::add_to_linker::<_, HasWasi>(linker, closure)?;
    wasi::filesystem::types::add_to_linker::<_, HasWasi>(linker, closure)?;
    wasi::filesystem::preopens::add_to_linker::<_, HasWasi>(linker, closure)?;
    wasi::io::error::add_to_linker::<_, HasWasi>(linker, closure)?;
    wasi::io::poll::add_to_linker::<_, HasWasi>(linker, closure)?;
    wasi::io::streams::add_to_linker::<_, HasWasi>(linker, closure)?;
    wasi::random::random::add_to_linker::<_, HasWasi>(linker, closure)?;
    wasi::random::insecure::add_to_linker::<_, HasWasi>(linker, closure)?;
    wasi::random::insecure_seed::add_to_linker::<_, HasWasi>(linker, closure)?;
    wasi::cli::exit::add_to_linker::<_, HasWasi>(linker, closure)?;
    wasi::cli::environment::add_to_linker::<_, HasWasi>(linker, closure)?;
    wasi::cli::stdin::add_to_linker::<_, HasWasi>(linker, closure)?;
    wasi::cli::stdout::add_to_linker::<_, HasWasi>(linker, closure)?;
    wasi::cli::stderr::add_to_linker::<_, HasWasi>(linker, closure)?;
    wasi::cli::terminal_input::add_to_linker::<_, HasWasi>(linker, closure)?;
    wasi::cli::terminal_output::add_to_linker::<_, HasWasi>(linker, closure)?;
    wasi::cli::terminal_stdin::add_to_linker::<_, HasWasi>(linker, closure)?;
    wasi::cli::terminal_stdout::add_to_linker::<_, HasWasi>(linker, closure)?;
    wasi::cli::terminal_stderr::add_to_linker::<_, HasWasi>(linker, closure)?;
    wasi::sockets::tcp::add_to_linker::<_, HasWasi>(linker, closure)?;
    wasi::sockets::tcp_create_socket::add_to_linker::<_, HasWasi>(linker, closure)?;
    wasi::sockets::udp::add_to_linker::<_, HasWasi>(linker, closure)?;
    wasi::sockets::udp_create_socket::add_to_linker::<_, HasWasi>(linker, closure)?;
    wasi::sockets::instance_network::add_to_linker::<_, HasWasi>(linker, closure)?;
    wasi::sockets::network::add_to_linker::<_, HasWasi>(linker, closure)?;
    wasi::sockets::ip_name_lookup::add_to_linker::<_, HasWasi>(linker, closure)?;
    Ok(())
}

impl<T> wasi::clocks::monotonic_clock::Host for WasiImpl<T>
where
    T: WasiView,
{
    fn now(&mut self) -> wasmtime::Result<Instant> {
        latest::clocks::monotonic_clock::Host::now(self)
    }

    fn resolution(&mut self) -> wasmtime::Result<Instant> {
        latest::clocks::monotonic_clock::Host::resolution(self)
    }

    fn subscribe_instant(&mut self, when: Instant) -> wasmtime::Result<Resource<Pollable>> {
        latest::clocks::monotonic_clock::Host::subscribe_instant(self, when)
    }

    fn subscribe_duration(&mut self, when: Duration) -> wasmtime::Result<Resource<Pollable>> {
        latest::clocks::monotonic_clock::Host::subscribe_duration(self, when)
    }
}

impl<T> wasi::clocks::wall_clock::Host for WasiImpl<T>
where
    T: WasiView,
{
    fn now(&mut self) -> wasmtime::Result<Datetime> {
        Ok(latest::clocks::wall_clock::Host::now(self)?.into())
    }

    fn resolution(&mut self) -> wasmtime::Result<Datetime> {
        Ok(latest::clocks::wall_clock::Host::resolution(self)?.into())
    }
}

impl<T> wasi::filesystem::types::Host for WasiImpl<T>
where
    T: WasiView,
{
    fn filesystem_error_code(
        &mut self,
        err: Resource<wasi::filesystem::types::Error>,
    ) -> wasmtime::Result<Option<FsErrorCode>> {
        Ok(latest::filesystem::types::Host::filesystem_error_code(self, err)?.map(|e| e.into()))
    }
}

impl<T> wasi::filesystem::types::HostDescriptor for WasiImpl<T>
where
    T: WasiView,
{
    fn read_via_stream(
        &mut self,
        self_: Resource<Descriptor>,
        offset: Filesize,
    ) -> wasmtime::Result<Result<Resource<InputStream>, FsErrorCode>> {
        convert_result(latest::filesystem::types::HostDescriptor::read_via_stream(
            self, self_, offset,
        ))
    }

    fn write_via_stream(
        &mut self,
        self_: Resource<Descriptor>,
        offset: Filesize,
    ) -> wasmtime::Result<Result<Resource<OutputStream>, FsErrorCode>> {
        convert_result(latest::filesystem::types::HostDescriptor::write_via_stream(
            self, self_, offset,
        ))
    }

    fn append_via_stream(
        &mut self,
        self_: Resource<Descriptor>,
    ) -> wasmtime::Result<Result<Resource<OutputStream>, FsErrorCode>> {
        convert_result(latest::filesystem::types::HostDescriptor::append_via_stream(self, self_))
    }

    async fn advise(
        &mut self,
        self_: Resource<Descriptor>,
        offset: Filesize,
        length: Filesize,
        advice: Advice,
    ) -> wasmtime::Result<Result<(), FsErrorCode>> {
        convert_result(
            latest::filesystem::types::HostDescriptor::advise(
                self,
                self_,
                offset,
                length,
                advice.into(),
            )
            .await,
        )
    }

    async fn sync_data(
        &mut self,
        self_: Resource<Descriptor>,
    ) -> wasmtime::Result<Result<(), FsErrorCode>> {
        convert_result(latest::filesystem::types::HostDescriptor::sync_data(self, self_).await)
    }

    async fn get_flags(
        &mut self,
        self_: Resource<Descriptor>,
    ) -> wasmtime::Result<Result<DescriptorFlags, FsErrorCode>> {
        convert_result(latest::filesystem::types::HostDescriptor::get_flags(self, self_).await)
    }

    async fn get_type(
        &mut self,
        self_: Resource<Descriptor>,
    ) -> wasmtime::Result<Result<DescriptorType, FsErrorCode>> {
        convert_result(latest::filesystem::types::HostDescriptor::get_type(self, self_).await)
    }

    async fn set_size(
        &mut self,
        self_: Resource<Descriptor>,
        size: Filesize,
    ) -> wasmtime::Result<Result<(), FsErrorCode>> {
        convert_result(latest::filesystem::types::HostDescriptor::set_size(self, self_, size).await)
    }

    async fn set_times(
        &mut self,
        self_: Resource<Descriptor>,
        data_access_timestamp: NewTimestamp,
        data_modification_timestamp: NewTimestamp,
    ) -> wasmtime::Result<Result<(), FsErrorCode>> {
        convert_result(
            latest::filesystem::types::HostDescriptor::set_times(
                self,
                self_,
                data_access_timestamp.into(),
                data_modification_timestamp.into(),
            )
            .await,
        )
    }

    async fn read(
        &mut self,
        self_: Resource<Descriptor>,
        length: Filesize,
        offset: Filesize,
    ) -> wasmtime::Result<Result<(Vec<u8>, bool), FsErrorCode>> {
        convert_result(
            latest::filesystem::types::HostDescriptor::read(self, self_, length, offset).await,
        )
    }

    async fn write(
        &mut self,
        self_: Resource<Descriptor>,
        buffer: Vec<u8>,
        offset: Filesize,
    ) -> wasmtime::Result<Result<Filesize, FsErrorCode>> {
        convert_result(
            latest::filesystem::types::HostDescriptor::write(self, self_, buffer, offset).await,
        )
    }

    async fn read_directory(
        &mut self,
        self_: Resource<Descriptor>,
    ) -> wasmtime::Result<Result<Resource<DirectoryEntryStream>, FsErrorCode>> {
        convert_result(latest::filesystem::types::HostDescriptor::read_directory(self, self_).await)
    }

    async fn sync(
        &mut self,
        self_: Resource<Descriptor>,
    ) -> wasmtime::Result<Result<(), FsErrorCode>> {
        convert_result(latest::filesystem::types::HostDescriptor::sync(self, self_).await)
    }

    async fn create_directory_at(
        &mut self,
        self_: Resource<Descriptor>,
        path: String,
    ) -> wasmtime::Result<Result<(), FsErrorCode>> {
        convert_result(
            latest::filesystem::types::HostDescriptor::create_directory_at(self, self_, path).await,
        )
    }

    async fn stat(
        &mut self,
        self_: Resource<Descriptor>,
    ) -> wasmtime::Result<Result<DescriptorStat, FsErrorCode>> {
        convert_result(latest::filesystem::types::HostDescriptor::stat(self, self_).await)
    }

    async fn stat_at(
        &mut self,
        self_: Resource<Descriptor>,
        path_flags: PathFlags,
        path: String,
    ) -> wasmtime::Result<Result<DescriptorStat, FsErrorCode>> {
        convert_result(
            latest::filesystem::types::HostDescriptor::stat_at(
                self,
                self_,
                path_flags.into(),
                path,
            )
            .await,
        )
    }

    async fn set_times_at(
        &mut self,
        self_: Resource<Descriptor>,
        path_flags: PathFlags,
        path: String,
        data_access_timestamp: NewTimestamp,
        data_modification_timestamp: NewTimestamp,
    ) -> wasmtime::Result<Result<(), FsErrorCode>> {
        convert_result(
            latest::filesystem::types::HostDescriptor::set_times_at(
                self,
                self_,
                path_flags.into(),
                path,
                data_access_timestamp.into(),
                data_modification_timestamp.into(),
            )
            .await,
        )
    }

    async fn link_at(
        &mut self,
        self_: Resource<Descriptor>,
        old_path_flags: PathFlags,
        old_path: String,
        new_descriptor: Resource<Descriptor>,
        new_path: String,
    ) -> wasmtime::Result<Result<(), FsErrorCode>> {
        convert_result(
            latest::filesystem::types::HostDescriptor::link_at(
                self,
                self_,
                old_path_flags.into(),
                old_path,
                new_descriptor,
                new_path,
            )
            .await,
        )
    }

    async fn open_at(
        &mut self,
        self_: Resource<Descriptor>,
        path_flags: PathFlags,
        path: String,
        open_flags: OpenFlags,
        flags: DescriptorFlags,
    ) -> wasmtime::Result<Result<Resource<Descriptor>, FsErrorCode>> {
        convert_result(
            latest::filesystem::types::HostDescriptor::open_at(
                self,
                self_,
                path_flags.into(),
                path,
                open_flags.into(),
                flags.into(),
            )
            .await,
        )
    }

    async fn readlink_at(
        &mut self,
        self_: Resource<Descriptor>,
        path: String,
    ) -> wasmtime::Result<Result<String, FsErrorCode>> {
        convert_result(
            latest::filesystem::types::HostDescriptor::readlink_at(self, self_, path).await,
        )
    }

    async fn remove_directory_at(
        &mut self,
        self_: Resource<Descriptor>,
        path: String,
    ) -> wasmtime::Result<Result<(), FsErrorCode>> {
        convert_result(
            latest::filesystem::types::HostDescriptor::remove_directory_at(self, self_, path).await,
        )
    }

    async fn rename_at(
        &mut self,
        self_: Resource<Descriptor>,
        old_path: String,
        new_descriptor: Resource<Descriptor>,
        new_path: String,
    ) -> wasmtime::Result<Result<(), FsErrorCode>> {
        convert_result(
            latest::filesystem::types::HostDescriptor::rename_at(
                self,
                self_,
                old_path,
                new_descriptor,
                new_path,
            )
            .await,
        )
    }

    async fn symlink_at(
        &mut self,
        self_: Resource<Descriptor>,
        old_path: String,
        new_path: String,
    ) -> wasmtime::Result<Result<(), FsErrorCode>> {
        convert_result(
            latest::filesystem::types::HostDescriptor::symlink_at(self, self_, old_path, new_path)
                .await,
        )
    }

    async fn unlink_file_at(
        &mut self,
        self_: Resource<Descriptor>,
        path: String,
    ) -> wasmtime::Result<Result<(), FsErrorCode>> {
        convert_result(
            latest::filesystem::types::HostDescriptor::unlink_file_at(self, self_, path).await,
        )
    }

    async fn is_same_object(
        &mut self,
        self_: Resource<Descriptor>,
        other: Resource<Descriptor>,
    ) -> wasmtime::Result<bool> {
        latest::filesystem::types::HostDescriptor::is_same_object(self, self_, other).await
    }

    async fn metadata_hash(
        &mut self,
        self_: Resource<Descriptor>,
    ) -> wasmtime::Result<Result<MetadataHashValue, FsErrorCode>> {
        convert_result(latest::filesystem::types::HostDescriptor::metadata_hash(self, self_).await)
    }

    async fn metadata_hash_at(
        &mut self,
        self_: Resource<Descriptor>,
        path_flags: PathFlags,
        path: String,
    ) -> wasmtime::Result<Result<MetadataHashValue, FsErrorCode>> {
        convert_result(
            latest::filesystem::types::HostDescriptor::metadata_hash_at(
                self,
                self_,
                path_flags.into(),
                path,
            )
            .await,
        )
    }

    fn drop(&mut self, rep: Resource<Descriptor>) -> wasmtime::Result<()> {
        latest::filesystem::types::HostDescriptor::drop(self, rep)
    }
}

impl<T> wasi::filesystem::types::HostDirectoryEntryStream for WasiImpl<T>
where
    T: WasiView,
{
    async fn read_directory_entry(
        &mut self,
        self_: Resource<DirectoryEntryStream>,
    ) -> wasmtime::Result<Result<Option<DirectoryEntry>, FsErrorCode>> {
        convert_result(
            latest::filesystem::types::HostDirectoryEntryStream::read_directory_entry(self, self_)
                .await
                .map(|e| e.map(DirectoryEntry::from)),
        )
    }

    fn drop(&mut self, rep: Resource<DirectoryEntryStream>) -> wasmtime::Result<()> {
        latest::filesystem::types::HostDirectoryEntryStream::drop(self, rep)
    }
}

impl<T> wasi::filesystem::preopens::Host for WasiImpl<T>
where
    T: WasiView,
{
    fn get_directories(&mut self) -> wasmtime::Result<Vec<(Resource<Descriptor>, String)>> {
        latest::filesystem::preopens::Host::get_directories(self)
    }
}

impl<T> wasi::io::poll::Host for WasiImpl<T>
where
    T: WasiView,
{
    async fn poll(&mut self, list: Vec<Resource<Pollable>>) -> wasmtime::Result<Vec<u32>> {
        latest::io::poll::Host::poll(&mut self.0, list).await
    }
}

impl<T> wasi::io::poll::HostPollable for WasiImpl<T>
where
    T: WasiView,
{
    async fn block(&mut self, rep: Resource<Pollable>) -> wasmtime::Result<()> {
        latest::io::poll::HostPollable::block(&mut self.0, rep).await
    }

    async fn ready(&mut self, rep: Resource<Pollable>) -> wasmtime::Result<bool> {
        latest::io::poll::HostPollable::ready(&mut self.0, rep).await
    }

    fn drop(&mut self, rep: Resource<Pollable>) -> wasmtime::Result<()> {
        latest::io::poll::HostPollable::drop(&mut self.0, rep)
    }
}

impl<T> wasi::io::error::Host for WasiImpl<T> where T: WasiView {}

impl<T> wasi::io::error::HostError for WasiImpl<T>
where
    T: WasiView,
{
    fn to_debug_string(&mut self, self_: Resource<IoError>) -> wasmtime::Result<String> {
        latest::io::error::HostError::to_debug_string(&mut self.0, self_)
    }

    fn drop(&mut self, rep: Resource<IoError>) -> wasmtime::Result<()> {
        latest::io::error::HostError::drop(&mut self.0, rep)
    }
}

fn convert_stream_result<T, T2>(
    mut view: impl WasiView,
    result: Result<T, wasmtime_wasi::p2::StreamError>,
) -> wasmtime::Result<Result<T2, StreamError>>
where
    T2: From<T>,
{
    match result {
        Ok(e) => Ok(Ok(e.into())),
        Err(wasmtime_wasi::p2::StreamError::Closed) => Ok(Err(StreamError::Closed)),
        Err(wasmtime_wasi::p2::StreamError::LastOperationFailed(e)) => {
            let e = view.table().push(e)?;
            Ok(Err(StreamError::LastOperationFailed(e)))
        }
        Err(wasmtime_wasi::p2::StreamError::Trap(e)) => Err(e),
    }
}

impl<T> wasi::io::streams::Host for WasiImpl<T> where T: WasiView {}

impl<T> wasi::io::streams::HostInputStream for WasiImpl<T>
where
    T: WasiView,
{
    fn read(
        &mut self,
        self_: Resource<InputStream>,
        len: u64,
    ) -> wasmtime::Result<Result<Vec<u8>, StreamError>> {
        let result = latest::io::streams::HostInputStream::read(&mut self.0, self_, len);
        convert_stream_result(self, result)
    }

    async fn blocking_read(
        &mut self,
        self_: Resource<InputStream>,
        len: u64,
    ) -> wasmtime::Result<Result<Vec<u8>, StreamError>> {
        let result =
            latest::io::streams::HostInputStream::blocking_read(&mut self.0, self_, len).await;
        convert_stream_result(self, result)
    }

    fn skip(
        &mut self,
        self_: Resource<InputStream>,
        len: u64,
    ) -> wasmtime::Result<Result<u64, StreamError>> {
        let result = latest::io::streams::HostInputStream::skip(&mut self.0, self_, len);
        convert_stream_result(self, result)
    }

    async fn blocking_skip(
        &mut self,
        self_: Resource<InputStream>,
        len: u64,
    ) -> wasmtime::Result<Result<u64, StreamError>> {
        let result =
            latest::io::streams::HostInputStream::blocking_skip(&mut self.0, self_, len).await;
        convert_stream_result(self, result)
    }

    fn subscribe(&mut self, self_: Resource<InputStream>) -> wasmtime::Result<Resource<Pollable>> {
        latest::io::streams::HostInputStream::subscribe(&mut self.0, self_)
    }

    async fn drop(&mut self, rep: Resource<InputStream>) -> wasmtime::Result<()> {
        latest::io::streams::HostInputStream::drop(&mut self.0, rep).await
    }
}

impl<T> wasi::io::streams::HostOutputStream for WasiImpl<T>
where
    T: WasiView,
{
    fn check_write(
        &mut self,
        self_: Resource<OutputStream>,
    ) -> wasmtime::Result<Result<u64, StreamError>> {
        let result = latest::io::streams::HostOutputStream::check_write(&mut self.0, self_);
        convert_stream_result(self, result)
    }

    fn write(
        &mut self,
        self_: Resource<OutputStream>,
        contents: Vec<u8>,
    ) -> wasmtime::Result<Result<(), StreamError>> {
        let result = latest::io::streams::HostOutputStream::write(&mut self.0, self_, contents);
        convert_stream_result(self, result)
    }

    async fn blocking_write_and_flush(
        &mut self,
        self_: Resource<OutputStream>,
        contents: Vec<u8>,
    ) -> wasmtime::Result<Result<(), StreamError>> {
        let result = latest::io::streams::HostOutputStream::blocking_write_and_flush(
            &mut self.0,
            self_,
            contents,
        )
        .await;
        convert_stream_result(self, result)
    }

    fn flush(
        &mut self,
        self_: Resource<OutputStream>,
    ) -> wasmtime::Result<Result<(), StreamError>> {
        let result = latest::io::streams::HostOutputStream::flush(&mut self.0, self_);
        convert_stream_result(self, result)
    }

    async fn blocking_flush(
        &mut self,
        self_: Resource<OutputStream>,
    ) -> wasmtime::Result<Result<(), StreamError>> {
        let result =
            latest::io::streams::HostOutputStream::blocking_flush(&mut self.0, self_).await;
        convert_stream_result(self, result)
    }

    fn subscribe(&mut self, self_: Resource<OutputStream>) -> wasmtime::Result<Resource<Pollable>> {
        latest::io::streams::HostOutputStream::subscribe(&mut self.0, self_)
    }

    fn write_zeroes(
        &mut self,
        self_: Resource<OutputStream>,
        len: u64,
    ) -> wasmtime::Result<Result<(), StreamError>> {
        let result = latest::io::streams::HostOutputStream::write_zeroes(&mut self.0, self_, len);
        convert_stream_result(self, result)
    }

    async fn blocking_write_zeroes_and_flush(
        &mut self,
        self_: Resource<OutputStream>,
        len: u64,
    ) -> wasmtime::Result<Result<(), StreamError>> {
        let result = latest::io::streams::HostOutputStream::blocking_write_zeroes_and_flush(
            &mut self.0,
            self_,
            len,
        )
        .await;
        convert_stream_result(self, result)
    }

    fn splice(
        &mut self,
        self_: Resource<OutputStream>,
        src: Resource<InputStream>,
        len: u64,
    ) -> wasmtime::Result<Result<u64, StreamError>> {
        let result = latest::io::streams::HostOutputStream::splice(&mut self.0, self_, src, len);
        convert_stream_result(self, result)
    }

    async fn blocking_splice(
        &mut self,
        self_: Resource<OutputStream>,
        src: Resource<InputStream>,
        len: u64,
    ) -> wasmtime::Result<Result<u64, StreamError>> {
        let result =
            latest::io::streams::HostOutputStream::blocking_splice(&mut self.0, self_, src, len)
                .await;
        convert_stream_result(self, result)
    }

    async fn drop(&mut self, rep: Resource<OutputStream>) -> wasmtime::Result<()> {
        latest::io::streams::HostOutputStream::drop(&mut self.0, rep).await
    }
}

impl<T> wasi::random::random::Host for WasiImpl<T>
where
    T: WasiView,
{
    fn get_random_bytes(&mut self, len: u64) -> wasmtime::Result<Vec<u8>> {
        latest::random::random::Host::get_random_bytes(self, len)
    }

    fn get_random_u64(&mut self) -> wasmtime::Result<u64> {
        latest::random::random::Host::get_random_u64(self)
    }
}

impl<T> wasi::random::insecure::Host for WasiImpl<T>
where
    T: WasiView,
{
    fn get_insecure_random_bytes(&mut self, len: u64) -> wasmtime::Result<Vec<u8>> {
        latest::random::insecure::Host::get_insecure_random_bytes(self, len)
    }

    fn get_insecure_random_u64(&mut self) -> wasmtime::Result<u64> {
        latest::random::insecure::Host::get_insecure_random_u64(self)
    }
}

impl<T> wasi::random::insecure_seed::Host for WasiImpl<T>
where
    T: WasiView,
{
    fn insecure_seed(&mut self) -> wasmtime::Result<(u64, u64)> {
        latest::random::insecure_seed::Host::insecure_seed(self)
    }
}

impl<T> wasi::cli::exit::Host for WasiImpl<T>
where
    T: WasiView,
{
    fn exit(&mut self, status: Result<(), ()>) -> wasmtime::Result<()> {
        latest::cli::exit::Host::exit(self, status)
    }
}

impl<T> wasi::cli::environment::Host for WasiImpl<T>
where
    T: WasiView,
{
    fn get_environment(&mut self) -> wasmtime::Result<Vec<(String, String)>> {
        latest::cli::environment::Host::get_environment(self)
    }

    fn get_arguments(&mut self) -> wasmtime::Result<Vec<String>> {
        latest::cli::environment::Host::get_arguments(self)
    }

    fn initial_cwd(&mut self) -> wasmtime::Result<Option<String>> {
        latest::cli::environment::Host::initial_cwd(self)
    }
}

impl<T> wasi::cli::stdin::Host for WasiImpl<T>
where
    T: WasiView,
{
    fn get_stdin(&mut self) -> wasmtime::Result<Resource<InputStream>> {
        latest::cli::stdin::Host::get_stdin(self)
    }
}

impl<T> wasi::cli::stdout::Host for WasiImpl<T>
where
    T: WasiView,
{
    fn get_stdout(&mut self) -> wasmtime::Result<Resource<OutputStream>> {
        latest::cli::stdout::Host::get_stdout(self)
    }
}

impl<T> wasi::cli::stderr::Host for WasiImpl<T>
where
    T: WasiView,
{
    fn get_stderr(&mut self) -> wasmtime::Result<Resource<OutputStream>> {
        latest::cli::stderr::Host::get_stderr(self)
    }
}

impl<T> wasi::cli::terminal_stdin::Host for WasiImpl<T>
where
    T: WasiView,
{
    fn get_terminal_stdin(&mut self) -> wasmtime::Result<Option<Resource<TerminalInput>>> {
        latest::cli::terminal_stdin::Host::get_terminal_stdin(self)
    }
}

impl<T> wasi::cli::terminal_stdout::Host for WasiImpl<T>
where
    T: WasiView,
{
    fn get_terminal_stdout(&mut self) -> wasmtime::Result<Option<Resource<TerminalOutput>>> {
        latest::cli::terminal_stdout::Host::get_terminal_stdout(self)
    }
}

impl<T> wasi::cli::terminal_stderr::Host for WasiImpl<T>
where
    T: WasiView,
{
    fn get_terminal_stderr(&mut self) -> wasmtime::Result<Option<Resource<TerminalOutput>>> {
        latest::cli::terminal_stderr::Host::get_terminal_stderr(self)
    }
}

impl<T> wasi::cli::terminal_input::Host for WasiImpl<T> where T: WasiView {}

impl<T> wasi::cli::terminal_input::HostTerminalInput for WasiImpl<T>
where
    T: WasiView,
{
    fn drop(&mut self, rep: Resource<TerminalInput>) -> wasmtime::Result<()> {
        latest::cli::terminal_input::HostTerminalInput::drop(self, rep)
    }
}

impl<T> wasi::cli::terminal_output::Host for WasiImpl<T> where T: WasiView {}

impl<T> wasi::cli::terminal_output::HostTerminalOutput for WasiImpl<T>
where
    T: WasiView,
{
    fn drop(&mut self, rep: Resource<TerminalOutput>) -> wasmtime::Result<()> {
        latest::cli::terminal_output::HostTerminalOutput::drop(self, rep)
    }
}

impl<T> wasi::sockets::tcp::Host for WasiImpl<T> where T: WasiView {}

impl<T> wasi::sockets::tcp::HostTcpSocket for WasiImpl<T>
where
    T: WasiView,
{
    async fn start_bind(
        &mut self,
        self_: Resource<TcpSocket>,
        network: Resource<Network>,
        local_address: IpSocketAddress,
    ) -> wasmtime::Result<Result<(), SocketErrorCode>> {
        convert_result(
            latest::sockets::tcp::HostTcpSocket::start_bind(
                self,
                self_,
                network,
                local_address.into(),
            )
            .await,
        )
    }

    fn finish_bind(
        &mut self,
        self_: Resource<TcpSocket>,
    ) -> wasmtime::Result<Result<(), SocketErrorCode>> {
        convert_result(latest::sockets::tcp::HostTcpSocket::finish_bind(
            self, self_,
        ))
    }

    async fn start_connect(
        &mut self,
        self_: Resource<TcpSocket>,
        network: Resource<Network>,
        remote_address: IpSocketAddress,
    ) -> wasmtime::Result<Result<(), SocketErrorCode>> {
        convert_result(
            latest::sockets::tcp::HostTcpSocket::start_connect(
                self,
                self_,
                network,
                remote_address.into(),
            )
            .await,
        )
    }

    fn finish_connect(
        &mut self,
        self_: Resource<TcpSocket>,
    ) -> wasmtime::Result<Result<(Resource<InputStream>, Resource<OutputStream>), SocketErrorCode>>
    {
        convert_result(latest::sockets::tcp::HostTcpSocket::finish_connect(
            self, self_,
        ))
    }

    fn start_listen(
        &mut self,
        self_: Resource<TcpSocket>,
    ) -> wasmtime::Result<Result<(), SocketErrorCode>> {
        convert_result(latest::sockets::tcp::HostTcpSocket::start_listen(
            self, self_,
        ))
    }

    fn finish_listen(
        &mut self,
        self_: Resource<TcpSocket>,
    ) -> wasmtime::Result<Result<(), SocketErrorCode>> {
        convert_result(latest::sockets::tcp::HostTcpSocket::finish_listen(
            self, self_,
        ))
    }

    fn accept(
        &mut self,
        self_: Resource<TcpSocket>,
    ) -> wasmtime::Result<
        Result<
            (
                Resource<TcpSocket>,
                Resource<InputStream>,
                Resource<OutputStream>,
            ),
            SocketErrorCode,
        >,
    > {
        convert_result(latest::sockets::tcp::HostTcpSocket::accept(self, self_))
    }

    fn local_address(
        &mut self,
        self_: Resource<TcpSocket>,
    ) -> wasmtime::Result<Result<IpSocketAddress, SocketErrorCode>> {
        convert_result(latest::sockets::tcp::HostTcpSocket::local_address(
            self, self_,
        ))
    }

    fn remote_address(
        &mut self,
        self_: Resource<TcpSocket>,
    ) -> wasmtime::Result<Result<IpSocketAddress, SocketErrorCode>> {
        convert_result(latest::sockets::tcp::HostTcpSocket::remote_address(
            self, self_,
        ))
    }

    fn address_family(&mut self, self_: Resource<TcpSocket>) -> wasmtime::Result<IpAddressFamily> {
        latest::sockets::tcp::HostTcpSocket::address_family(self, self_).map(|e| e.into())
    }

    fn ipv6_only(
        &mut self,
        _self_: Resource<TcpSocket>,
    ) -> wasmtime::Result<Result<bool, SocketErrorCode>> {
        anyhow::bail!("ipv6-only API no longer supported")
    }

    fn set_ipv6_only(
        &mut self,
        _self_: Resource<TcpSocket>,
        _value: bool,
    ) -> wasmtime::Result<Result<(), SocketErrorCode>> {
        anyhow::bail!("ipv6-only API no longer supported")
    }

    fn set_listen_backlog_size(
        &mut self,
        self_: Resource<TcpSocket>,
        value: u64,
    ) -> wasmtime::Result<Result<(), SocketErrorCode>> {
        convert_result(
            latest::sockets::tcp::HostTcpSocket::set_listen_backlog_size(self, self_, value),
        )
    }

    fn is_listening(&mut self, self_: Resource<TcpSocket>) -> wasmtime::Result<bool> {
        latest::sockets::tcp::HostTcpSocket::is_listening(self, self_)
    }

    fn keep_alive_enabled(
        &mut self,
        self_: Resource<TcpSocket>,
    ) -> wasmtime::Result<Result<bool, SocketErrorCode>> {
        convert_result(latest::sockets::tcp::HostTcpSocket::keep_alive_enabled(
            self, self_,
        ))
    }

    fn set_keep_alive_enabled(
        &mut self,
        self_: Resource<TcpSocket>,
        value: bool,
    ) -> wasmtime::Result<Result<(), SocketErrorCode>> {
        convert_result(latest::sockets::tcp::HostTcpSocket::set_keep_alive_enabled(
            self, self_, value,
        ))
    }

    fn keep_alive_idle_time(
        &mut self,
        self_: Resource<TcpSocket>,
    ) -> wasmtime::Result<Result<Duration, SocketErrorCode>> {
        convert_result(latest::sockets::tcp::HostTcpSocket::keep_alive_idle_time(
            self, self_,
        ))
    }

    fn set_keep_alive_idle_time(
        &mut self,
        self_: Resource<TcpSocket>,
        value: Duration,
    ) -> wasmtime::Result<Result<(), SocketErrorCode>> {
        convert_result(
            latest::sockets::tcp::HostTcpSocket::set_keep_alive_idle_time(self, self_, value),
        )
    }

    fn keep_alive_interval(
        &mut self,
        self_: Resource<TcpSocket>,
    ) -> wasmtime::Result<Result<Duration, SocketErrorCode>> {
        convert_result(latest::sockets::tcp::HostTcpSocket::keep_alive_interval(
            self, self_,
        ))
    }

    fn set_keep_alive_interval(
        &mut self,
        self_: Resource<TcpSocket>,
        value: Duration,
    ) -> wasmtime::Result<Result<(), SocketErrorCode>> {
        convert_result(
            latest::sockets::tcp::HostTcpSocket::set_keep_alive_interval(self, self_, value),
        )
    }

    fn keep_alive_count(
        &mut self,
        self_: Resource<TcpSocket>,
    ) -> wasmtime::Result<Result<u32, SocketErrorCode>> {
        convert_result(latest::sockets::tcp::HostTcpSocket::keep_alive_count(
            self, self_,
        ))
    }

    fn set_keep_alive_count(
        &mut self,
        self_: Resource<TcpSocket>,
        value: u32,
    ) -> wasmtime::Result<Result<(), SocketErrorCode>> {
        convert_result(latest::sockets::tcp::HostTcpSocket::set_keep_alive_count(
            self, self_, value,
        ))
    }

    fn hop_limit(
        &mut self,
        self_: Resource<TcpSocket>,
    ) -> wasmtime::Result<Result<u8, SocketErrorCode>> {
        convert_result(latest::sockets::tcp::HostTcpSocket::hop_limit(self, self_))
    }

    fn set_hop_limit(
        &mut self,
        self_: Resource<TcpSocket>,
        value: u8,
    ) -> wasmtime::Result<Result<(), SocketErrorCode>> {
        convert_result(latest::sockets::tcp::HostTcpSocket::set_hop_limit(
            self, self_, value,
        ))
    }

    fn receive_buffer_size(
        &mut self,
        self_: Resource<TcpSocket>,
    ) -> wasmtime::Result<Result<u64, SocketErrorCode>> {
        convert_result(latest::sockets::tcp::HostTcpSocket::receive_buffer_size(
            self, self_,
        ))
    }

    fn set_receive_buffer_size(
        &mut self,
        self_: Resource<TcpSocket>,
        value: u64,
    ) -> wasmtime::Result<Result<(), SocketErrorCode>> {
        convert_result(
            latest::sockets::tcp::HostTcpSocket::set_receive_buffer_size(self, self_, value),
        )
    }

    fn send_buffer_size(
        &mut self,
        self_: Resource<TcpSocket>,
    ) -> wasmtime::Result<Result<u64, SocketErrorCode>> {
        convert_result(latest::sockets::tcp::HostTcpSocket::send_buffer_size(
            self, self_,
        ))
    }

    fn set_send_buffer_size(
        &mut self,
        self_: Resource<TcpSocket>,
        value: u64,
    ) -> wasmtime::Result<Result<(), SocketErrorCode>> {
        convert_result(latest::sockets::tcp::HostTcpSocket::set_send_buffer_size(
            self, self_, value,
        ))
    }

    fn subscribe(&mut self, self_: Resource<TcpSocket>) -> wasmtime::Result<Resource<Pollable>> {
        latest::sockets::tcp::HostTcpSocket::subscribe(self, self_)
    }

    fn shutdown(
        &mut self,
        self_: Resource<TcpSocket>,
        shutdown_type: ShutdownType,
    ) -> wasmtime::Result<Result<(), SocketErrorCode>> {
        convert_result(latest::sockets::tcp::HostTcpSocket::shutdown(
            self,
            self_,
            shutdown_type.into(),
        ))
    }

    fn drop(&mut self, rep: Resource<TcpSocket>) -> wasmtime::Result<()> {
        latest::sockets::tcp::HostTcpSocket::drop(self, rep)
    }
}

impl<T> wasi::sockets::tcp_create_socket::Host for WasiImpl<T>
where
    T: WasiView,
{
    fn create_tcp_socket(
        &mut self,
        address_family: IpAddressFamily,
    ) -> wasmtime::Result<Result<Resource<TcpSocket>, SocketErrorCode>> {
        convert_result(latest::sockets::tcp_create_socket::Host::create_tcp_socket(
            self,
            address_family.into(),
        ))
    }
}

impl<T> wasi::sockets::udp::Host for WasiImpl<T> where T: WasiView {}

impl<T> wasi::sockets::udp::HostUdpSocket for WasiImpl<T>
where
    T: WasiView,
{
    async fn start_bind(
        &mut self,
        self_: Resource<UdpSocket>,
        network: Resource<Network>,
        local_address: IpSocketAddress,
    ) -> wasmtime::Result<Result<(), SocketErrorCode>> {
        convert_result(
            latest::sockets::udp::HostUdpSocket::start_bind(
                self,
                self_,
                network,
                local_address.into(),
            )
            .await,
        )
    }

    fn finish_bind(
        &mut self,
        self_: Resource<UdpSocket>,
    ) -> wasmtime::Result<Result<(), SocketErrorCode>> {
        convert_result(latest::sockets::udp::HostUdpSocket::finish_bind(
            self, self_,
        ))
    }

    fn local_address(
        &mut self,
        self_: Resource<UdpSocket>,
    ) -> wasmtime::Result<Result<IpSocketAddress, SocketErrorCode>> {
        convert_result(latest::sockets::udp::HostUdpSocket::local_address(
            self, self_,
        ))
    }

    fn remote_address(
        &mut self,
        self_: Resource<UdpSocket>,
    ) -> wasmtime::Result<Result<IpSocketAddress, SocketErrorCode>> {
        convert_result(latest::sockets::udp::HostUdpSocket::remote_address(
            self, self_,
        ))
    }

    fn address_family(&mut self, self_: Resource<UdpSocket>) -> wasmtime::Result<IpAddressFamily> {
        latest::sockets::udp::HostUdpSocket::address_family(self, self_).map(|e| e.into())
    }

    fn ipv6_only(
        &mut self,
        _self_: Resource<UdpSocket>,
    ) -> wasmtime::Result<Result<bool, SocketErrorCode>> {
        anyhow::bail!("ipv6-only API no longer supported")
    }

    fn set_ipv6_only(
        &mut self,
        _self_: Resource<UdpSocket>,
        _value: bool,
    ) -> wasmtime::Result<Result<(), SocketErrorCode>> {
        anyhow::bail!("ipv6-only API no longer supported")
    }

    fn unicast_hop_limit(
        &mut self,
        self_: Resource<UdpSocket>,
    ) -> wasmtime::Result<Result<u8, SocketErrorCode>> {
        convert_result(latest::sockets::udp::HostUdpSocket::unicast_hop_limit(
            self, self_,
        ))
    }

    fn set_unicast_hop_limit(
        &mut self,
        self_: Resource<UdpSocket>,
        value: u8,
    ) -> wasmtime::Result<Result<(), SocketErrorCode>> {
        convert_result(latest::sockets::udp::HostUdpSocket::set_unicast_hop_limit(
            self, self_, value,
        ))
    }

    fn receive_buffer_size(
        &mut self,
        self_: Resource<UdpSocket>,
    ) -> wasmtime::Result<Result<u64, SocketErrorCode>> {
        convert_result(latest::sockets::udp::HostUdpSocket::receive_buffer_size(
            self, self_,
        ))
    }

    fn set_receive_buffer_size(
        &mut self,
        self_: Resource<UdpSocket>,
        value: u64,
    ) -> wasmtime::Result<Result<(), SocketErrorCode>> {
        convert_result(
            latest::sockets::udp::HostUdpSocket::set_receive_buffer_size(self, self_, value),
        )
    }

    fn send_buffer_size(
        &mut self,
        self_: Resource<UdpSocket>,
    ) -> wasmtime::Result<Result<u64, SocketErrorCode>> {
        convert_result(latest::sockets::udp::HostUdpSocket::send_buffer_size(
            self, self_,
        ))
    }

    fn set_send_buffer_size(
        &mut self,
        self_: Resource<UdpSocket>,
        value: u64,
    ) -> wasmtime::Result<Result<(), SocketErrorCode>> {
        convert_result(latest::sockets::udp::HostUdpSocket::set_send_buffer_size(
            self, self_, value,
        ))
    }

    async fn stream(
        &mut self,
        self_: Resource<UdpSocket>,
        remote_address: Option<IpSocketAddress>,
    ) -> wasmtime::Result<
        Result<
            (
                Resource<IncomingDatagramStream>,
                Resource<OutgoingDatagramStream>,
            ),
            SocketErrorCode,
        >,
    > {
        convert_result(
            latest::sockets::udp::HostUdpSocket::stream(
                self,
                self_,
                remote_address.map(|a| a.into()),
            )
            .await,
        )
    }

    fn subscribe(&mut self, self_: Resource<UdpSocket>) -> wasmtime::Result<Resource<Pollable>> {
        latest::sockets::udp::HostUdpSocket::subscribe(self, self_)
    }

    fn drop(&mut self, rep: Resource<UdpSocket>) -> wasmtime::Result<()> {
        latest::sockets::udp::HostUdpSocket::drop(self, rep)
    }
}

impl<T> wasi::sockets::udp::HostOutgoingDatagramStream for WasiImpl<T>
where
    T: WasiView,
{
    fn check_send(
        &mut self,
        self_: Resource<OutgoingDatagramStream>,
    ) -> wasmtime::Result<Result<u64, SocketErrorCode>> {
        convert_result(latest::sockets::udp::HostOutgoingDatagramStream::check_send(self, self_))
    }

    async fn send(
        &mut self,
        self_: Resource<OutgoingDatagramStream>,
        datagrams: Vec<OutgoingDatagram>,
    ) -> wasmtime::Result<Result<u64, SocketErrorCode>> {
        convert_result(
            latest::sockets::udp::HostOutgoingDatagramStream::send(
                self,
                self_,
                datagrams.into_iter().map(|d| d.into()).collect(),
            )
            .await,
        )
    }

    fn subscribe(
        &mut self,
        self_: Resource<OutgoingDatagramStream>,
    ) -> wasmtime::Result<Resource<Pollable>> {
        latest::sockets::udp::HostOutgoingDatagramStream::subscribe(self, self_)
    }

    fn drop(&mut self, rep: Resource<OutgoingDatagramStream>) -> wasmtime::Result<()> {
        latest::sockets::udp::HostOutgoingDatagramStream::drop(self, rep)
    }
}

impl<T> wasi::sockets::udp::HostIncomingDatagramStream for WasiImpl<T>
where
    T: WasiView,
{
    fn receive(
        &mut self,
        self_: Resource<IncomingDatagramStream>,
        max_results: u64,
    ) -> wasmtime::Result<Result<Vec<IncomingDatagram>, SocketErrorCode>> {
        convert_result(latest::sockets::udp::HostIncomingDatagramStream::receive(
            self,
            self_,
            max_results,
        ))
        .map(|r| r.map(|r: Vec<_>| r.into_iter().map(|d| d.into()).collect()))
    }

    fn subscribe(
        &mut self,
        self_: Resource<IncomingDatagramStream>,
    ) -> wasmtime::Result<Resource<Pollable>> {
        latest::sockets::udp::HostIncomingDatagramStream::subscribe(self, self_)
    }

    fn drop(&mut self, rep: Resource<IncomingDatagramStream>) -> wasmtime::Result<()> {
        latest::sockets::udp::HostIncomingDatagramStream::drop(self, rep)
    }
}

impl<T> wasi::sockets::udp_create_socket::Host for WasiImpl<T>
where
    T: WasiView,
{
    fn create_udp_socket(
        &mut self,
        address_family: IpAddressFamily,
    ) -> wasmtime::Result<Result<Resource<UdpSocket>, SocketErrorCode>> {
        convert_result(latest::sockets::udp_create_socket::Host::create_udp_socket(
            self,
            address_family.into(),
        ))
    }
}

impl<T> wasi::sockets::instance_network::Host for WasiImpl<T>
where
    T: WasiView,
{
    fn instance_network(&mut self) -> wasmtime::Result<Resource<Network>> {
        latest::sockets::instance_network::Host::instance_network(self)
    }
}

impl<T> wasi::sockets::network::Host for WasiImpl<T> where T: WasiView {}

impl<T> wasi::sockets::network::HostNetwork for WasiImpl<T>
where
    T: WasiView,
{
    fn drop(&mut self, rep: Resource<Network>) -> wasmtime::Result<()> {
        latest::sockets::network::HostNetwork::drop(self, rep)
    }
}

impl<T> wasi::sockets::ip_name_lookup::Host for WasiImpl<T>
where
    T: WasiView,
{
    fn resolve_addresses(
        &mut self,
        network: Resource<Network>,
        name: String,
    ) -> wasmtime::Result<Result<Resource<ResolveAddressStream>, SocketErrorCode>> {
        convert_result(latest::sockets::ip_name_lookup::Host::resolve_addresses(
            self, network, name,
        ))
    }
}

impl<T> wasi::sockets::ip_name_lookup::HostResolveAddressStream for WasiImpl<T>
where
    T: WasiView,
{
    fn resolve_next_address(
        &mut self,
        self_: Resource<ResolveAddressStream>,
    ) -> wasmtime::Result<Result<Option<IpAddress>, SocketErrorCode>> {
        convert_result(
            latest::sockets::ip_name_lookup::HostResolveAddressStream::resolve_next_address(
                self, self_,
            )
            .map(|e| e.map(|e| e.into())),
        )
    }

    fn subscribe(
        &mut self,
        self_: Resource<ResolveAddressStream>,
    ) -> wasmtime::Result<Resource<Pollable>> {
        latest::sockets::ip_name_lookup::HostResolveAddressStream::subscribe(self, self_)
    }

    fn drop(&mut self, rep: Resource<ResolveAddressStream>) -> wasmtime::Result<()> {
        latest::sockets::ip_name_lookup::HostResolveAddressStream::drop(self, rep)
    }
}

convert! {
    struct latest::clocks::wall_clock::Datetime [<=>] Datetime {
        seconds,
        nanoseconds,
    }

    enum latest::filesystem::types::ErrorCode => FsErrorCode {
        Access,
        WouldBlock,
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
    }

    enum Advice => latest::filesystem::types::Advice {
        Normal,
        Sequential,
        Random,
        WillNeed,
        DontNeed,
        NoReuse,
    }

    flags DescriptorFlags [<=>] latest::filesystem::types::DescriptorFlags {
        READ,
        WRITE,
        FILE_INTEGRITY_SYNC,
        DATA_INTEGRITY_SYNC,
        REQUESTED_WRITE_SYNC,
        MUTATE_DIRECTORY,
    }

    enum DescriptorType [<=>] latest::filesystem::types::DescriptorType {
        Unknown,
        BlockDevice,
        CharacterDevice,
        Directory,
        Fifo,
        SymbolicLink,
        RegularFile,
        Socket,
    }

    enum NewTimestamp => latest::filesystem::types::NewTimestamp {
        NoChange,
        Now,
        Timestamp(e),
    }

    flags PathFlags => latest::filesystem::types::PathFlags {
        SYMLINK_FOLLOW,
    }

    flags OpenFlags => latest::filesystem::types::OpenFlags {
        CREATE,
        DIRECTORY,
        EXCLUSIVE,
        TRUNCATE,
    }

    struct latest::filesystem::types::MetadataHashValue => MetadataHashValue {
        lower,
        upper,
    }

    struct latest::filesystem::types::DirectoryEntry => DirectoryEntry {
        type_,
        name,
    }


    enum latest::sockets::network::ErrorCode => SocketErrorCode {
        Unknown,
        AccessDenied,
        NotSupported,
        InvalidArgument,
        OutOfMemory,
        Timeout,
        ConcurrencyConflict,
        NotInProgress,
        WouldBlock,
        InvalidState,
        NewSocketLimit,
        AddressNotBindable,
        AddressInUse,
        RemoteUnreachable,
        ConnectionRefused,
        ConnectionReset,
        ConnectionAborted,
        DatagramTooLarge,
        NameUnresolvable,
        TemporaryResolverFailure,
        PermanentResolverFailure,
    }

    enum latest::sockets::network::IpAddress [<=>] IpAddress {
        Ipv4(e),
        Ipv6(e),
    }

    enum latest::sockets::network::IpSocketAddress [<=>] IpSocketAddress {
        Ipv4(e),
        Ipv6(e),
    }

    struct latest::sockets::network::Ipv4SocketAddress [<=>] Ipv4SocketAddress {
        port,
        address,
    }

    struct latest::sockets::network::Ipv6SocketAddress [<=>] Ipv6SocketAddress {
        port,
        flow_info,
        scope_id,
        address,
    }

    enum latest::sockets::network::IpAddressFamily [<=>] IpAddressFamily {
        Ipv4,
        Ipv6,
    }

    enum ShutdownType => latest::sockets::tcp::ShutdownType {
        Receive,
        Send,
        Both,
    }

    struct latest::sockets::udp::IncomingDatagram => IncomingDatagram {
        data,
        remote_address,
    }
}

impl From<latest::filesystem::types::DescriptorStat> for DescriptorStat {
    fn from(e: latest::filesystem::types::DescriptorStat) -> DescriptorStat {
        DescriptorStat {
            type_: e.type_.into(),
            link_count: e.link_count,
            size: e.size,
            data_access_timestamp: e.data_access_timestamp.map(|e| e.into()),
            data_modification_timestamp: e.data_modification_timestamp.map(|e| e.into()),
            status_change_timestamp: e.status_change_timestamp.map(|e| e.into()),
        }
    }
}

impl From<OutgoingDatagram> for latest::sockets::udp::OutgoingDatagram {
    fn from(d: OutgoingDatagram) -> Self {
        Self {
            data: d.data,
            remote_address: d.remote_address.map(|a| a.into()),
        }
    }
}
