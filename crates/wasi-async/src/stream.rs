use spin_core::wasmtime;

pub fn producer<T: Send + Sync + 'static>(rx: tokio::sync::mpsc::Receiver<T>) -> StreamProducer<T> {
    StreamProducer { rx }
}

pub struct StreamProducer<T> {
    rx: tokio::sync::mpsc::Receiver<T>,
}

impl<D, T: Send + Sync + 'static> wasmtime::component::StreamProducer<D> for StreamProducer<T> {
    type Item = T;

    type Buffer = Option<Self::Item>;

    fn poll_produce<'a>(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        store: wasmtime::StoreContextMut<'a, D>,
        mut destination: wasmtime::component::Destination<'a, Self::Item, Self::Buffer>,
        finish: bool,
    ) -> std::task::Poll<anyhow::Result<wasmtime::component::StreamResult>> {
        use std::task::Poll;
        use wasmtime::component::StreamResult;

        let remaining = destination.remaining(store);
        if remaining.is_some_and(|r| r == 0) {
            return Poll::Ready(Ok(StreamResult::Completed));
        }

        let recv = self.get_mut().rx.poll_recv(cx);
        match recv {
            Poll::Ready(None) => Poll::Ready(Ok(StreamResult::Dropped)),
            Poll::Pending => {
                if finish {
                    Poll::Ready(Ok(StreamResult::Cancelled))
                } else {
                    Poll::Pending
                }
            }
            Poll::Ready(Some(row)) => {
                destination.set_buffer(Some(row));
                Poll::Ready(Ok(StreamResult::Completed))
            }
        }
    }
}
