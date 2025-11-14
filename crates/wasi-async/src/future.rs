use spin_core::wasmtime;

pub fn producer<T: 'static + Send>(rx: tokio::sync::oneshot::Receiver<T>) -> FutureProducer<T> {
    FutureProducer { rx }
}

pub struct FutureProducer<T> {
    rx: tokio::sync::oneshot::Receiver<T>,
}

impl<D, T: 'static + Send> wasmtime::component::FutureProducer<D> for FutureProducer<T> {
    type Item = T;

    fn poll_produce(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        _store: wasmtime::StoreContextMut<D>,
        _finish: bool,
    ) -> std::task::Poll<anyhow::Result<Option<Self::Item>>> {
        use std::future::Future;
        use std::task::Poll;

        let pinned_rx = std::pin::Pin::new(&mut self.get_mut().rx);
        match pinned_rx.poll(cx) {
            Poll::Ready(Err(e)) => Poll::Ready(Err(anyhow::anyhow!("{e:#}"))),
            Poll::Ready(Ok(cols)) => Poll::Ready(Ok(Some(cols))),
            Poll::Pending => Poll::Pending,
        }
    }
}
