use anyhow::{anyhow, Result};
use url::Url;

const ALLOW_ALL_HOSTS: &str = "insecure:allow-all";

/// An HTTP host allow-list.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AllowedHttpHosts {
    /// All HTTP hosts are allowed (the "insecure:allow-all" value was present in the list)
    AllowAll,
    /// Only the specified hosts are allowed.
    AllowSpecific(Vec<AllowedHttpHost>),
}

impl Default for AllowedHttpHosts {
    fn default() -> Self {
        Self::AllowSpecific(vec![])
    }
}

/// An HTTP host allow-list entry.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AllowedHttpHost {
    pub(crate) domain: String,
    pub(crate) port: Option<u16>,
}

impl AllowedHttpHost {
    /// Creates a new allow-list entry.
    pub fn new(name: impl Into<String>, port: Option<u16>) -> Self {
        Self {
            domain: name.into(),
            port,
        }
    }
}

/// Parses a list of allowed HTTP hosts
pub fn parse_allowed_http_hosts(list: &[impl AsRef<str>]) -> Result<AllowedHttpHosts> {
    if list.iter().any(|domain| domain.as_ref() == ALLOW_ALL_HOSTS) {
        Ok(AllowedHttpHosts::AllowAll)
    } else {
        let parse_results = list
            .iter()
            .map(|h| parse_allowed_http_host(h.as_ref()))
            .collect::<Vec<_>>();
        let (hosts, errors) = partition_results(parse_results);

        if errors.is_empty() {
            Ok(AllowedHttpHosts::AllowSpecific(hosts))
        } else {
            Err(anyhow!(
                "One or more allowed_http_hosts entries was invalid:\n{}",
                errors.join("\n")
            ))
        }
    }
}

fn parse_allowed_http_host(text: &str) -> Result<AllowedHttpHost, String> {
    // If you call Url::parse, it accepts things like `localhost:3001`, inferring
    // `localhost` as a scheme. That's unhelpful for us, so we do a crude check
    // before trying to treat the string as a URL.
    if text.contains("//") {
        parse_allowed_http_host_from_schemed(text)
    } else {
        parse_allowed_http_host_from_unschemed(text)
    }
}

fn parse_allowed_http_host_from_unschemed(text: &str) -> Result<AllowedHttpHost, String> {
    // Host name parsing is quite hairy (thanks, IPv6), so punt it off to the
    // Url type which gets paid big bucks to do it properly. (But preserve the
    // original un-URL-ified string for use in error messages.)
    let urlised = format!("http://{text}");
    let fake_url = Url::parse(&urlised)
        .map_err(|_| format!("{text} isn't a valid host or host:port string"))?;
    parse_allowed_http_host_from_http_url(&fake_url, text)
}

fn parse_allowed_http_host_from_schemed(text: &str) -> Result<AllowedHttpHost, String> {
    let url = Url::parse(text).map_err(|e| format!("{text} isn't a valid HTTP host URL: {e}"))?;

    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("{text} isn't a valid host or host:port string"));
    }

    parse_allowed_http_host_from_http_url(&url, text)
}

fn parse_allowed_http_host_from_http_url(url: &Url, text: &str) -> Result<AllowedHttpHost, String> {
    let host = url
        .host_str()
        .ok_or_else(|| format!("{text} doesn't contain a host name"))?;

    let has_path = url.path().len() > 1; // allow "/"
    if has_path {
        return Err(format!(
            "{text} contains a path, should be host and optional port only"
        ));
    }

    Ok(AllowedHttpHost::new(host, url.port()))
}

fn partition_results<T, E>(results: Vec<Result<T, E>>) -> (Vec<T>, Vec<E>) {
    // We are going to to be OPTIMISTIC do you hear me
    let mut oks = Vec::with_capacity(results.len());
    let mut errs = vec![];

    for result in results {
        match result {
            Ok(t) => oks.push(t),
            Err(e) => errs.push(e),
        }
    }

    (oks, errs)
}

#[cfg(test)]
mod test {
    use super::*;

    impl AllowedHttpHost {
        /// An allow-list entry that specifies a host and allows the default port.
        fn host(name: impl Into<String>) -> Self {
            Self {
                domain: name.into(),
                port: None,
            }
        }

        /// An allow-list entry that specifies a host and port.
        fn host_and_port(name: impl Into<String>, port: u16) -> Self {
            Self {
                domain: name.into(),
                port: Some(port),
            }
        }
    }

    #[test]
    fn test_allowed_hosts_accepts_http_url() {
        assert_eq!(
            AllowedHttpHost::host("spin.fermyon.dev"),
            parse_allowed_http_host("http://spin.fermyon.dev").unwrap()
        );
        assert_eq!(
            AllowedHttpHost::host("spin.fermyon.dev"),
            parse_allowed_http_host("http://spin.fermyon.dev/").unwrap()
        );
        assert_eq!(
            AllowedHttpHost::host("spin.fermyon.dev"),
            parse_allowed_http_host("https://spin.fermyon.dev").unwrap()
        );
    }

    #[test]
    fn test_allowed_hosts_accepts_http_url_with_port() {
        assert_eq!(
            AllowedHttpHost::host_and_port("spin.fermyon.dev", 4444),
            parse_allowed_http_host("http://spin.fermyon.dev:4444").unwrap()
        );
        assert_eq!(
            AllowedHttpHost::host_and_port("spin.fermyon.dev", 4444),
            parse_allowed_http_host("http://spin.fermyon.dev:4444/").unwrap()
        );
        assert_eq!(
            AllowedHttpHost::host_and_port("spin.fermyon.dev", 5555),
            parse_allowed_http_host("https://spin.fermyon.dev:5555").unwrap()
        );
    }

    #[test]
    fn test_allowed_hosts_accepts_plain_host() {
        assert_eq!(
            AllowedHttpHost::host("spin.fermyon.dev"),
            parse_allowed_http_host("spin.fermyon.dev").unwrap()
        );
    }

    #[test]
    fn test_allowed_hosts_accepts_plain_host_with_port() {
        assert_eq!(
            AllowedHttpHost::host_and_port("spin.fermyon.dev", 7777),
            parse_allowed_http_host("spin.fermyon.dev:7777").unwrap()
        );
    }

    #[test]
    fn test_allowed_hosts_accepts_self() {
        assert_eq!(
            AllowedHttpHost::host("self"),
            parse_allowed_http_host("self").unwrap()
        );
    }

    #[test]
    fn test_allowed_hosts_accepts_localhost_addresses() {
        assert_eq!(
            AllowedHttpHost::host("localhost"),
            parse_allowed_http_host("localhost").unwrap()
        );
        assert_eq!(
            AllowedHttpHost::host("localhost"),
            parse_allowed_http_host("http://localhost").unwrap()
        );
        assert_eq!(
            AllowedHttpHost::host_and_port("localhost", 3001),
            parse_allowed_http_host("localhost:3001").unwrap()
        );
        assert_eq!(
            AllowedHttpHost::host_and_port("localhost", 3001),
            parse_allowed_http_host("http://localhost:3001").unwrap()
        );
    }

    #[test]
    fn test_allowed_hosts_accepts_ip_addresses() {
        assert_eq!(
            AllowedHttpHost::host("192.168.1.1"),
            parse_allowed_http_host("192.168.1.1").unwrap()
        );
        assert_eq!(
            AllowedHttpHost::host("192.168.1.1"),
            parse_allowed_http_host("http://192.168.1.1").unwrap()
        );
        assert_eq!(
            AllowedHttpHost::host_and_port("192.168.1.1", 3002),
            parse_allowed_http_host("192.168.1.1:3002").unwrap()
        );
        assert_eq!(
            AllowedHttpHost::host_and_port("192.168.1.1", 3002),
            parse_allowed_http_host("http://192.168.1.1:3002").unwrap()
        );
        assert_eq!(
            AllowedHttpHost::host("[::1]"),
            parse_allowed_http_host("[::1]").unwrap()
        );
        assert_eq!(
            AllowedHttpHost::host_and_port("[::1]", 8001),
            parse_allowed_http_host("http://[::1]:8001").unwrap()
        );
    }

    #[test]
    fn test_allowed_hosts_rejects_path() {
        assert!(parse_allowed_http_host("http://spin.fermyon.dev/a").is_err());
        assert!(parse_allowed_http_host("http://spin.fermyon.dev:6666/a/b").is_err());
    }

    #[test]
    fn test_allowed_hosts_rejects_ftp_url() {
        assert!(parse_allowed_http_host("ftp://spin.fermyon.dev").is_err());
        assert!(parse_allowed_http_host("ftp://spin.fermyon.dev:6666").is_err());
    }

    #[test]
    fn test_allowed_hosts_respects_allow_all() {
        assert_eq!(
            AllowedHttpHosts::AllowAll,
            parse_allowed_http_hosts(&["insecure:allow-all"]).unwrap()
        );
        assert_eq!(
            AllowedHttpHosts::AllowAll,
            parse_allowed_http_hosts(&["spin.fermyon.dev", "insecure:allow-all"]).unwrap()
        );
    }
}
