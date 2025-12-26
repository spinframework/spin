use std::net::SocketAddr;

use http::{Request, Response};
use http_body_util::{BodyExt, Full};
use spin_world::async_trait;
use wasmtime_wasi_http::{body::HyperOutgoingBody, HttpResult};

pub type HyperBody = HyperOutgoingBody;

/// An outbound HTTP request interceptor to be used with
/// [`super::InstanceState::set_request_interceptor`].
#[async_trait]
pub trait OutboundHttpInterceptor: Send + Sync {
    /// Intercept an outgoing HTTP request.
    ///
    /// If this method returns [`InterceptOutcome::Continue`], the (possibly
    /// updated) request will be passed on to the default outgoing request
    /// handler.
    ///
    /// If this method returns [`InterceptOutcome::Complete`], the inner result
    /// will be returned as the result of the request, bypassing the default
    /// handler. The `request` will also be dropped immediately.
    async fn intercept(&self, request: InterceptRequest) -> HttpResult<InterceptOutcome>;
}

/// The type returned by an [`OutboundHttpInterceptor`].
pub enum InterceptOutcome {
    /// The intercepted request will be passed on to the default outgoing
    /// request handler.
    Continue(InterceptRequest),
    /// The given response will be returned as the result of the intercepted
    /// request, bypassing the default handler.
    Complete(Response<HyperBody>),
}

/// An intercepted outgoing HTTP request.
///
/// This is a wrapper that implements `DerefMut<Target = Request<()>>` for
/// inspection and modification of the request envelope. If the body needs to be
/// consumed, call [`Self::into_hyper_request`].
pub struct InterceptRequest {
    inner: Request<()>,
    body: InterceptBody,
    pub(crate) override_connect_addr: Option<SocketAddr>,
}

enum InterceptBody {
    Hyper(HyperBody),
    Vec(Vec<u8>),
}

impl InterceptRequest {
    /// Overrides the IP and port that will be connected to for this outbound
    /// request.
    ///
    /// This override does not have any effect on TLS server name checking or
    /// HTTP authority / host headers.
    ///
    /// The IP will be checked against blocked IP networks but it will not be
    /// checked against `allowed_outbound_hosts`; if that check needs to occur
    /// it must be performed by the interceptor.
    pub fn override_connect_addr(&mut self, endpoint: SocketAddr) {
        self.override_connect_addr = Some(endpoint);
    }

    pub fn into_hyper_request(self) -> Request<HyperBody> {
        let (parts, ()) = self.inner.into_parts();
        Request::from_parts(parts, self.body.into())
    }

    pub(crate) fn into_vec_request(self) -> Option<Request<Vec<u8>>> {
        let InterceptBody::Vec(bytes) = self.body else {
            return None;
        };
        let (parts, ()) = self.inner.into_parts();
        Some(Request::from_parts(parts, bytes))
    }
}

impl std::ops::Deref for InterceptRequest {
    type Target = Request<()>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl std::ops::DerefMut for InterceptRequest {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl From<Request<HyperBody>> for InterceptRequest {
    fn from(req: Request<HyperBody>) -> Self {
        let (parts, body) = req.into_parts();
        Self {
            inner: Request::from_parts(parts, ()),
            body: InterceptBody::Hyper(body),
            override_connect_addr: None,
        }
    }
}

impl From<Request<Vec<u8>>> for InterceptRequest {
    fn from(req: Request<Vec<u8>>) -> Self {
        let (parts, body) = req.into_parts();
        Self {
            inner: Request::from_parts(parts, ()),
            body: InterceptBody::Vec(body),
            override_connect_addr: None,
        }
    }
}

impl From<InterceptBody> for HyperBody {
    fn from(body: InterceptBody) -> Self {
        match body {
            InterceptBody::Hyper(body) => body,
            InterceptBody::Vec(bytes) => Full::new(bytes.into())
                .map_err(|err| match err {})
                .boxed_unsync(),
        }
    }
}
