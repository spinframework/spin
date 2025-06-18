use std::{
    borrow::Cow,
    net::SocketAddr,
    str::{self, FromStr},
};

use anyhow::Result;
use http::Uri;
use hyper::Request;
use spin_factor_outbound_networking::config::allowed_hosts::is_service_chaining_host;
use spin_http::routes::RouteMatch;

use crate::Body;

// We need to make the following pieces of information available to both executors.
// While the values we set are identical, the way they are passed to the
// modules is going to be different, so each executor must must use the info
// in its standardized way (environment variables for the Wagi executor, and custom headers
// for the Spin HTTP executor).
pub const FULL_URL: [&str; 2] = ["SPIN_FULL_URL", "X_FULL_URL"];
pub const PATH_INFO: [&str; 2] = ["SPIN_PATH_INFO", "PATH_INFO"];
pub const MATCHED_ROUTE: [&str; 2] = ["SPIN_MATCHED_ROUTE", "X_MATCHED_ROUTE"];
pub const COMPONENT_ROUTE: [&str; 2] = ["SPIN_COMPONENT_ROUTE", "X_COMPONENT_ROUTE"];
pub const RAW_COMPONENT_ROUTE: [&str; 2] = ["SPIN_RAW_COMPONENT_ROUTE", "X_RAW_COMPONENT_ROUTE"];
pub const BASE_PATH: [&str; 2] = ["SPIN_BASE_PATH", "X_BASE_PATH"];
pub const CLIENT_ADDR: [&str; 2] = ["SPIN_CLIENT_ADDR", "X_CLIENT_ADDR"];

// Header key/value pairs that use copy on write to avoid allocation
pub type HeaderPair<'a> = ([Cow<'static, str>; 2], Cow<'a, str>);

/// Compute the default headers to be passed to the component.
pub fn compute_default_headers<'a>(
    uri: &Uri,
    host: &str,
    route_match: &'a RouteMatch,
    client_addr: SocketAddr,
) -> anyhow::Result<Vec<HeaderPair<'a>>> {
    fn owned(strs: &[&'static str; 2]) -> [Cow<'static, str>; 2] {
        [strs[0].into(), strs[1].into()]
    }

    let owned_full_url = owned(&FULL_URL);
    let owned_path_info = owned(&PATH_INFO);
    let owned_matched_route = owned(&MATCHED_ROUTE);
    let owned_component_route = owned(&COMPONENT_ROUTE);
    let owned_raw_component_route = owned(&RAW_COMPONENT_ROUTE);
    let owned_base_path = owned(&BASE_PATH);
    let owned_client_addr = owned(&CLIENT_ADDR);

    let mut res = vec![];
    let abs_path = uri
        .path_and_query()
        .expect("cannot get path and query")
        .as_str();

    let path_info = route_match.trailing_wildcard();

    let scheme = uri.scheme_str().unwrap_or("http");

    let full_url = format!("{}://{}{}", scheme, host, abs_path);

    res.push((owned_path_info, path_info));
    res.push((owned_full_url, full_url.into()));
    res.push((owned_matched_route, route_match.based_route().into()));

    res.push((owned_base_path, "/".into()));
    res.push((owned_raw_component_route, route_match.raw_route().into()));
    res.push((
        owned_component_route,
        route_match.raw_route_or_prefix().into(),
    ));
    res.push((owned_client_addr, client_addr.to_string().into()));

    for (wild_name, wild_value) in route_match.named_wildcards() {
        let wild_header = format!("SPIN_PATH_MATCH_{}", wild_name.to_ascii_uppercase()).into();
        let wild_wagi_header = format!("X_PATH_MATCH_{}", wild_name.to_ascii_uppercase()).into();
        res.push(([wild_header, wild_wagi_header], wild_value.into()));
    }

    Ok(res)
}

pub fn strip_forbidden_headers(req: &mut Request<Body>) {
    let headers = req.headers_mut();
    if let Some(host_header) = headers.get("Host") {
        if let Ok(host) = host_header.to_str() {
            if is_service_chaining_host(host) {
                headers.remove("Host");
            }
        }
    }
}

pub fn prepare_request_headers(
    req: &Request<Body>,
    route_match: &RouteMatch,
    client_addr: SocketAddr,
) -> Result<Vec<(String, String)>> {
    let mut res = Vec::new();
    for (name, value) in req
        .headers()
        .iter()
        .map(|(name, value)| (name.to_string(), std::str::from_utf8(value.as_bytes())))
    {
        let value = value?.to_string();
        res.push((name, value));
    }

    let default_host = http::HeaderValue::from_str("localhost")?;
    let host = std::str::from_utf8(
        req.headers()
            .get("host")
            .unwrap_or(&default_host)
            .as_bytes(),
    )?;

    // Set the environment information (path info, base path, etc) as headers.
    // In the future, we might want to have this information in a context
    // object as opposed to headers.
    for (keys, val) in compute_default_headers(req.uri(), host, route_match, client_addr)? {
        res.push((prepare_header_key(&keys[0]), val.into_owned()));
    }

    Ok(res)
}

pub fn append_headers(
    map: &mut http::HeaderMap,
    headers: Option<Vec<(String, String)>>,
) -> Result<()> {
    if let Some(src) = headers {
        for (k, v) in src.iter() {
            map.insert(
                http::header::HeaderName::from_str(k)?,
                http::header::HeaderValue::from_str(v)?,
            );
        }
    };

    Ok(())
}

fn prepare_header_key(key: &str) -> String {
    key.replace('_', "-").to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::*;
    use anyhow::Result;
    use spin_http::routes::Router;

    #[test]
    fn test_spin_header_keys() {
        assert_eq!(
            prepare_header_key("SPIN_FULL_URL"),
            "spin-full-url".to_string()
        );
        assert_eq!(
            prepare_header_key("SPIN_PATH_INFO"),
            "spin-path-info".to_string()
        );
        assert_eq!(
            prepare_header_key("SPIN_RAW_COMPONENT_ROUTE"),
            "spin-raw-component-route".to_string()
        );
    }

    #[test]
    fn test_default_headers() -> Result<()> {
        let scheme = "https";
        let host = "fermyon.dev";
        let trigger_route = "/foo/...";
        let component_path = "/foo";
        let path_info = "/bar";
        let client_addr: SocketAddr = "127.0.0.1:8777".parse().unwrap();

        let req_uri = format!(
            "{}://{}{}{}?key1=value1&key2=value2",
            scheme, host, component_path, path_info
        );

        let req = http::Request::builder()
            .method("POST")
            .uri(req_uri)
            .body("")?;

        let router = Router::build("/", [("DUMMY", &trigger_route.into())], None)?;
        let route_match = router.route("/foo/bar")?;

        let default_headers = compute_default_headers(req.uri(), host, &route_match, client_addr)?;

        assert_eq!(
            search(&FULL_URL, &default_headers).unwrap(),
            "https://fermyon.dev/foo/bar?key1=value1&key2=value2".to_string()
        );
        assert_eq!(
            search(&PATH_INFO, &default_headers).unwrap(),
            "/bar".to_string()
        );
        assert_eq!(
            search(&MATCHED_ROUTE, &default_headers).unwrap(),
            "/foo/...".to_string()
        );
        assert_eq!(
            search(&BASE_PATH, &default_headers).unwrap(),
            "/".to_string()
        );
        assert_eq!(
            search(&RAW_COMPONENT_ROUTE, &default_headers).unwrap(),
            "/foo/...".to_string()
        );
        assert_eq!(
            search(&COMPONENT_ROUTE, &default_headers).unwrap(),
            "/foo".to_string()
        );
        assert_eq!(
            search(&CLIENT_ADDR, &default_headers).unwrap(),
            "127.0.0.1:8777".to_string()
        );

        Ok(())
    }

    #[test]
    fn test_default_headers_with_named_wildcards() -> Result<()> {
        let scheme = "https";
        let host = "fermyon.dev";
        let trigger_route = "/foo/:userid/...";
        let component_path = "/foo";
        let path_info = "/bar";
        let client_addr: SocketAddr = "127.0.0.1:8777".parse().unwrap();

        let req_uri = format!(
            "{}://{}{}/42{}?key1=value1&key2=value2",
            scheme, host, component_path, path_info
        );

        let req = http::Request::builder()
            .method("POST")
            .uri(req_uri)
            .body("")?;

        let router = Router::build("/", [("DUMMY", &trigger_route.into())], None)?;
        let route_match = router.route("/foo/42/bar")?;

        let default_headers = compute_default_headers(req.uri(), host, &route_match, client_addr)?;

        assert_eq!(
            search(&FULL_URL, &default_headers).unwrap(),
            "https://fermyon.dev/foo/42/bar?key1=value1&key2=value2".to_string()
        );
        assert_eq!(
            search(&PATH_INFO, &default_headers).unwrap(),
            "/bar".to_string()
        );
        assert_eq!(
            search(&MATCHED_ROUTE, &default_headers).unwrap(),
            "/foo/:userid/...".to_string()
        );
        assert_eq!(
            search(&BASE_PATH, &default_headers).unwrap(),
            "/".to_string()
        );
        assert_eq!(
            search(&RAW_COMPONENT_ROUTE, &default_headers).unwrap(),
            "/foo/:userid/...".to_string()
        );
        assert_eq!(
            search(&COMPONENT_ROUTE, &default_headers).unwrap(),
            "/foo/:userid".to_string()
        );
        assert_eq!(
            search(&CLIENT_ADDR, &default_headers).unwrap(),
            "127.0.0.1:8777".to_string()
        );

        assert_eq!(
            search(
                &["SPIN_PATH_MATCH_USERID", "X_PATH_MATCH_USERID"],
                &default_headers
            )
            .unwrap(),
            "42".to_string()
        );

        Ok(())
    }

    #[test]
    fn forbidden_headers_are_removed() {
        let mut req = Request::get("http://test.spin.internal")
            .header("Host", "test.spin.internal")
            .header("accept", "text/plain")
            .body(Default::default())
            .unwrap();

        strip_forbidden_headers(&mut req);

        assert_eq!(1, req.headers().len());
        assert!(req.headers().get("Host").is_none());

        let mut req = Request::get("http://test.spin.internal")
            .header("Host", "test.spin.internal:1234")
            .header("accept", "text/plain")
            .body(Default::default())
            .unwrap();

        strip_forbidden_headers(&mut req);

        assert_eq!(1, req.headers().len());
        assert!(req.headers().get("Host").is_none());
    }

    #[test]
    fn non_forbidden_headers_are_not_removed() {
        let mut req = Request::get("http://test.example.com")
            .header("Host", "test.example.org")
            .header("accept", "text/plain")
            .body(Default::default())
            .unwrap();

        strip_forbidden_headers(&mut req);

        assert_eq!(2, req.headers().len());
        assert!(req.headers().get("Host").is_some());
    }

    fn search(
        keys: &[&str; 2],
        headers: &[([Cow<'static, str>; 2], Cow<'_, str>)],
    ) -> Option<String> {
        let mut res: Option<String> = None;
        for (k, v) in headers {
            if k[0] == keys[0] && k[1] == keys[1] {
                res = Some(v.as_ref().to_owned());
            }
        }

        res
    }
}
