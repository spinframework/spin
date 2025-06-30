// This file contains code copied from https://github.com/deislabs/wagi
// The copied code's license is in this directory under LICENSE.wagi

use std::{collections::HashMap, net::SocketAddr};

use anyhow::Error;
use http::{
    header::{HeaderName, HOST},
    request::Parts,
    HeaderMap, HeaderValue, Response, StatusCode,
};

use crate::{body, routes::RouteMatch, Body};

/// This sets the version of CGI that WAGI adheres to.
///
/// At the point at which WAGI diverges from CGI, this value will be replaced with
/// WAGI/1.0
pub const WAGI_VERSION: &str = "CGI/1.1";

/// The CGI-defined "server software version".
pub const SERVER_SOFTWARE_VERSION: &str = "WAGI/1";

pub fn build_headers(
    route_match: &RouteMatch,
    req: &Parts,
    content_length: usize,
    client_addr: SocketAddr,
    default_host: &str,
    use_tls: bool,
) -> HashMap<String, String> {
    let (host, port) = parse_host_header_uri(&req.headers, &req.uri, default_host);
    let path_info = route_match.trailing_wildcard();

    let mut headers = HashMap::new();

    // CGI headers from RFC
    headers.insert("AUTH_TYPE".to_owned(), "".to_owned()); // Not currently supported

    // CONTENT_LENGTH (from the spec)
    // The server MUST set this meta-variable if and only if the request is
    // accompanied by a message-body entity.  The CONTENT_LENGTH value must
    // reflect the length of the message-body after the server has removed
    // any transfer-codings or content-codings.
    headers.insert("CONTENT_LENGTH".to_owned(), format!("{content_length}"));

    // CONTENT_TYPE (from the spec)
    // The server MUST set this meta-variable if an HTTP Content-Type field is present
    // in the client request header.  If the server receives a request with an
    // attached entity but no Content-Type header field, it MAY attempt to determine
    // the correct content type, otherwise it should omit this meta-variable.
    //
    // Right now, we don't attempt to determine a media type if none is presented.
    //
    // The spec seems to indicate that if CONTENT_LENGTH > 0, this may be set
    // to "application/octet-stream" if no type is otherwise set. Not sure that is
    // a good idea.
    headers.insert(
        "CONTENT_TYPE".to_owned(),
        req.headers
            .get("CONTENT_TYPE")
            .map(|c| c.to_str().unwrap_or(""))
            .unwrap_or("")
            .to_owned(),
    );

    let protocol = if use_tls { "https" } else { "http" };

    // Since this is not in the specification, an X_ is prepended, per spec.
    // NB: It is strange that there is not a way to do this already. The Display impl
    // seems to only provide the path.
    let uri = req.uri.clone();
    headers.insert(
        "X_FULL_URL".to_owned(),
        format!(
            "{}://{}:{}{}",
            protocol,
            host,
            port,
            uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("")
        ),
    );

    headers.insert("GATEWAY_INTERFACE".to_owned(), WAGI_VERSION.to_owned());

    // This is the Wagi route. This is different from PATH_INFO in that it may
    // have a trailing '/...'
    headers.insert(
        "X_MATCHED_ROUTE".to_owned(),
        route_match.based_route().to_string(),
    );

    headers.insert(
        "QUERY_STRING".to_owned(),
        req.uri.query().unwrap_or("").to_owned(),
    );

    headers.insert("REMOTE_ADDR".to_owned(), client_addr.ip().to_string());
    headers.insert("REMOTE_HOST".to_owned(), client_addr.ip().to_string()); // The server MAY substitute it with REMOTE_ADDR
    headers.insert("REMOTE_USER".to_owned(), "".to_owned()); // TODO: Parse this out of uri.authority?
    headers.insert("REQUEST_METHOD".to_owned(), req.method.to_string());

    // The Path component is /$SCRIPT_NAME/$PATH_INFO
    // SCRIPT_NAME is the route that matched.
    // https://datatracker.ietf.org/doc/html/rfc3875#section-4.1.13
    headers.insert(
        "SCRIPT_NAME".to_owned(),
        route_match.based_route_or_prefix().to_owned(),
    );
    // PATH_INFO is any path information after SCRIPT_NAME
    //
    // I am intentionally ignoring the PATH_INFO rule that says that a PATH_INFO
    // cannot have a path seperator in it. If it becomes important to distinguish
    // between what was decoded out of the path and what is encoded in the path,
    // the X_RAW_PATH_INFO can be used.
    //
    // https://datatracker.ietf.org/doc/html/rfc3875#section-4.1.5
    let pathsegment = path_info;
    let pathinfo = percent_encoding::percent_decode_str(&pathsegment).decode_utf8_lossy();
    headers.insert(
        "X_RAW_PATH_INFO".to_owned(),
        pathsegment.as_ref().to_owned(),
    );
    headers.insert("PATH_INFO".to_owned(), pathinfo.to_string());
    // PATH_TRANSLATED is the url-decoded version of PATH_INFO
    // https://datatracker.ietf.org/doc/html/rfc3875#section-4.1.6
    headers.insert("PATH_TRANSLATED".to_owned(), pathinfo.to_string());

    // From the spec: "the server would use the contents of the request's Host header
    // field to select the correct virtual host."
    headers.insert("SERVER_NAME".to_owned(), host);
    headers.insert("SERVER_PORT".to_owned(), port);
    headers.insert("SERVER_PROTOCOL".to_owned(), format!("{:?}", req.version));

    headers.insert(
        "SERVER_SOFTWARE".to_owned(),
        SERVER_SOFTWARE_VERSION.to_owned(),
    );

    // Normalize incoming HTTP headers. The spec says:
    // "The HTTP header field name is converted to upper case, has all
    // occurrences of "-" replaced with "_" and has "HTTP_" prepended to
    // give the meta-variable name."
    req.headers.iter().for_each(|header| {
        let key = format!(
            "HTTP_{}",
            header.0.as_str().to_uppercase().replace('-', "_")
        );
        // Per spec 4.1.18, skip some headers
        if key == "HTTP_AUTHORIZATION" || key == "HTTP_CONNECTION" {
            return;
        }
        let val = header.1.to_str().unwrap_or("CORRUPT VALUE").to_owned();
        headers.insert(key, val);
    });

    headers
}

/// Internal utility function for parsing a host header.
///
/// This attempts to use three sources to construct a definitive host/port pair, ordering
/// by precedent.
///
/// - The content of the host header is considered most authoritative.
/// - Next most authoritative is self.host, which is set at the CLI or in the config
/// - As a last resort, we use the host/port that Hyper gives us.
/// - If none of these provide sufficient data, which is definitely a possiblity,
///   we go with `localhost` as host and `80` as port. This, of course, is problematic,
///   but should only manifest if both the server and the client are behaving badly.
fn parse_host_header_uri(
    headers: &HeaderMap,
    uri: &hyper::Uri,
    default_host: &str,
) -> (String, String) {
    let host_header = headers.get(HOST).and_then(|v| match v.to_str() {
        Err(_) => None,
        Ok(s) => Some(s.to_owned()),
    });

    let mut host = uri
        .host()
        .map(|h| h.to_string())
        .unwrap_or_else(|| "localhost".to_owned());
    let mut port = uri.port_u16().unwrap_or(80).to_string();

    let mut parse_host = |hdr: String| {
        let mut parts = hdr.splitn(2, ':');
        match parts.next() {
            Some(h) if !h.is_empty() => h.clone_into(&mut host),
            _ => {}
        }
        match parts.next() {
            Some(p) if !p.is_empty() => {
                tracing::debug!(port = p, "Overriding port");
                p.clone_into(&mut port);
            }
            _ => {}
        }
    };

    // Override with local host field if set.
    if !default_host.is_empty() {
        parse_host(default_host.to_owned());
    }

    // Finally, the value of the HOST header is considered authoritative.
    // When it comes to port number, the HOST header isn't necessarily 100% trustworthy.
    // But it appears that this is still the best behavior for the CGI spec.
    if let Some(hdr) = host_header {
        parse_host(hdr);
    }

    (host, port)
}

pub fn compose_response(stdout: &[u8]) -> Result<Response<Body>, Error> {
    // Okay, once we get here, all the information we need to send back in the response
    // should be written to the STDOUT buffer. We fetch that, format it, and send
    // it back. In the process, we might need to alter the status code of the result.
    //
    // This is a little janky, but basically we are looping through the output once,
    // looking for the double-newline that distinguishes the headers from the body.
    // The headers can then be parsed separately, while the body can be sent back
    // to the client.
    let mut last = 0;
    let mut scan_headers = true;
    let mut buffer: Vec<u8> = Vec::new();
    let mut out_headers: Vec<u8> = Vec::new();
    stdout.iter().for_each(|i| {
        // Ignore CR in headers
        if scan_headers && *i == 13 {
            return;
        } else if scan_headers && *i == 10 && last == 10 {
            out_headers.append(&mut buffer);
            buffer = Vec::new();
            scan_headers = false;
            return; // Consume the linefeed
        }
        last = *i;
        buffer.push(*i)
    });
    let mut res = Response::new(body::full(buffer.into()));
    let mut sufficient_response = false;
    let mut explicit_status_code = false;
    parse_cgi_headers(String::from_utf8(out_headers)?)
        .iter()
        .for_each(|h| {
            use hyper::header::{CONTENT_TYPE, LOCATION};
            match h.0.to_lowercase().as_str() {
                "content-type" => {
                    sufficient_response = true;
                    res.headers_mut().insert(CONTENT_TYPE, h.1.parse().unwrap());
                }
                "status" => {
                    // The spec does not say that status is a sufficient response.
                    // (It says that it may be added along with Content-Type, because
                    // a status has a content type). However, CGI libraries in the wild
                    // do not set content type correctly if a status is an error.
                    // See https://datatracker.ietf.org/doc/html/rfc3875#section-6.2
                    sufficient_response = true;
                    explicit_status_code = true;
                    // Status can be `Status CODE [STRING]`, and we just want the CODE.
                    let status_code = h.1.split_once(' ').map(|(code, _)| code).unwrap_or(h.1);
                    tracing::debug!(status_code, "Raw status code");
                    match status_code.parse::<StatusCode>() {
                        Ok(code) => *res.status_mut() = code,
                        Err(e) => {
                            tracing::warn!("Failed to parse code: {}", e);
                            *res.status_mut() = StatusCode::BAD_GATEWAY;
                        }
                    }
                }
                "location" => {
                    sufficient_response = true;
                    res.headers_mut()
                        .insert(LOCATION, HeaderValue::from_str(h.1).unwrap());
                    if !explicit_status_code {
                        *res.status_mut() = StatusCode::from_u16(302).unwrap();
                    }
                }
                _ => {
                    // If the header can be parsed into a valid HTTP header, it is
                    // added to the headers. Otherwise it is ignored.
                    match HeaderName::from_lowercase(h.0.as_str().to_lowercase().as_bytes()) {
                        Ok(hdr) => {
                            res.headers_mut()
                                .insert(hdr, HeaderValue::from_str(h.1).unwrap());
                        }
                        Err(e) => {
                            tracing::error!(error = %e, header_name = %h.0, "Invalid header name")
                        }
                    }
                }
            }
        });
    if !sufficient_response {
        tracing::debug!("{:?}", res.body());
        return Ok(internal_error(
            // Technically, we let `status` be sufficient, but this is more lenient
            // than the specification.
            "Exactly one of 'location' or 'content-type' must be specified",
        ));
    }
    Ok(res)
}

fn parse_cgi_headers(headers: String) -> HashMap<String, String> {
    let mut map = HashMap::new();
    headers.trim().split('\n').for_each(|h| {
        let parts: Vec<&str> = h.splitn(2, ':').collect();
        if parts.len() != 2 {
            tracing::warn!(header = h, "corrupt header");
            return;
        }
        map.insert(parts[0].trim().to_owned(), parts[1].trim().to_owned());
    });
    map
}

/// Create an HTTP 500 response
fn internal_error(msg: impl std::string::ToString) -> Response<Body> {
    let message = msg.to_string();
    tracing::error!(error = %message, "HTTP 500 error");
    let mut res = Response::new(body::full(message.into()));
    *res.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
    res
}
