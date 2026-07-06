use spin_factor_wasi::{FutureReaderExt as _, convert, convert_result, reborrow};
use wasmtime::AsContextMut;
use wasmtime::component::{Access, Accessor, FutureReader, Linker, Resource, StreamReader};
use wasmtime_wasi_http::p3::{WasiHttp, WasiHttpCtxView, bindings as latest};

mod bindings {
    use super::latest;

    wasmtime::component::bindgen!({
        path: "../../wit",
        world: "wasi:http/service@0.3.0-rc-2026-03-15",
        imports: {
            "wasi:http/client.send": store | trappable,
            "wasi:http/types.[drop]request": store | trappable,
            "wasi:http/types.[drop]response": store | trappable,
            "wasi:http/types.[static]request.consume-body": store | trappable,
            "wasi:http/types.[static]request.new": store | trappable,
            "wasi:http/types.[static]response.consume-body": store | trappable,
            "wasi:http/types.[static]response.new": store | trappable,
            default: trappable,
        },
        exports: { default: async | store },
        with: {
            "wasi:http/types.fields": latest::http::types::Fields,
            "wasi:http/types.request": latest::http::types::Request,
            "wasi:http/types.request-options": latest::http::types::RequestOptions,
            "wasi:http/types.response": latest::http::types::Response,
        },
    });
}

pub use bindings::{Service, ServiceIndices};

mod wasi {
    pub use super::bindings::wasi::http0_3_0_rc_2026_03_15 as http;
}

pub(crate) fn add_to_linker<T>(
    linker: &mut Linker<T>,
    closure: fn(&mut T) -> WasiHttpCtxView<'_>,
) -> anyhow::Result<()>
where
    T: Send + 'static,
{
    wasi::http::types::add_to_linker::<_, WasiHttp>(linker, closure)?;
    wasi::http::client::add_to_linker::<_, WasiHttp>(linker, closure)?;
    Ok(())
}

impl wasi::http::types::Host for WasiHttpCtxView<'_> {}

impl wasi::http::types::HostResponse for WasiHttpCtxView<'_> {
    fn get_status_code(
        &mut self,
        res: Resource<wasi::http::types::Response>,
    ) -> wasmtime::Result<wasi::http::types::StatusCode> {
        latest::http::types::HostResponse::get_status_code(self, res)
    }

    fn set_status_code(
        &mut self,
        res: Resource<wasi::http::types::Response>,
        status_code: wasi::http::types::StatusCode,
    ) -> wasmtime::Result<Result<(), ()>> {
        latest::http::types::HostResponse::set_status_code(self, res, status_code)
    }

    fn get_headers(
        &mut self,
        res: Resource<wasi::http::types::Response>,
    ) -> wasmtime::Result<Resource<wasi::http::types::Headers>> {
        latest::http::types::HostResponse::get_headers(self, res)
    }
}

impl wasi::http::types::HostRequestOptions for WasiHttpCtxView<'_> {
    fn new(&mut self) -> wasmtime::Result<Resource<wasi::http::types::RequestOptions>> {
        latest::http::types::HostRequestOptions::new(self)
    }

    fn get_connect_timeout(
        &mut self,
        opts: Resource<wasi::http::types::RequestOptions>,
    ) -> wasmtime::Result<Option<wasi::http::types::Duration>> {
        latest::http::types::HostRequestOptions::get_connect_timeout(self, opts)
    }

    fn set_connect_timeout(
        &mut self,
        opts: Resource<wasi::http::types::RequestOptions>,
        duration: Option<wasi::http::types::Duration>,
    ) -> wasmtime::Result<Result<(), wasi::http::types::RequestOptionsError>> {
        convert_result(
            latest::http::types::HostRequestOptions::set_connect_timeout(self, opts, duration),
        )
    }

    fn get_first_byte_timeout(
        &mut self,
        opts: Resource<wasi::http::types::RequestOptions>,
    ) -> wasmtime::Result<Option<wasi::http::types::Duration>> {
        latest::http::types::HostRequestOptions::get_first_byte_timeout(self, opts)
    }

    fn set_first_byte_timeout(
        &mut self,
        opts: Resource<wasi::http::types::RequestOptions>,
        duration: Option<wasi::http::types::Duration>,
    ) -> wasmtime::Result<Result<(), wasi::http::types::RequestOptionsError>> {
        convert_result(
            latest::http::types::HostRequestOptions::set_first_byte_timeout(self, opts, duration),
        )
    }

    fn get_between_bytes_timeout(
        &mut self,
        opts: Resource<wasi::http::types::RequestOptions>,
    ) -> wasmtime::Result<Option<wasi::http::types::Duration>> {
        latest::http::types::HostRequestOptions::get_between_bytes_timeout(self, opts)
    }

    fn set_between_bytes_timeout(
        &mut self,
        opts: Resource<wasi::http::types::RequestOptions>,
        duration: Option<wasi::http::types::Duration>,
    ) -> wasmtime::Result<Result<(), wasi::http::types::RequestOptionsError>> {
        convert_result(
            latest::http::types::HostRequestOptions::set_between_bytes_timeout(
                self, opts, duration,
            ),
        )
    }

    fn clone(
        &mut self,
        opts: Resource<wasi::http::types::RequestOptions>,
    ) -> wasmtime::Result<Resource<wasi::http::types::RequestOptions>> {
        latest::http::types::HostRequestOptions::clone(self, opts)
    }

    fn drop(&mut self, opts: Resource<wasi::http::types::RequestOptions>) -> wasmtime::Result<()> {
        latest::http::types::HostRequestOptions::drop(self, opts)
    }
}

impl<T> wasi::http::types::HostRequestWithStore<T> for WasiHttp {
    fn new(
        mut store: Access<T, Self>,
        headers: Resource<wasi::http::types::Headers>,
        contents: Option<StreamReader<u8>>,
        trailers: FutureReader<
            Result<Option<Resource<wasi::http::types::Trailers>>, wasi::http::types::ErrorCode>,
        >,
        options: Option<Resource<wasi::http::types::RequestOptions>>,
    ) -> wasmtime::Result<(
        Resource<wasi::http::types::Request>,
        FutureReader<Result<(), wasi::http::types::ErrorCode>>,
    )> {
        let trailers = trailers.try_map(store.as_context_mut(), |v| v.map_err(|v| v.into()))?;
        latest::http::types::HostRequestWithStore::new(
            reborrow(&mut store),
            headers,
            contents,
            trailers,
            options,
        )
        .and_then(|(req, future)| Ok((req, future.try_map(store, |v| v.map_err(|v| v.into()))?)))
    }

    fn consume_body(
        mut store: Access<T, Self>,
        req: Resource<wasi::http::types::Request>,
        fut: FutureReader<Result<(), wasi::http::types::ErrorCode>>,
    ) -> wasmtime::Result<(
        StreamReader<u8>,
        FutureReader<
            Result<Option<Resource<wasi::http::types::Trailers>>, wasi::http::types::ErrorCode>,
        >,
    )> {
        let fut = fut.try_map(store.as_context_mut(), |v| v.map_err(|v| v.into()))?;
        latest::http::types::HostRequestWithStore::consume_body(reborrow(&mut store), req, fut)
            .and_then(|(stream, future)| {
                Ok((stream, future.try_map(store, |v| v.map_err(|v| v.into()))?))
            })
    }

    fn drop(
        store: Access<'_, T, Self>,
        req: Resource<wasi::http::types::Request>,
    ) -> wasmtime::Result<()> {
        latest::http::types::HostRequestWithStore::drop(store, req)
    }
}

impl wasi::http::types::HostRequest for WasiHttpCtxView<'_> {
    fn get_method(
        &mut self,
        req: Resource<wasi::http::types::Request>,
    ) -> wasmtime::Result<wasi::http::types::Method> {
        latest::http::types::HostRequest::get_method(self, req).map(|v| v.into())
    }

    fn set_method(
        &mut self,
        req: Resource<wasi::http::types::Request>,
        method: wasi::http::types::Method,
    ) -> wasmtime::Result<Result<(), ()>> {
        latest::http::types::HostRequest::set_method(self, req, method.into())
    }

    fn get_path_with_query(
        &mut self,
        req: Resource<wasi::http::types::Request>,
    ) -> wasmtime::Result<Option<String>> {
        latest::http::types::HostRequest::get_path_with_query(self, req)
    }

    fn set_path_with_query(
        &mut self,
        req: Resource<wasi::http::types::Request>,
        path_with_query: Option<String>,
    ) -> wasmtime::Result<Result<(), ()>> {
        latest::http::types::HostRequest::set_path_with_query(self, req, path_with_query)
    }

    fn get_scheme(
        &mut self,
        req: Resource<wasi::http::types::Request>,
    ) -> wasmtime::Result<Option<wasi::http::types::Scheme>> {
        latest::http::types::HostRequest::get_scheme(self, req).map(|v| v.map(|v| v.into()))
    }

    fn set_scheme(
        &mut self,
        req: Resource<wasi::http::types::Request>,
        scheme: Option<wasi::http::types::Scheme>,
    ) -> wasmtime::Result<Result<(), ()>> {
        latest::http::types::HostRequest::set_scheme(self, req, scheme.map(|v| v.into()))
    }

    fn get_authority(
        &mut self,
        req: Resource<wasi::http::types::Request>,
    ) -> wasmtime::Result<Option<String>> {
        latest::http::types::HostRequest::get_authority(self, req)
    }

    fn set_authority(
        &mut self,
        req: Resource<wasi::http::types::Request>,
        authority: Option<String>,
    ) -> wasmtime::Result<Result<(), ()>> {
        latest::http::types::HostRequest::set_authority(self, req, authority)
    }

    fn get_options(
        &mut self,
        req: Resource<wasi::http::types::Request>,
    ) -> wasmtime::Result<Option<Resource<wasi::http::types::RequestOptions>>> {
        latest::http::types::HostRequest::get_options(self, req)
    }

    fn get_headers(
        &mut self,
        req: Resource<wasi::http::types::Request>,
    ) -> wasmtime::Result<Resource<wasi::http::types::Headers>> {
        latest::http::types::HostRequest::get_headers(self, req)
    }
}

impl wasi::http::types::HostFields for WasiHttpCtxView<'_> {
    fn new(&mut self) -> wasmtime::Result<Resource<wasi::http::types::Fields>> {
        latest::http::types::HostFields::new(self)
    }

    fn from_list(
        &mut self,
        entries: Vec<(wasi::http::types::FieldName, wasi::http::types::FieldValue)>,
    ) -> wasmtime::Result<Result<Resource<wasi::http::types::Fields>, wasi::http::types::HeaderError>>
    {
        convert_result(latest::http::types::HostFields::from_list(self, entries))
    }

    fn get(
        &mut self,
        fields: Resource<wasi::http::types::Fields>,
        name: wasi::http::types::FieldName,
    ) -> wasmtime::Result<Vec<wasi::http::types::FieldValue>> {
        latest::http::types::HostFields::get(self, fields, name)
    }

    fn has(
        &mut self,
        fields: Resource<wasi::http::types::Fields>,
        name: wasi::http::types::FieldName,
    ) -> wasmtime::Result<bool> {
        latest::http::types::HostFields::has(self, fields, name)
    }

    fn set(
        &mut self,
        fields: Resource<wasi::http::types::Fields>,
        name: wasi::http::types::FieldName,
        value: Vec<wasi::http::types::FieldValue>,
    ) -> wasmtime::Result<Result<(), wasi::http::types::HeaderError>> {
        convert_result(latest::http::types::HostFields::set(
            self, fields, name, value,
        ))
    }

    fn delete(
        &mut self,
        fields: Resource<wasi::http::types::Fields>,
        name: wasi::http::types::FieldName,
    ) -> wasmtime::Result<Result<(), wasi::http::types::HeaderError>> {
        convert_result(latest::http::types::HostFields::delete(self, fields, name))
    }

    fn get_and_delete(
        &mut self,
        fields: Resource<wasi::http::types::Fields>,
        name: wasi::http::types::FieldName,
    ) -> wasmtime::Result<Result<Vec<wasi::http::types::FieldValue>, wasi::http::types::HeaderError>>
    {
        convert_result(latest::http::types::HostFields::get_and_delete(
            self, fields, name,
        ))
    }

    fn append(
        &mut self,
        fields: Resource<wasi::http::types::Fields>,
        name: wasi::http::types::FieldName,
        value: wasi::http::types::FieldValue,
    ) -> wasmtime::Result<Result<(), wasi::http::types::HeaderError>> {
        convert_result(latest::http::types::HostFields::append(
            self, fields, name, value,
        ))
    }

    fn copy_all(
        &mut self,
        fields: Resource<wasi::http::types::Fields>,
    ) -> wasmtime::Result<Vec<(wasi::http::types::FieldName, wasi::http::types::FieldValue)>> {
        latest::http::types::HostFields::copy_all(self, fields)
    }

    fn clone(
        &mut self,
        fields: Resource<wasi::http::types::Fields>,
    ) -> wasmtime::Result<Resource<wasi::http::types::Fields>> {
        latest::http::types::HostFields::clone(self, fields)
    }

    fn drop(&mut self, fields: Resource<wasi::http::types::Fields>) -> wasmtime::Result<()> {
        latest::http::types::HostFields::drop(self, fields)
    }
}

impl<T> wasi::http::types::HostResponseWithStore<T> for WasiHttp {
    fn new(
        mut store: Access<T, Self>,
        headers: Resource<wasi::http::types::Headers>,
        contents: Option<StreamReader<u8>>,
        trailers: FutureReader<
            Result<Option<Resource<wasi::http::types::Trailers>>, wasi::http::types::ErrorCode>,
        >,
    ) -> wasmtime::Result<(
        Resource<wasi::http::types::Response>,
        FutureReader<Result<(), wasi::http::types::ErrorCode>>,
    )> {
        let trailers = trailers.try_map(store.as_context_mut(), |v| v.map_err(|v| v.into()))?;
        latest::http::types::HostResponseWithStore::new(
            reborrow(&mut store),
            headers,
            contents,
            trailers,
        )
        .and_then(|(res, future)| Ok((res, future.try_map(store, |v| v.map_err(|v| v.into()))?)))
    }

    fn consume_body(
        mut store: Access<T, Self>,
        res: Resource<wasi::http::types::Response>,
        fut: FutureReader<Result<(), wasi::http::types::ErrorCode>>,
    ) -> wasmtime::Result<(
        StreamReader<u8>,
        FutureReader<
            Result<Option<Resource<wasi::http::types::Trailers>>, wasi::http::types::ErrorCode>,
        >,
    )> {
        let fut = fut.try_map(store.as_context_mut(), |v| v.map_err(|v| v.into()))?;
        latest::http::types::HostResponseWithStore::consume_body(reborrow(&mut store), res, fut)
            .and_then(|(stream, future)| {
                Ok((stream, future.try_map(store, |v| v.map_err(|v| v.into()))?))
            })
    }

    fn drop(
        store: Access<'_, T, Self>,
        res: Resource<wasi::http::types::Response>,
    ) -> wasmtime::Result<()> {
        latest::http::types::HostResponseWithStore::drop(store, res)
    }
}

impl<T> wasi::http::client::HostWithStore<T> for WasiHttp {
    async fn send(
        store: &Accessor<T, Self>,
        req: Resource<wasi::http::types::Request>,
    ) -> wasmtime::Result<Result<Resource<wasi::http::types::Response>, wasi::http::types::ErrorCode>>
    {
        convert_result(latest::http::client::HostWithStore::send(store, req).await)
    }
}

impl wasi::http::client::Host for WasiHttpCtxView<'_> {}

convert! {
    struct latest::http::types::DnsErrorPayload [<=>] wasi::http::types::DnsErrorPayload {
        rcode,
        info_code,
    }

    struct latest::http::types::TlsAlertReceivedPayload [<=>] wasi::http::types::TlsAlertReceivedPayload {
        alert_id,
        alert_message,
    }

    struct latest::http::types::FieldSizePayload [<=>] wasi::http::types::FieldSizePayload {
        field_name,
        field_size,
    }

    enum latest::http::types::RequestOptionsError => wasi::http::types::RequestOptionsError {
        NotSupported,
        Immutable,
        Other(v),
    }

    enum latest::http::types::Method [<=>] wasi::http::types::Method {
        Get,
        Head,
        Post,
        Put,
        Delete,
        Connect,
        Options,
        Trace,
        Patch,
        Other(v),
    }

    enum latest::http::types::Scheme [<=>] wasi::http::types::Scheme {
        Http,
        Https,
        Other(v),
    }

    enum latest::http::types::HeaderError [<=>] wasi::http::types::HeaderError {
        InvalidSyntax,
        Forbidden,
        Immutable,
        SizeExceeded,
        Other(v),
    }
}

impl From<wasi::http::types::ErrorCode> for latest::http::types::ErrorCode {
    fn from(e: wasi::http::types::ErrorCode) -> Self {
        match e {
            wasi::http::types::ErrorCode::DnsTimeout => latest::http::types::ErrorCode::DnsTimeout,
            wasi::http::types::ErrorCode::DnsError(e) => {
                latest::http::types::ErrorCode::DnsError(e.into())
            }
            wasi::http::types::ErrorCode::DestinationNotFound => {
                latest::http::types::ErrorCode::DestinationNotFound
            }
            wasi::http::types::ErrorCode::DestinationUnavailable => {
                latest::http::types::ErrorCode::DestinationUnavailable
            }
            wasi::http::types::ErrorCode::DestinationIpProhibited => {
                latest::http::types::ErrorCode::DestinationIpProhibited
            }
            wasi::http::types::ErrorCode::DestinationIpUnroutable => {
                latest::http::types::ErrorCode::DestinationIpUnroutable
            }
            wasi::http::types::ErrorCode::ConnectionRefused => {
                latest::http::types::ErrorCode::ConnectionRefused
            }
            wasi::http::types::ErrorCode::ConnectionTerminated => {
                latest::http::types::ErrorCode::ConnectionTerminated
            }
            wasi::http::types::ErrorCode::ConnectionTimeout => {
                latest::http::types::ErrorCode::ConnectionTimeout
            }
            wasi::http::types::ErrorCode::ConnectionReadTimeout => {
                latest::http::types::ErrorCode::ConnectionReadTimeout
            }
            wasi::http::types::ErrorCode::ConnectionWriteTimeout => {
                latest::http::types::ErrorCode::ConnectionWriteTimeout
            }
            wasi::http::types::ErrorCode::ConnectionLimitReached => {
                latest::http::types::ErrorCode::ConnectionLimitReached
            }
            wasi::http::types::ErrorCode::TlsProtocolError => {
                latest::http::types::ErrorCode::TlsProtocolError
            }
            wasi::http::types::ErrorCode::TlsCertificateError => {
                latest::http::types::ErrorCode::TlsCertificateError
            }
            wasi::http::types::ErrorCode::TlsAlertReceived(e) => {
                latest::http::types::ErrorCode::TlsAlertReceived(e.into())
            }
            wasi::http::types::ErrorCode::HttpRequestDenied => {
                latest::http::types::ErrorCode::HttpRequestDenied
            }
            wasi::http::types::ErrorCode::HttpRequestLengthRequired => {
                latest::http::types::ErrorCode::HttpRequestLengthRequired
            }
            wasi::http::types::ErrorCode::HttpRequestBodySize(e) => {
                latest::http::types::ErrorCode::HttpRequestBodySize(e)
            }
            wasi::http::types::ErrorCode::HttpRequestMethodInvalid => {
                latest::http::types::ErrorCode::HttpRequestMethodInvalid
            }
            wasi::http::types::ErrorCode::HttpRequestUriInvalid => {
                latest::http::types::ErrorCode::HttpRequestUriInvalid
            }
            wasi::http::types::ErrorCode::HttpRequestUriTooLong => {
                latest::http::types::ErrorCode::HttpRequestUriTooLong
            }
            wasi::http::types::ErrorCode::HttpRequestHeaderSectionSize(e) => {
                latest::http::types::ErrorCode::HttpRequestHeaderSectionSize(e)
            }
            wasi::http::types::ErrorCode::HttpRequestHeaderSize(e) => {
                latest::http::types::ErrorCode::HttpRequestHeaderSize(e.map(|e| e.into()))
            }
            wasi::http::types::ErrorCode::HttpRequestTrailerSectionSize(e) => {
                latest::http::types::ErrorCode::HttpRequestTrailerSectionSize(e)
            }
            wasi::http::types::ErrorCode::HttpRequestTrailerSize(e) => {
                latest::http::types::ErrorCode::HttpRequestTrailerSize(e.into())
            }
            wasi::http::types::ErrorCode::HttpResponseIncomplete => {
                latest::http::types::ErrorCode::HttpResponseIncomplete
            }
            wasi::http::types::ErrorCode::HttpResponseHeaderSectionSize(e) => {
                latest::http::types::ErrorCode::HttpResponseHeaderSectionSize(e)
            }
            wasi::http::types::ErrorCode::HttpResponseHeaderSize(e) => {
                latest::http::types::ErrorCode::HttpResponseHeaderSize(e.into())
            }
            wasi::http::types::ErrorCode::HttpResponseBodySize(e) => {
                latest::http::types::ErrorCode::HttpResponseBodySize(e)
            }
            wasi::http::types::ErrorCode::HttpResponseTrailerSectionSize(e) => {
                latest::http::types::ErrorCode::HttpResponseTrailerSectionSize(e)
            }
            wasi::http::types::ErrorCode::HttpResponseTrailerSize(e) => {
                latest::http::types::ErrorCode::HttpResponseTrailerSize(e.into())
            }
            wasi::http::types::ErrorCode::HttpResponseTransferCoding(e) => {
                latest::http::types::ErrorCode::HttpResponseTransferCoding(e)
            }
            wasi::http::types::ErrorCode::HttpResponseContentCoding(e) => {
                latest::http::types::ErrorCode::HttpResponseContentCoding(e)
            }
            wasi::http::types::ErrorCode::HttpResponseTimeout => {
                latest::http::types::ErrorCode::HttpResponseTimeout
            }
            wasi::http::types::ErrorCode::HttpUpgradeFailed => {
                latest::http::types::ErrorCode::HttpUpgradeFailed
            }
            wasi::http::types::ErrorCode::HttpProtocolError => {
                latest::http::types::ErrorCode::HttpProtocolError
            }
            wasi::http::types::ErrorCode::LoopDetected => {
                latest::http::types::ErrorCode::LoopDetected
            }
            wasi::http::types::ErrorCode::ConfigurationError => {
                latest::http::types::ErrorCode::ConfigurationError
            }
            wasi::http::types::ErrorCode::InternalError(e) => {
                latest::http::types::ErrorCode::InternalError(e)
            }
        }
    }
}

impl From<latest::http::types::ErrorCode> for wasi::http::types::ErrorCode {
    fn from(e: latest::http::types::ErrorCode) -> Self {
        match e {
            latest::http::types::ErrorCode::DnsTimeout => wasi::http::types::ErrorCode::DnsTimeout,
            latest::http::types::ErrorCode::DnsError(e) => {
                wasi::http::types::ErrorCode::DnsError(e.into())
            }
            latest::http::types::ErrorCode::DestinationNotFound => {
                wasi::http::types::ErrorCode::DestinationNotFound
            }
            latest::http::types::ErrorCode::DestinationUnavailable => {
                wasi::http::types::ErrorCode::DestinationUnavailable
            }
            latest::http::types::ErrorCode::DestinationIpProhibited => {
                wasi::http::types::ErrorCode::DestinationIpProhibited
            }
            latest::http::types::ErrorCode::DestinationIpUnroutable => {
                wasi::http::types::ErrorCode::DestinationIpUnroutable
            }
            latest::http::types::ErrorCode::ConnectionRefused => {
                wasi::http::types::ErrorCode::ConnectionRefused
            }
            latest::http::types::ErrorCode::ConnectionTerminated => {
                wasi::http::types::ErrorCode::ConnectionTerminated
            }
            latest::http::types::ErrorCode::ConnectionTimeout => {
                wasi::http::types::ErrorCode::ConnectionTimeout
            }
            latest::http::types::ErrorCode::ConnectionReadTimeout => {
                wasi::http::types::ErrorCode::ConnectionReadTimeout
            }
            latest::http::types::ErrorCode::ConnectionWriteTimeout => {
                wasi::http::types::ErrorCode::ConnectionWriteTimeout
            }
            latest::http::types::ErrorCode::ConnectionLimitReached => {
                wasi::http::types::ErrorCode::ConnectionLimitReached
            }
            latest::http::types::ErrorCode::TlsProtocolError => {
                wasi::http::types::ErrorCode::TlsProtocolError
            }
            latest::http::types::ErrorCode::TlsCertificateError => {
                wasi::http::types::ErrorCode::TlsCertificateError
            }
            latest::http::types::ErrorCode::TlsAlertReceived(e) => {
                wasi::http::types::ErrorCode::TlsAlertReceived(e.into())
            }
            latest::http::types::ErrorCode::HttpRequestDenied => {
                wasi::http::types::ErrorCode::HttpRequestDenied
            }
            latest::http::types::ErrorCode::HttpRequestLengthRequired => {
                wasi::http::types::ErrorCode::HttpRequestLengthRequired
            }
            latest::http::types::ErrorCode::HttpRequestBodySize(e) => {
                wasi::http::types::ErrorCode::HttpRequestBodySize(e)
            }
            latest::http::types::ErrorCode::HttpRequestMethodInvalid => {
                wasi::http::types::ErrorCode::HttpRequestMethodInvalid
            }
            latest::http::types::ErrorCode::HttpRequestUriInvalid => {
                wasi::http::types::ErrorCode::HttpRequestUriInvalid
            }
            latest::http::types::ErrorCode::HttpRequestUriTooLong => {
                wasi::http::types::ErrorCode::HttpRequestUriTooLong
            }
            latest::http::types::ErrorCode::HttpRequestHeaderSectionSize(e) => {
                wasi::http::types::ErrorCode::HttpRequestHeaderSectionSize(e)
            }
            latest::http::types::ErrorCode::HttpRequestHeaderSize(e) => {
                wasi::http::types::ErrorCode::HttpRequestHeaderSize(e.map(|e| e.into()))
            }
            latest::http::types::ErrorCode::HttpRequestTrailerSectionSize(e) => {
                wasi::http::types::ErrorCode::HttpRequestTrailerSectionSize(e)
            }
            latest::http::types::ErrorCode::HttpRequestTrailerSize(e) => {
                wasi::http::types::ErrorCode::HttpRequestTrailerSize(e.into())
            }
            latest::http::types::ErrorCode::HttpResponseIncomplete => {
                wasi::http::types::ErrorCode::HttpResponseIncomplete
            }
            latest::http::types::ErrorCode::HttpResponseHeaderSectionSize(e) => {
                wasi::http::types::ErrorCode::HttpResponseHeaderSectionSize(e)
            }
            latest::http::types::ErrorCode::HttpResponseHeaderSize(e) => {
                wasi::http::types::ErrorCode::HttpResponseHeaderSize(e.into())
            }
            latest::http::types::ErrorCode::HttpResponseBodySize(e) => {
                wasi::http::types::ErrorCode::HttpResponseBodySize(e)
            }
            latest::http::types::ErrorCode::HttpResponseTrailerSectionSize(e) => {
                wasi::http::types::ErrorCode::HttpResponseTrailerSectionSize(e)
            }
            latest::http::types::ErrorCode::HttpResponseTrailerSize(e) => {
                wasi::http::types::ErrorCode::HttpResponseTrailerSize(e.into())
            }
            latest::http::types::ErrorCode::HttpResponseTransferCoding(e) => {
                wasi::http::types::ErrorCode::HttpResponseTransferCoding(e)
            }
            latest::http::types::ErrorCode::HttpResponseContentCoding(e) => {
                wasi::http::types::ErrorCode::HttpResponseContentCoding(e)
            }
            latest::http::types::ErrorCode::HttpResponseTimeout => {
                wasi::http::types::ErrorCode::HttpResponseTimeout
            }
            latest::http::types::ErrorCode::HttpUpgradeFailed => {
                wasi::http::types::ErrorCode::HttpUpgradeFailed
            }
            latest::http::types::ErrorCode::HttpProtocolError => {
                wasi::http::types::ErrorCode::HttpProtocolError
            }
            latest::http::types::ErrorCode::LoopDetected => {
                wasi::http::types::ErrorCode::LoopDetected
            }
            latest::http::types::ErrorCode::ConfigurationError => {
                wasi::http::types::ErrorCode::ConfigurationError
            }
            latest::http::types::ErrorCode::InternalError(e) => {
                wasi::http::types::ErrorCode::InternalError(e)
            }
        }
    }
}
