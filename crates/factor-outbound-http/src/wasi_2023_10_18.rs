use anyhow::Result;
use wasmtime::component::{Linker, Resource};
use wasmtime_wasi_http::bindings as latest;
use wasmtime_wasi_http::{WasiHttpImpl, WasiHttpView};

mod bindings {
    use super::latest;

    wasmtime::component::bindgen!({
        path: "../../wit",
        world: "wasi:http/proxy@0.2.0-rc-2023-10-18",
        async: {
            // Only need async exports
            only_imports: [],
        },
        with: {
            "wasi:io/poll/pollable": latest::io::poll::Pollable,
            "wasi:io/streams/input-stream": latest::io::streams::InputStream,
            "wasi:io/streams/output-stream": latest::io::streams::OutputStream,
            "wasi:io/streams/error": latest::io::streams::Error,
            "wasi:http/types/incoming-response": latest::http::types::IncomingResponse,
            "wasi:http/types/incoming-request": latest::http::types::IncomingRequest,
            "wasi:http/types/incoming-body": latest::http::types::IncomingBody,
            "wasi:http/types/outgoing-response": latest::http::types::OutgoingResponse,
            "wasi:http/types/outgoing-request": latest::http::types::OutgoingRequest,
            "wasi:http/types/outgoing-body": latest::http::types::OutgoingBody,
            "wasi:http/types/fields": latest::http::types::Fields,
            "wasi:http/types/response-outparam": latest::http::types::ResponseOutparam,
            "wasi:http/types/future-incoming-response": latest::http::types::FutureIncomingResponse,
            "wasi:http/types/future-trailers": latest::http::types::FutureTrailers,
        },
        trappable_imports: true,
    });
}

mod wasi {
    pub use super::bindings::wasi::{http0_2_0_rc_2023_10_18 as http, io0_2_0_rc_2023_10_18 as io};
}

pub mod exports {
    pub mod wasi {
        pub use super::super::bindings::exports::wasi::http0_2_0_rc_2023_10_18 as http;
    }
}

pub use bindings::{Proxy, ProxyIndices};
use wasi::http::types::{
    Error as HttpError, Fields, FutureIncomingResponse, FutureTrailers, Headers, IncomingBody,
    IncomingRequest, IncomingResponse, Method, OutgoingBody, OutgoingRequest, OutgoingResponse,
    RequestOptions, ResponseOutparam, Scheme, StatusCode, Trailers,
};
use wasi::io::poll::Pollable;
use wasi::io::streams::{InputStream, OutputStream};

use crate::wasi::{HasHttp, WasiHttpImplInner};

pub(crate) fn add_to_linker<T>(
    linker: &mut Linker<T>,
    closure: fn(&mut T) -> WasiHttpImpl<WasiHttpImplInner<'_>>,
) -> Result<()>
where
    T: Send + 'static,
{
    wasi::http::types::add_to_linker::<_, HasHttp>(linker, closure)?;
    wasi::http::outgoing_handler::add_to_linker::<_, HasHttp>(linker, closure)?;
    Ok(())
}

impl<T> wasi::http::types::Host for WasiHttpImpl<T> where T: WasiHttpView + Send {}

impl<T> wasi::http::types::HostFields for WasiHttpImpl<T>
where
    T: WasiHttpView + Send,
{
    fn new(
        &mut self,
        entries: Vec<(String, Vec<u8>)>,
    ) -> wasmtime::Result<wasmtime::component::Resource<Fields>> {
        match latest::http::types::HostFields::from_list(self, entries)? {
            Ok(fields) => Ok(fields),
            Err(e) => Err(e.into()),
        }
    }

    fn get(
        &mut self,
        self_: wasmtime::component::Resource<Fields>,
        name: String,
    ) -> wasmtime::Result<Vec<Vec<u8>>> {
        latest::http::types::HostFields::get(self, self_, name)
    }

    fn set(
        &mut self,
        self_: wasmtime::component::Resource<Fields>,
        name: String,
        value: Vec<Vec<u8>>,
    ) -> wasmtime::Result<()> {
        latest::http::types::HostFields::set(self, self_, name, value)??;
        Ok(())
    }

    fn delete(
        &mut self,
        self_: wasmtime::component::Resource<Fields>,
        name: String,
    ) -> wasmtime::Result<()> {
        latest::http::types::HostFields::delete(self, self_, name)??;
        Ok(())
    }

    fn append(
        &mut self,
        self_: wasmtime::component::Resource<Fields>,
        name: String,
        value: Vec<u8>,
    ) -> wasmtime::Result<()> {
        latest::http::types::HostFields::append(self, self_, name, value)??;
        Ok(())
    }

    fn entries(
        &mut self,
        self_: wasmtime::component::Resource<Fields>,
    ) -> wasmtime::Result<Vec<(String, Vec<u8>)>> {
        latest::http::types::HostFields::entries(self, self_)
    }

    fn clone(
        &mut self,
        self_: wasmtime::component::Resource<Fields>,
    ) -> wasmtime::Result<wasmtime::component::Resource<Fields>> {
        latest::http::types::HostFields::clone(self, self_)
    }

    fn drop(&mut self, rep: wasmtime::component::Resource<Fields>) -> wasmtime::Result<()> {
        latest::http::types::HostFields::drop(self, rep)
    }
}

impl<T> wasi::http::types::HostIncomingRequest for WasiHttpImpl<T>
where
    T: WasiHttpView + Send,
{
    fn method(
        &mut self,
        self_: wasmtime::component::Resource<IncomingRequest>,
    ) -> wasmtime::Result<Method> {
        latest::http::types::HostIncomingRequest::method(self, self_).map(|e| e.into())
    }

    fn path_with_query(
        &mut self,
        self_: wasmtime::component::Resource<IncomingRequest>,
    ) -> wasmtime::Result<Option<String>> {
        latest::http::types::HostIncomingRequest::path_with_query(self, self_)
    }

    fn scheme(
        &mut self,
        self_: wasmtime::component::Resource<IncomingRequest>,
    ) -> wasmtime::Result<Option<Scheme>> {
        latest::http::types::HostIncomingRequest::scheme(self, self_).map(|e| e.map(|e| e.into()))
    }

    fn authority(
        &mut self,
        self_: wasmtime::component::Resource<IncomingRequest>,
    ) -> wasmtime::Result<Option<String>> {
        latest::http::types::HostIncomingRequest::authority(self, self_)
    }

    fn headers(
        &mut self,
        self_: wasmtime::component::Resource<IncomingRequest>,
    ) -> wasmtime::Result<wasmtime::component::Resource<Headers>> {
        latest::http::types::HostIncomingRequest::headers(self, self_)
    }

    fn consume(
        &mut self,
        self_: wasmtime::component::Resource<IncomingRequest>,
    ) -> wasmtime::Result<Result<wasmtime::component::Resource<IncomingBody>, ()>> {
        latest::http::types::HostIncomingRequest::consume(self, self_)
    }

    fn drop(
        &mut self,
        rep: wasmtime::component::Resource<IncomingRequest>,
    ) -> wasmtime::Result<()> {
        latest::http::types::HostIncomingRequest::drop(self, rep)
    }
}

impl<T> wasi::http::types::HostIncomingResponse for WasiHttpImpl<T>
where
    T: WasiHttpView + Send,
{
    fn status(
        &mut self,
        self_: wasmtime::component::Resource<IncomingResponse>,
    ) -> wasmtime::Result<StatusCode> {
        latest::http::types::HostIncomingResponse::status(self, self_)
    }

    fn headers(
        &mut self,
        self_: wasmtime::component::Resource<IncomingResponse>,
    ) -> wasmtime::Result<wasmtime::component::Resource<Headers>> {
        latest::http::types::HostIncomingResponse::headers(self, self_)
    }

    fn consume(
        &mut self,
        self_: wasmtime::component::Resource<IncomingResponse>,
    ) -> wasmtime::Result<Result<wasmtime::component::Resource<IncomingBody>, ()>> {
        latest::http::types::HostIncomingResponse::consume(self, self_)
    }

    fn drop(
        &mut self,
        rep: wasmtime::component::Resource<IncomingResponse>,
    ) -> wasmtime::Result<()> {
        latest::http::types::HostIncomingResponse::drop(self, rep)
    }
}

impl<T> wasi::http::types::HostIncomingBody for WasiHttpImpl<T>
where
    T: WasiHttpView + Send,
{
    fn stream(
        &mut self,
        self_: wasmtime::component::Resource<IncomingBody>,
    ) -> wasmtime::Result<Result<wasmtime::component::Resource<InputStream>, ()>> {
        latest::http::types::HostIncomingBody::stream(self, self_)
    }

    fn finish(
        &mut self,
        this: wasmtime::component::Resource<IncomingBody>,
    ) -> wasmtime::Result<wasmtime::component::Resource<FutureTrailers>> {
        latest::http::types::HostIncomingBody::finish(self, this)
    }

    fn drop(&mut self, rep: wasmtime::component::Resource<IncomingBody>) -> wasmtime::Result<()> {
        latest::http::types::HostIncomingBody::drop(self, rep)
    }
}

impl<T> wasi::http::types::HostOutgoingRequest for WasiHttpImpl<T>
where
    T: WasiHttpView + Send,
{
    fn new(
        &mut self,
        method: Method,
        path_with_query: Option<String>,
        scheme: Option<Scheme>,
        authority: Option<String>,
        headers: wasmtime::component::Resource<Headers>,
    ) -> wasmtime::Result<wasmtime::component::Resource<OutgoingRequest>> {
        let headers = latest::http::types::HostFields::clone(self, headers)?;
        let request = latest::http::types::HostOutgoingRequest::new(self, headers)?;
        let borrow = || Resource::new_borrow(request.rep());

        if let Err(()) =
            latest::http::types::HostOutgoingRequest::set_method(self, borrow(), method.into())?
        {
            latest::http::types::HostOutgoingRequest::drop(self, request)?;
            anyhow::bail!("invalid method supplied");
        }

        if let Err(()) = latest::http::types::HostOutgoingRequest::set_path_with_query(
            self,
            borrow(),
            path_with_query,
        )? {
            latest::http::types::HostOutgoingRequest::drop(self, request)?;
            anyhow::bail!("invalid path-with-query supplied");
        }

        // Historical WASI would fill in an empty authority with a port which
        // got just enough working to get things through. Current WASI requires
        // the authority, though, so perform the translation manually here.
        let authority = authority.unwrap_or_else(|| match &scheme {
            Some(Scheme::Http) | Some(Scheme::Other(_)) => ":80".to_string(),
            Some(Scheme::Https) | None => ":443".to_string(),
        });
        if let Err(()) = latest::http::types::HostOutgoingRequest::set_scheme(
            self,
            borrow(),
            scheme.map(|s| s.into()),
        )? {
            latest::http::types::HostOutgoingRequest::drop(self, request)?;
            anyhow::bail!("invalid scheme supplied");
        }

        if let Err(()) = latest::http::types::HostOutgoingRequest::set_authority(
            self,
            borrow(),
            Some(authority),
        )? {
            latest::http::types::HostOutgoingRequest::drop(self, request)?;
            anyhow::bail!("invalid authority supplied");
        }

        Ok(request)
    }

    fn write(
        &mut self,
        self_: wasmtime::component::Resource<OutgoingRequest>,
    ) -> wasmtime::Result<Result<wasmtime::component::Resource<OutgoingBody>, ()>> {
        latest::http::types::HostOutgoingRequest::body(self, self_)
    }

    fn drop(
        &mut self,
        rep: wasmtime::component::Resource<OutgoingRequest>,
    ) -> wasmtime::Result<()> {
        latest::http::types::HostOutgoingRequest::drop(self, rep)
    }
}

impl<T> wasi::http::types::HostOutgoingResponse for WasiHttpImpl<T>
where
    T: WasiHttpView + Send,
{
    fn new(
        &mut self,
        status_code: StatusCode,
        headers: wasmtime::component::Resource<Headers>,
    ) -> wasmtime::Result<wasmtime::component::Resource<OutgoingResponse>> {
        let headers = latest::http::types::HostFields::clone(self, headers)?;
        let response = latest::http::types::HostOutgoingResponse::new(self, headers)?;
        let borrow = || Resource::new_borrow(response.rep());

        if let Err(()) =
            latest::http::types::HostOutgoingResponse::set_status_code(self, borrow(), status_code)?
        {
            latest::http::types::HostOutgoingResponse::drop(self, response)?;
            anyhow::bail!("invalid status code supplied");
        }

        Ok(response)
    }

    fn write(
        &mut self,
        self_: wasmtime::component::Resource<OutgoingResponse>,
    ) -> wasmtime::Result<Result<wasmtime::component::Resource<OutgoingBody>, ()>> {
        latest::http::types::HostOutgoingResponse::body(self, self_)
    }

    fn drop(
        &mut self,
        rep: wasmtime::component::Resource<OutgoingResponse>,
    ) -> wasmtime::Result<()> {
        latest::http::types::HostOutgoingResponse::drop(self, rep)
    }
}

impl<T> wasi::http::types::HostOutgoingBody for WasiHttpImpl<T>
where
    T: WasiHttpView + Send,
{
    fn write(
        &mut self,
        self_: wasmtime::component::Resource<OutgoingBody>,
    ) -> wasmtime::Result<Result<wasmtime::component::Resource<OutputStream>, ()>> {
        latest::http::types::HostOutgoingBody::write(self, self_)
    }

    fn finish(
        &mut self,
        this: wasmtime::component::Resource<OutgoingBody>,
        trailers: Option<wasmtime::component::Resource<Trailers>>,
    ) -> wasmtime::Result<()> {
        latest::http::types::HostOutgoingBody::finish(self, this, trailers)?;
        Ok(())
    }

    fn drop(&mut self, rep: wasmtime::component::Resource<OutgoingBody>) -> wasmtime::Result<()> {
        latest::http::types::HostOutgoingBody::drop(self, rep)
    }
}

impl<T> wasi::http::types::HostResponseOutparam for WasiHttpImpl<T>
where
    T: WasiHttpView + Send,
{
    fn set(
        &mut self,
        param: wasmtime::component::Resource<ResponseOutparam>,
        response: Result<wasmtime::component::Resource<OutgoingResponse>, HttpError>,
    ) -> wasmtime::Result<()> {
        let response = response.map_err(|err| {
            // TODO: probably need to figure out a better mapping between
            // errors, but that seems like it would require string matching,
            // which also seems not great.
            let msg = match err {
                HttpError::InvalidUrl(s) => format!("invalid url: {s}"),
                HttpError::TimeoutError(s) => format!("timeout: {s}"),
                HttpError::ProtocolError(s) => format!("protocol error: {s}"),
                HttpError::UnexpectedError(s) => format!("unexpected error: {s}"),
            };
            latest::http::types::ErrorCode::InternalError(Some(msg))
        });
        latest::http::types::HostResponseOutparam::set(self, param, response)
    }

    fn drop(
        &mut self,
        rep: wasmtime::component::Resource<ResponseOutparam>,
    ) -> wasmtime::Result<()> {
        latest::http::types::HostResponseOutparam::drop(self, rep)
    }
}

impl<T> wasi::http::types::HostFutureTrailers for WasiHttpImpl<T>
where
    T: WasiHttpView + Send,
{
    fn subscribe(
        &mut self,
        self_: wasmtime::component::Resource<FutureTrailers>,
    ) -> wasmtime::Result<wasmtime::component::Resource<Pollable>> {
        latest::http::types::HostFutureTrailers::subscribe(self, self_)
    }

    fn get(
        &mut self,
        self_: wasmtime::component::Resource<FutureTrailers>,
    ) -> wasmtime::Result<Option<Result<wasmtime::component::Resource<Trailers>, HttpError>>> {
        match latest::http::types::HostFutureTrailers::get(self, self_)? {
            Some(Ok(Ok(Some(trailers)))) => Ok(Some(Ok(trailers))),
            // Return an empty trailers if no trailers popped out since this
            // version of WASI couldn't represent the lack of trailers.
            Some(Ok(Ok(None))) => Ok(Some(Ok(latest::http::types::HostFields::new(self)?))),
            Some(Ok(Err(e))) => Ok(Some(Err(e.into()))),
            Some(Err(())) => Err(anyhow::anyhow!("trailers have already been retrieved")),
            None => Ok(None),
        }
    }

    fn drop(&mut self, rep: wasmtime::component::Resource<FutureTrailers>) -> wasmtime::Result<()> {
        latest::http::types::HostFutureTrailers::drop(self, rep)
    }
}

impl<T> wasi::http::types::HostFutureIncomingResponse for WasiHttpImpl<T>
where
    T: WasiHttpView + Send,
{
    fn get(
        &mut self,
        self_: wasmtime::component::Resource<FutureIncomingResponse>,
    ) -> wasmtime::Result<
        Option<Result<Result<wasmtime::component::Resource<IncomingResponse>, HttpError>, ()>>,
    > {
        match latest::http::types::HostFutureIncomingResponse::get(self, self_)? {
            None => Ok(None),
            Some(Ok(Ok(response))) => Ok(Some(Ok(Ok(response)))),
            Some(Ok(Err(e))) => Ok(Some(Ok(Err(e.into())))),
            Some(Err(())) => Ok(Some(Err(()))),
        }
    }

    fn subscribe(
        &mut self,
        self_: wasmtime::component::Resource<FutureIncomingResponse>,
    ) -> wasmtime::Result<wasmtime::component::Resource<Pollable>> {
        latest::http::types::HostFutureIncomingResponse::subscribe(self, self_)
    }

    fn drop(
        &mut self,
        rep: wasmtime::component::Resource<FutureIncomingResponse>,
    ) -> wasmtime::Result<()> {
        latest::http::types::HostFutureIncomingResponse::drop(self, rep)
    }
}

impl<T> wasi::http::outgoing_handler::Host for WasiHttpImpl<T>
where
    T: WasiHttpView + Send,
{
    fn handle(
        &mut self,
        request: wasmtime::component::Resource<OutgoingRequest>,
        options: Option<RequestOptions>,
    ) -> wasmtime::Result<Result<wasmtime::component::Resource<FutureIncomingResponse>, HttpError>>
    {
        let options = match options {
            Some(RequestOptions {
                connect_timeout_ms,
                first_byte_timeout_ms,
                between_bytes_timeout_ms,
            }) => {
                let options = latest::http::types::HostRequestOptions::new(self)?;
                let borrow = || Resource::new_borrow(request.rep());

                if let Some(ms) = connect_timeout_ms {
                    if let Err(()) = latest::http::types::HostRequestOptions::set_connect_timeout(
                        self,
                        borrow(),
                        Some(ms.into()),
                    )? {
                        latest::http::types::HostRequestOptions::drop(self, options)?;
                        anyhow::bail!("invalid connect timeout supplied");
                    }
                }

                if let Some(ms) = first_byte_timeout_ms {
                    if let Err(()) =
                        latest::http::types::HostRequestOptions::set_first_byte_timeout(
                            self,
                            borrow(),
                            Some(ms.into()),
                        )?
                    {
                        latest::http::types::HostRequestOptions::drop(self, options)?;
                        anyhow::bail!("invalid first byte timeout supplied");
                    }
                }

                if let Some(ms) = between_bytes_timeout_ms {
                    if let Err(()) =
                        latest::http::types::HostRequestOptions::set_between_bytes_timeout(
                            self,
                            borrow(),
                            Some(ms.into()),
                        )?
                    {
                        latest::http::types::HostRequestOptions::drop(self, options)?;
                        anyhow::bail!("invalid between bytes timeout supplied");
                    }
                }

                Some(options)
            }
            None => None,
        };
        match latest::http::outgoing_handler::Host::handle(self, request, options) {
            Ok(resp) => Ok(Ok(resp)),
            Err(e) => Ok(Err(e.downcast()?.into())),
        }
    }
}

macro_rules! convert {
    () => {};
    ($kind:ident $from:path [<=>] $to:path { $($body:tt)* } $($rest:tt)*) => {
        convert!($kind $from => $to { $($body)* });
        convert!($kind $to => $from { $($body)* });

        convert!($($rest)*);
    };
    (struct $from:ty => $to:path { $($field:ident,)* } $($rest:tt)*) => {
        impl From<$from> for $to {
            fn from(e: $from) -> $to {
                $to {
                    $( $field: e.$field.into(), )*
                }
            }
        }

        convert!($($rest)*);
    };
    (enum $from:path => $to:path { $($variant:ident $(($e:ident))?,)* } $($rest:tt)*) => {
        impl From<$from> for $to {
            fn from(e: $from) -> $to {
                use $from as A;
                use $to as B;
                match e {
                    $(
                        A::$variant $(($e))? => B::$variant $(($e.into()))?,
                    )*
                }
            }
        }

        convert!($($rest)*);
    };
    (flags $from:path => $to:path { $($flag:ident,)* } $($rest:tt)*) => {
        impl From<$from> for $to {
            fn from(e: $from) -> $to {
                use $from as A;
                use $to as B;
                let mut out = B::empty();
                $(
                    if e.contains(A::$flag) {
                        out |= B::$flag;
                    }
                )*
                out
            }
        }

        convert!($($rest)*);
    };
}

pub(crate) use convert;

convert! {
    enum latest::http::types::Method [<=>] Method {
        Get,
        Head,
        Post,
        Put,
        Delete,
        Connect,
        Options,
        Trace,
        Patch,
        Other(e),
    }

    enum latest::http::types::Scheme [<=>] Scheme {
        Http,
        Https,
        Other(e),
    }
}

impl From<latest::http::types::ErrorCode> for HttpError {
    fn from(e: latest::http::types::ErrorCode) -> HttpError {
        // TODO: should probably categorize this better given the typed info
        // we have in `e`.
        HttpError::UnexpectedError(e.to_string())
    }
}
