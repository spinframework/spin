use std::io::{self, Read, Write};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite};
use wasmtime_wasi::cli::{IsTerminal, StdinStream, StdoutStream};
use wasmtime_wasi::p2::{InputStream, OutputStream, Pollable, StreamError};

/// A [`OutputStream`] that writes to a `Write` type.
///
/// `StdinStream::stream` and `StdoutStream::new` can be called more than once in components
/// which are composed of multiple subcomponents, since each subcomponent will potentially want
/// its own handle. This means the streams need to be shareable. The easiest way to do that is
/// provide cloneable implementations of streams which operate synchronously.
///
/// Note that this amounts to doing synchronous I/O in an asynchronous context, which we'd normally
/// prefer to avoid, but the properly asynchronous implementations Host{In|Out}putStream based on
/// `AsyncRead`/`AsyncWrite`` are quite hairy and probably not worth it for "normal" stdio streams in
/// Spin. If this does prove to be a performance bottleneck, though, we can certainly revisit it.
pub struct PipedWriteStream<T>(Arc<Mutex<T>>);

impl<T> PipedWriteStream<T> {
    pub fn new(inner: T) -> Self {
        Self(Arc::new(Mutex::new(inner)))
    }
}

impl<T> Clone for PipedWriteStream<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: Write + Send + Sync + 'static> OutputStream for PipedWriteStream<T> {
    fn write(&mut self, bytes: bytes::Bytes) -> Result<(), StreamError> {
        self.0
            .lock()
            .unwrap()
            .write_all(&bytes)
            .map_err(|e| StreamError::LastOperationFailed(wasmtime::Error::from(e)))
    }

    fn flush(&mut self) -> Result<(), StreamError> {
        self.0
            .lock()
            .unwrap()
            .flush()
            .map_err(|e| StreamError::LastOperationFailed(wasmtime::Error::from(e)))
    }

    fn check_write(&mut self) -> Result<usize, StreamError> {
        Ok(1024 * 1024)
    }
}

impl<T: Write + Send + Sync + 'static> AsyncWrite for PipedWriteStream<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Poll::Ready(self.0.lock().unwrap().write(buf))
    }
    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(self.0.lock().unwrap().flush())
    }
    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

impl<T> IsTerminal for PipedWriteStream<T> {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl<T: Write + Send + Sync + 'static> StdoutStream for PipedWriteStream<T> {
    fn p2_stream(&self) -> Box<dyn OutputStream> {
        Box::new(self.clone())
    }
    fn async_stream(&self) -> Box<dyn AsyncWrite + Send + Sync> {
        Box::new(self.clone())
    }
}

#[async_trait]
impl<T: Write + Send + Sync + 'static> Pollable for PipedWriteStream<T> {
    async fn ready(&mut self) {}
}

/// A [`InputStream`] that reads to a `Read` type.
///
/// See [`PipedWriteStream`] for more information on why this is synchronous.
pub struct PipeReadStream<T> {
    buffer: Vec<u8>,
    inner: Arc<Mutex<T>>,
}

impl<T> PipeReadStream<T> {
    pub fn new(inner: T) -> Self {
        Self {
            buffer: vec![0_u8; 64 * 1024],
            inner: Arc::new(Mutex::new(inner)),
        }
    }
}

impl<T> Clone for PipeReadStream<T> {
    fn clone(&self) -> Self {
        Self {
            buffer: vec![0_u8; 64 * 1024],
            inner: self.inner.clone(),
        }
    }
}

impl<T> IsTerminal for PipeReadStream<T> {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl<T: Read + Send + Sync + 'static> InputStream for PipeReadStream<T> {
    fn read(&mut self, size: usize) -> wasmtime_wasi::p2::StreamResult<bytes::Bytes> {
        let size = size.min(self.buffer.len());

        let count = self
            .inner
            .lock()
            .unwrap()
            .read(&mut self.buffer[..size])
            .map_err(|e| StreamError::LastOperationFailed(wasmtime::Error::from(e)))?;
        if count == 0 {
            return Err(wasmtime_wasi::p2::StreamError::Closed);
        }

        Ok(bytes::Bytes::copy_from_slice(&self.buffer[..count]))
    }
}

impl<T: Read + Send + Sync + 'static> AsyncRead for PipeReadStream<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let result = self
            .inner
            .lock()
            .unwrap()
            .read(buf.initialize_unfilled())
            .map(|n| buf.advance(n));
        Poll::Ready(result)
    }
}

#[async_trait]
impl<T: Read + Send + Sync + 'static> Pollable for PipeReadStream<T> {
    async fn ready(&mut self) {}
}

impl<T: Read + Send + Sync + 'static> StdinStream for PipeReadStream<T> {
    fn p2_stream(&self) -> Box<dyn InputStream> {
        Box::new(self.clone())
    }

    fn async_stream(&self) -> Box<dyn AsyncRead + Send + Sync> {
        Box::new(self.clone())
    }
}
