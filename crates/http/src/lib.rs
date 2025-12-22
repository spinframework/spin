#[cfg(feature = "runtime")]
pub use wasmtime_wasi_http::body::HyperIncomingBody as Body;

pub mod app_info;
pub mod config;
pub mod trigger;
#[cfg(feature = "runtime")]
pub mod wagi;

pub use spin_http_routes as routes;
pub use spin_http_routes::WELL_KNOWN_PREFIX;

#[cfg(feature = "runtime")]
pub mod body {
    use super::Body;
    use http_body_util::{combinators::UnsyncBoxBody, BodyExt, Empty, Full};
    use hyper::body::Bytes;

    pub fn full(bytes: Bytes) -> Body {
        UnsyncBoxBody::new(Full::new(bytes).map_err(|_| unreachable!()))
    }

    pub fn empty() -> Body {
        UnsyncBoxBody::new(Empty::new().map_err(|_| unreachable!()))
    }
}
