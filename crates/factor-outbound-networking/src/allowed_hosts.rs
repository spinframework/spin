use std::ops::Range;

use anyhow::{bail, ensure, Context};
use spin_factors::{App, AppComponent};
use spin_locked_app::MetadataKey;
use url::Host;

const ALLOWED_HOSTS_KEY: MetadataKey<Vec<String>> = MetadataKey::new("allowed_outbound_hosts");
const ALLOWED_HTTP_KEY: MetadataKey<Vec<String>> = MetadataKey::new("allowed_http_hosts");

pub const SERVICE_CHAINING_DOMAIN: &str = "spin.internal";
pub const SERVICE_CHAINING_DOMAIN_SUFFIX: &str = ".spin.internal";

/// Get the raw values of the `allowed_outbound_hosts` locked app metadata key.
///
/// This has support for converting the old `allowed_http_hosts` key to the new `allowed_outbound_hosts` key.
pub fn allowed_outbound_hosts(component: &AppComponent) -> anyhow::Result<Vec<String>> {
    let mut allowed_hosts = component
        .get_metadata(ALLOWED_HOSTS_KEY)
        .with_context(|| {
            format!(
                "locked app metadata was malformed for key {}",
                ALLOWED_HOSTS_KEY.as_ref()
            )
        })?
        .unwrap_or_default();
    let allowed_http = component
        .get_metadata(ALLOWED_HTTP_KEY)
        .map(|h| h.unwrap_or_default())
        .unwrap_or_default();
    let converted =
        spin_manifest::compat::convert_allowed_http_to_allowed_hosts(&allowed_http, false)
            .unwrap_or_default();
    allowed_hosts.extend(converted);
    Ok(allowed_hosts)
}

/// Validates that all service chaining of an app will be satisfied by the
/// supplied subset of components.
///
/// This does a best effort look up of components that are
/// allowed to be accessed through service chaining and will error early if a
/// component is configured to to chain to another component that is not
/// retained. All wildcard service chaining is disallowed and all templated URLs
/// are ignored.
pub fn validate_service_chaining_for_components(
    app: &App,
    retained_components: &[&str],
) -> anyhow::Result<()> {
    app
        .triggers().try_for_each(|t| {
            let Ok(component) = t.component() else  { return Ok(()) };
            if retained_components.contains(&component.id()) {
            let allowed_hosts = allowed_outbound_hosts(&component).context("failed to get allowed hosts")?;
            for host in allowed_hosts {
                // Templated URLs are not yet resolved at this point, so ignore unresolvable URIs
                if let Ok(uri) = host.parse::<http::Uri>() {
                    if let Some(chaining_target) = parse_service_chaining_target(&uri) {
                        if !retained_components.contains(&chaining_target.as_ref()) {
                            if chaining_target == "*" {
                                return  Err(anyhow::anyhow!("Selected component '{}' cannot use wildcard service chaining: allowed_outbound_hosts = [\"http://*.spin.internal\"]", component.id()));
                            }
                            return  Err(anyhow::anyhow!(
                                "Selected component '{}' cannot use service chaining to unselected component: allowed_outbound_hosts = [\"http://{}.spin.internal\"]",
                                component.id(), chaining_target
                            ));
                        }
                    }
                }
            }
        }
        anyhow::Ok(())
    })?;

    Ok(())
}

/// Represents a single `allowed_outbound_hosts` item.
#[derive(Eq, Debug, Clone)]
pub struct AllowedHostConfig {
    original: String,
    scheme: SchemeConfig,
    host: HostConfig,
    port: PortConfig,
}

impl AllowedHostConfig {
    /// Parses the given string as an `allowed_hosts_config` item.
    pub fn parse(url: impl Into<String>) -> anyhow::Result<Self> {
        let original = url.into();
        let url = original.trim();
        let Some((scheme, rest)) = url.split_once("://") else {
            match url {
                "*" | ":" | "" | "?" => bail!("{url:?} is not an allowed outbound host format.\nHosts must be in the form <scheme>://<host>[:<port>], with '*' wildcards allowed for each.\nIf you intended to allow all outbound networking, you can use '*://*:*' - this will obviate all network sandboxing.\nLearn more: https://spinframework.dev/v3/http-outbound#granting-http-permissions-to-components"),
                _ => bail!("{url:?} does not contain a scheme (e.g., 'http://' or '*://')\nLearn more: https://spinframework.dev/v3/http-outbound#granting-http-permissions-to-components"),
            }
        };
        let (host, rest) = rest.rsplit_once(':').unwrap_or((rest, ""));
        let port = match rest.split_once('/') {
            Some((port, path)) => {
                if !path.is_empty() {
                    bail!("{url:?} has a path but is not allowed to");
                }
                port
            }
            None => rest,
        };

        let port = PortConfig::parse(port, scheme)
            .with_context(|| format!("Invalid allowed host port {port:?}"))?;
        let scheme = SchemeConfig::parse(scheme)
            .with_context(|| format!("Invalid allowed host scheme {scheme:?}"))?;
        let host =
            HostConfig::parse(host).with_context(|| format!("Invalid allowed host {host:?}"))?;

        Ok(Self {
            scheme,
            host,
            port,
            original,
        })
    }

    /// Returns the scheme part of this config.
    pub fn scheme(&self) -> &SchemeConfig {
        &self.scheme
    }

    /// Returns the host part of this config.
    pub fn host(&self) -> &HostConfig {
        &self.host
    }

    /// Returns the port part of this config.
    pub fn port(&self) -> &PortConfig {
        &self.port
    }

    /// Returns true if this config is for service chaining requests.
    pub fn is_for_service_chaining(&self) -> bool {
        self.host.is_for_service_chaining()
    }

    /// Returns true if the given url is allowed by this config.
    fn allows(&self, url: &OutboundUrl) -> bool {
        self.scheme.allows(&url.scheme)
            && self.host.allows(&url.host)
            && self.port.allows(url.port, &url.scheme)
    }

    /// Returns true if this config allows relative ("self") requests to any of
    /// the given schemes.
    fn allows_relative(&self, schemes: &[&str]) -> bool {
        schemes.iter().any(|s| self.scheme.allows(s)) && self.host.allows_relative()
    }
}

impl PartialEq for AllowedHostConfig {
    fn eq(&self, other: &Self) -> bool {
        self.scheme == other.scheme && self.host == other.host && self.port == other.port
    }
}

impl std::fmt::Display for AllowedHostConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.original)
    }
}

/// Represents the scheme part of an allowed_outbound_hosts item.
#[derive(PartialEq, Eq, Debug, Clone)]
pub enum SchemeConfig {
    /// Any scheme is allowed: `*://`
    Any,
    /// Any of the given schemes are allowed
    List(Vec<String>),
}

impl SchemeConfig {
    /// Parses the scheme part of an allowed_outbound_hosts item.
    fn parse(scheme: &str) -> anyhow::Result<Self> {
        if scheme == "*" {
            return Ok(Self::Any);
        }

        if scheme.starts_with('{') {
            // TODO:
            anyhow::bail!("scheme lists are not yet supported")
        }

        if scheme.chars().any(|c| !c.is_alphabetic()) {
            anyhow::bail!("only alphabetic character(s) are allowed");
        }

        Ok(Self::List(vec![scheme.into()]))
    }

    /// Returns true if any scheme is allowed (i.e. `*://`).
    pub fn allows_any(&self) -> bool {
        matches!(self, Self::Any)
    }

    /// Returns true if the given scheme is allowed.
    fn allows(&self, scheme: &str) -> bool {
        match self {
            SchemeConfig::Any => true,
            SchemeConfig::List(l) => l.iter().any(|s| s.as_str() == scheme),
        }
    }
}

/// Represents the host part of an allowed_outbound_hosts item.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum HostConfig {
    Any,
    AnySubdomain(String),
    ToSelf,
    Literal(Host),
    Cidr(ip_network::IpNetwork),
}

impl HostConfig {
    /// Parses the host part of an allowed_outbound_hosts item.
    fn parse(mut host: &str) -> anyhow::Result<Self> {
        host = host.trim();
        if host == "*" {
            return Ok(Self::Any);
        }

        if host == "self" || host == "self.alt" {
            return Ok(Self::ToSelf);
        }

        if host.starts_with('{') {
            ensure!(host.ends_with('}'));
            bail!("host lists are not yet supported")
        }

        if let Ok(net) = ip_network::IpNetwork::from_str_truncate(host) {
            return Ok(Self::Cidr(net));
        }

        host = host.trim_end_matches('/');
        if host.contains('/') {
            bail!("must not include a path");
        }

        if let Some(domain) = host.strip_prefix("*.") {
            if domain.contains('*') {
                bail!("wildcards are allowed only as prefixes");
            }
            return Ok(Self::AnySubdomain(format!(".{domain}")));
        }

        if host.contains('*') {
            bail!("wildcards are allowed only as subdomains");
        }

        Self::literal(host)
    }

    /// Returns a HostConfig from the given literal host name.
    fn literal(host: &str) -> anyhow::Result<Self> {
        Ok(Self::Literal(Host::parse(host)?))
    }

    /// Returns true if the given host is allowed by this config.
    fn allows(&self, host: &str) -> bool {
        let host: Host = match Host::parse(host) {
            Ok(host) => host,
            Err(err) => {
                tracing::warn!(?err, "invalid host in HostConfig::allows");
                return false;
            }
        };
        match (self, host) {
            (HostConfig::Any, _) => true,
            (HostConfig::AnySubdomain(suffix), Host::Domain(domain)) => domain.ends_with(suffix),
            (HostConfig::Literal(literal), host) => host == *literal,
            (HostConfig::Cidr(c), Host::Ipv4(ip)) => c.contains(ip),
            (HostConfig::Cidr(c), Host::Ipv6(ip)) => c.contains(ip),
            // Note: HostConfig::ToSelf is checked with ::allow_relative
            _ => false,
        }
    }

    /// Returns true if this config allows relative ("self") requests.
    fn allows_relative(&self) -> bool {
        matches!(self, Self::Any | Self::ToSelf)
    }

    /// Returns true if this config is for service chaining requests.
    fn is_for_service_chaining(&self) -> bool {
        match self {
            Self::Literal(Host::Domain(domain)) => domain.ends_with(SERVICE_CHAINING_DOMAIN_SUFFIX),
            Self::AnySubdomain(suffix) => suffix == SERVICE_CHAINING_DOMAIN_SUFFIX,
            _ => false,
        }
    }
}

/// Represents the port part of an allowed_outbound_hosts item.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum PortConfig {
    Any,
    List(Vec<IndividualPortConfig>),
}

impl PortConfig {
    /// Parses the port part of an allowed_outbound_hosts item.
    fn parse(port: &str, scheme: &str) -> anyhow::Result<PortConfig> {
        if port.is_empty() {
            return well_known_port(scheme)
                .map(|p| PortConfig::List(vec![IndividualPortConfig::Port(p)]))
                .with_context(|| format!("no port was provided and the scheme {scheme:?} does not have a known default port number"));
        }
        if port == "*" {
            return Ok(PortConfig::Any);
        }

        if port.starts_with('{') {
            // TODO:
            bail!("port lists are not yet supported")
        }

        let port = IndividualPortConfig::parse(port)?;

        Ok(Self::List(vec![port]))
    }

    fn allows(&self, port: Option<u16>, scheme: &str) -> bool {
        match self {
            PortConfig::Any => true,
            PortConfig::List(l) => {
                let port = match port.or_else(|| well_known_port(scheme)) {
                    Some(p) => p,
                    None => return false,
                };
                l.iter().any(|p| p.allows(port))
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum IndividualPortConfig {
    Port(u16),
    Range(Range<u16>),
}

impl IndividualPortConfig {
    fn parse(port: &str) -> anyhow::Result<Self> {
        if let Some((start, end)) = port.split_once("..") {
            let start = start
                .parse()
                .with_context(|| format!("port range {port:?} contains non-number"))?;
            let end = end
                .parse()
                .with_context(|| format!("port range {port:?} contains non-number"))?;
            return Ok(Self::Range(start..end));
        }
        Ok(Self::Port(port.parse().with_context(|| {
            format!("port {port:?} is not a number")
        })?))
    }

    fn allows(&self, port: u16) -> bool {
        match self {
            IndividualPortConfig::Port(p) => p == &port,
            IndividualPortConfig::Range(r) => r.contains(&port),
        }
    }
}

fn well_known_port(scheme: &str) -> Option<u16> {
    match scheme {
        "postgres" => Some(5432),
        "mysql" => Some(3306),
        "redis" => Some(6379),
        "mqtt" => Some(1883),
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    }
}

#[derive(PartialEq, Eq, Debug, Clone)]
pub enum AllowedHostsConfig {
    All,
    SpecificHosts(Vec<AllowedHostConfig>),
}

enum PartialAllowedHostConfig {
    Exact(AllowedHostConfig),
    Unresolved(spin_expressions::Template),
}

impl PartialAllowedHostConfig {
    fn resolve(
        self,
        resolver: &spin_expressions::PreparedResolver,
    ) -> anyhow::Result<AllowedHostConfig> {
        match self {
            Self::Exact(h) => Ok(h),
            Self::Unresolved(t) => AllowedHostConfig::parse(resolver.resolve_template(&t)?),
        }
    }
}

impl AllowedHostsConfig {
    pub fn parse<S: AsRef<str>>(
        hosts: &[S],
        resolver: &spin_expressions::PreparedResolver,
    ) -> anyhow::Result<AllowedHostsConfig> {
        let partial = Self::parse_partial(hosts)?;
        let allowed = partial
            .into_iter()
            .map(|p| p.resolve(resolver))
            .collect::<anyhow::Result<Vec<_>>>()?;
        Ok(Self::SpecificHosts(allowed))
    }

    pub fn validate<S: AsRef<str>>(hosts: &[S]) -> anyhow::Result<()> {
        _ = Self::parse_partial(hosts)?;
        Ok(())
    }

    // Parses literals but defers templates. This is pulled out so that `validate`
    // doesn't have to deal with resolving templates.
    fn parse_partial<S: AsRef<str>>(hosts: &[S]) -> anyhow::Result<Vec<PartialAllowedHostConfig>> {
        if hosts.len() == 1 && hosts[0].as_ref() == "insecure:allow-all" {
            bail!("'insecure:allow-all' is not allowed - use '*://*:*' instead if you really want to allow all outbound traffic'")
        }
        let mut allowed = Vec::with_capacity(hosts.len());
        for host in hosts {
            let template = spin_expressions::Template::new(host.as_ref())?;
            if template.is_literal() {
                allowed.push(PartialAllowedHostConfig::Exact(AllowedHostConfig::parse(
                    host.as_ref(),
                )?));
            } else {
                allowed.push(PartialAllowedHostConfig::Unresolved(template));
            }
        }
        Ok(allowed)
    }

    /// Determine if the supplied url is allowed
    pub fn allows(&self, url: &OutboundUrl) -> bool {
        match self {
            AllowedHostsConfig::All => true,
            AllowedHostsConfig::SpecificHosts(hosts) => hosts.iter().any(|h| h.allows(url)),
        }
    }

    pub fn allows_relative_url(&self, schemes: &[&str]) -> bool {
        match self {
            AllowedHostsConfig::All => true,
            AllowedHostsConfig::SpecificHosts(hosts) => {
                hosts.iter().any(|h| h.allows_relative(schemes))
            }
        }
    }
}

impl Default for AllowedHostsConfig {
    fn default() -> Self {
        Self::SpecificHosts(Vec::new())
    }
}

/// A parsed URL used for outbound networking.
#[derive(Debug, Clone)]
pub struct OutboundUrl {
    scheme: String,
    host: String,
    port: Option<u16>,
    original: String,
}

impl OutboundUrl {
    /// Parses a URL.
    ///
    /// If parsing `url` fails, `{scheme}://` is prepended to `url` and parsing is tried again.
    pub fn parse(url: impl Into<String>, scheme: &str) -> anyhow::Result<Self> {
        let mut url = url.into();
        let original = url.clone();

        // Ensure that the authority is url encoded. Since the authority is ignored after this,
        // we can always url encode the authority even if it is already encoded.
        if let Some(at) = url.find('@') {
            let scheme_end = url.find("://").map(|e| e + 3).unwrap_or(0);
            let path_start = url[scheme_end..]
                .find('/') // This can calculate the wrong index if the username or password contains a '/'
                .map(|e| e + scheme_end)
                .unwrap_or(usize::MAX);

            if at < path_start {
                let userinfo = &url[scheme_end..at];

                let encoded = urlencoding::encode(userinfo);
                let prefix = &url[..scheme_end];
                let suffix = &url[scheme_end + userinfo.len()..];
                url = format!("{prefix}{encoded}{suffix}");
            }
        }

        let parsed = match url::Url::parse(&url) {
            Ok(url) if url.has_host() => Ok(url),
            first_try => {
                let second_try: anyhow::Result<url::Url> = format!("{scheme}://{url}")
                    .as_str()
                    .try_into()
                    .context("could not convert into a url");
                match (second_try, first_try.map_err(|e| e.into())) {
                    (Ok(u), _) => Ok(u),
                    // Return an error preferring the error from the first attempt if present
                    (_, Err(e)) | (Err(e), _) => Err(e),
                }
            }
        }?;

        Ok(Self {
            scheme: parsed.scheme().to_owned(),
            host: parsed
                .host_str()
                .with_context(|| format!("{url:?} does not have a host component"))?
                .to_owned(),
            port: parsed.port(),
            original,
        })
    }

    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    pub fn authority(&self) -> String {
        if let Some(port) = self.port {
            format!("{}:{port}", self.host)
        } else {
            self.host.clone()
        }
    }
}

impl std::fmt::Display for OutboundUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.original)
    }
}

pub fn is_service_chaining_host(host: &str) -> bool {
    parse_service_chaining_host(host).is_some()
}

pub fn parse_service_chaining_target(url: &http::Uri) -> Option<String> {
    let host = url.authority().map(|a| a.host().trim())?;
    parse_service_chaining_host(host)
}

fn parse_service_chaining_host(host: &str) -> Option<String> {
    let (host, _) = host.rsplit_once(':').unwrap_or((host, ""));

    let (first, rest) = host.split_once('.')?;

    if rest == SERVICE_CHAINING_DOMAIN {
        Some(first.to_owned())
    } else {
        None
    }
}

#[cfg(test)]
mod test {
    impl AllowedHostConfig {
        fn new(scheme: SchemeConfig, host: HostConfig, port: PortConfig) -> Self {
            Self {
                scheme,
                host,
                port,
                original: String::new(),
            }
        }
    }

    impl SchemeConfig {
        fn new(scheme: &str) -> Self {
            Self::List(vec![scheme.into()])
        }
    }

    impl HostConfig {
        fn unwrap_literal(host: &str) -> Self {
            Self::literal(host).unwrap_or_else(|_| panic!("invalid host {host:?}"))
        }

        fn subdomain(domain: &str) -> Self {
            Self::AnySubdomain(format!(".{domain}"))
        }
    }

    impl PortConfig {
        fn new(port: u16) -> Self {
            Self::List(vec![IndividualPortConfig::Port(port)])
        }

        fn range(port: Range<u16>) -> Self {
            Self::List(vec![IndividualPortConfig::Range(port)])
        }
    }

    fn dummy_resolver() -> spin_expressions::PreparedResolver {
        spin_expressions::PreparedResolver::default()
    }

    use ip_network::{IpNetwork, Ipv4Network, Ipv6Network};

    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn outbound_url_handles_at_in_paths() {
        let url = "https://example.com/file@0.1.0.json";
        let url = OutboundUrl::parse(url, "https").expect("should have parsed url");
        assert_eq!("example.com", url.host);

        let url = "https://user:password@example.com/file@0.1.0.json";
        let url = OutboundUrl::parse(url, "https").expect("should have parsed url");
        assert_eq!("example.com", url.host);

        let url = "https://user:pass#word@example.com/file@0.1.0.json";
        let url = OutboundUrl::parse(url, "https").expect("should have parsed url");
        assert_eq!("example.com", url.host);

        let url = "https://user:password@example.com";
        let url = OutboundUrl::parse(url, "https").expect("should have parsed url");
        assert_eq!("example.com", url.host);
    }

    #[test]
    fn test_allowed_hosts_accepts_url_without_port() {
        assert_eq!(
            AllowedHostConfig::new(
                SchemeConfig::new("http"),
                HostConfig::unwrap_literal("spin.fermyon.dev"),
                PortConfig::new(80)
            ),
            AllowedHostConfig::parse("http://spin.fermyon.dev").unwrap()
        );

        assert_eq!(
            AllowedHostConfig::new(
                SchemeConfig::new("http"),
                // Trailing slash is removed
                HostConfig::unwrap_literal("spin.fermyon.dev"),
                PortConfig::new(80)
            ),
            AllowedHostConfig::parse("http://spin.fermyon.dev/").unwrap()
        );

        assert_eq!(
            AllowedHostConfig::new(
                SchemeConfig::new("https"),
                HostConfig::unwrap_literal("spin.fermyon.dev"),
                PortConfig::new(443)
            ),
            AllowedHostConfig::parse("https://spin.fermyon.dev").unwrap()
        );
    }

    #[test]
    fn test_allowed_hosts_accepts_url_with_port() {
        assert_eq!(
            AllowedHostConfig::new(
                SchemeConfig::new("http"),
                HostConfig::unwrap_literal("spin.fermyon.dev"),
                PortConfig::new(4444)
            ),
            AllowedHostConfig::parse("http://spin.fermyon.dev:4444").unwrap()
        );
        assert_eq!(
            AllowedHostConfig::new(
                SchemeConfig::new("http"),
                HostConfig::unwrap_literal("spin.fermyon.dev"),
                PortConfig::new(4444)
            ),
            AllowedHostConfig::parse("http://spin.fermyon.dev:4444/").unwrap()
        );
        assert_eq!(
            AllowedHostConfig::new(
                SchemeConfig::new("https"),
                HostConfig::unwrap_literal("spin.fermyon.dev"),
                PortConfig::new(5555)
            ),
            AllowedHostConfig::parse("https://spin.fermyon.dev:5555").unwrap()
        );
    }

    #[test]
    fn test_allowed_hosts_accepts_url_with_port_range() {
        assert_eq!(
            AllowedHostConfig::new(
                SchemeConfig::new("http"),
                HostConfig::unwrap_literal("spin.fermyon.dev"),
                PortConfig::range(4444..5555)
            ),
            AllowedHostConfig::parse("http://spin.fermyon.dev:4444..5555").unwrap()
        );
    }

    #[test]
    fn test_allowed_hosts_does_not_accept_plain_host_without_port() {
        assert!(AllowedHostConfig::parse("spin.fermyon.dev").is_err());
    }

    #[test]
    fn test_allowed_hosts_does_not_accept_plain_host_without_scheme() {
        assert!(AllowedHostConfig::parse("spin.fermyon.dev:80").is_err());
    }

    #[test]
    fn test_allowed_hosts_accepts_host_with_glob_scheme() {
        assert_eq!(
            AllowedHostConfig::new(
                SchemeConfig::Any,
                HostConfig::unwrap_literal("spin.fermyon.dev"),
                PortConfig::new(7777)
            ),
            AllowedHostConfig::parse("*://spin.fermyon.dev:7777").unwrap()
        )
    }

    #[test]
    fn test_allowed_hosts_accepts_self() {
        assert_eq!(
            AllowedHostConfig::new(
                SchemeConfig::new("http"),
                HostConfig::ToSelf,
                PortConfig::new(80)
            ),
            AllowedHostConfig::parse("http://self").unwrap()
        );
    }

    #[test]
    fn test_allowed_hosts_accepts_localhost_addresses() {
        assert!(AllowedHostConfig::parse("localhost").is_err());
        assert_eq!(
            AllowedHostConfig::new(
                SchemeConfig::new("http"),
                HostConfig::unwrap_literal("localhost"),
                PortConfig::new(80)
            ),
            AllowedHostConfig::parse("http://localhost").unwrap()
        );
        assert!(AllowedHostConfig::parse("localhost:3001").is_err());
        assert_eq!(
            AllowedHostConfig::new(
                SchemeConfig::new("http"),
                HostConfig::unwrap_literal("localhost"),
                PortConfig::new(3001)
            ),
            AllowedHostConfig::parse("http://localhost:3001").unwrap()
        );
    }

    #[test]
    fn test_allowed_hosts_accepts_subdomain_wildcards() {
        assert_eq!(
            AllowedHostConfig::new(
                SchemeConfig::new("http"),
                HostConfig::subdomain("example.com"),
                PortConfig::new(80)
            ),
            AllowedHostConfig::parse("http://*.example.com").unwrap()
        );
    }

    #[test]
    fn test_allowed_hosts_accepts_ip_addresses() {
        assert_eq!(
            AllowedHostConfig::new(
                SchemeConfig::new("http"),
                HostConfig::unwrap_literal("192.168.1.1"),
                PortConfig::new(80)
            ),
            AllowedHostConfig::parse("http://192.168.1.1").unwrap()
        );
        assert_eq!(
            AllowedHostConfig::new(
                SchemeConfig::new("http"),
                HostConfig::unwrap_literal("192.168.1.1"),
                PortConfig::new(3002)
            ),
            AllowedHostConfig::parse("http://192.168.1.1:3002").unwrap()
        );
        assert_eq!(
            AllowedHostConfig::new(
                SchemeConfig::new("http"),
                HostConfig::unwrap_literal("[::1]"),
                PortConfig::new(8001)
            ),
            AllowedHostConfig::parse("http://[::1]:8001").unwrap()
        );

        assert!(AllowedHostConfig::parse("http://[::1]").is_err())
    }

    #[test]
    fn test_allowed_hosts_accepts_ip_cidr() {
        assert_eq!(
            AllowedHostConfig::new(
                SchemeConfig::Any,
                HostConfig::Cidr(IpNetwork::V4(
                    Ipv4Network::new(Ipv4Addr::new(127, 0, 0, 0), 24).unwrap()
                )),
                PortConfig::new(80)
            ),
            AllowedHostConfig::parse("*://127.0.0.0/24:80").unwrap()
        );
        assert!(AllowedHostConfig::parse("*://127.0.0.0/24").is_err());
        assert_eq!(
            AllowedHostConfig::new(
                SchemeConfig::Any,
                HostConfig::Cidr(IpNetwork::V6(
                    Ipv6Network::new(Ipv6Addr::new(0xff00, 0, 0, 0, 0, 0, 0, 0), 8).unwrap()
                )),
                PortConfig::new(80)
            ),
            AllowedHostConfig::parse("*://ff00::/8:80").unwrap()
        );
    }

    #[test]
    fn test_allowed_hosts_rejects_path() {
        // An empty path is allowed
        assert!(AllowedHostConfig::parse("http://spin.fermyon.dev/").is_ok());
        // All other paths are not allowed
        assert!(AllowedHostConfig::parse("http://spin.fermyon.dev/a").is_err());
        assert!(AllowedHostConfig::parse("http://spin.fermyon.dev:6666/a/b").is_err());
        assert!(AllowedHostConfig::parse("http://*.fermyon.dev/a").is_err());
    }

    #[test]
    fn test_rejects_invalid_parts() {
        for invalid in [
            "h@x://localhost",
            "http://invalid host",
            "http://inv@lid-host",
            "http://",
            "http://:80",
        ] {
            AllowedHostConfig::parse(invalid).expect_err(invalid);
        }
    }

    #[test]
    fn test_allowed_hosts_respects_allow_all() {
        assert!(AllowedHostsConfig::parse(&["insecure:allow-all"], &dummy_resolver()).is_err());
        assert!(AllowedHostsConfig::parse(
            &["spin.fermyon.dev", "insecure:allow-all"],
            &dummy_resolver()
        )
        .is_err());
    }

    #[test]
    fn test_allowed_all_globs() {
        assert_eq!(
            AllowedHostConfig::new(SchemeConfig::Any, HostConfig::Any, PortConfig::Any),
            AllowedHostConfig::parse("*://*:*").unwrap()
        );
    }

    #[test]
    fn test_missing_scheme() {
        assert!(AllowedHostConfig::parse("example.com").is_err());
    }

    #[test]
    fn test_allowed_hosts_can_be_specific() {
        let allowed = AllowedHostsConfig::parse(
            &["*://spin.fermyon.dev:443", "http://example.com:8383"],
            &dummy_resolver(),
        )
        .unwrap();
        assert!(
            allowed.allows(&OutboundUrl::parse("http://example.com:8383/foo/bar", "http").unwrap())
        );
        // Allow urls with and without a trailing slash
        assert!(allowed.allows(&OutboundUrl::parse("https://spin.fermyon.dev", "https").unwrap()));
        assert!(allowed.allows(&OutboundUrl::parse("https://spin.fermyon.dev/", "https").unwrap()));
        assert!(!allowed.allows(&OutboundUrl::parse("http://example.com/", "http").unwrap()));
        assert!(!allowed.allows(&OutboundUrl::parse("http://google.com/", "http").unwrap()));
        assert!(allowed.allows(&OutboundUrl::parse("spin.fermyon.dev:443", "https").unwrap()));
        assert!(allowed.allows(&OutboundUrl::parse("example.com:8383", "http").unwrap()));
    }

    #[test]
    fn test_allowed_hosts_with_trailing_slash() {
        let allowed =
            AllowedHostsConfig::parse(&["https://my.api.com/"], &dummy_resolver()).unwrap();
        assert!(allowed.allows(&OutboundUrl::parse("https://my.api.com", "https").unwrap()));
        assert!(allowed.allows(&OutboundUrl::parse("https://my.api.com/", "https").unwrap()));
    }

    #[test]
    fn test_allowed_hosts_can_be_subdomain_wildcards() {
        let allowed = AllowedHostsConfig::parse(
            &["http://*.example.com", "http://*.example2.com:8383"],
            &dummy_resolver(),
        )
        .unwrap();
        assert!(
            allowed.allows(&OutboundUrl::parse("http://a.example.com/foo/bar", "http").unwrap())
        );
        assert!(
            allowed.allows(&OutboundUrl::parse("http://a.b.example.com/foo/bar", "http").unwrap())
        );
        assert!(allowed
            .allows(&OutboundUrl::parse("http://a.b.example2.com:8383/foo/bar", "http").unwrap()));
        assert!(!allowed
            .allows(&OutboundUrl::parse("http://a.b.example2.com/foo/bar", "http").unwrap()));
        assert!(!allowed.allows(&OutboundUrl::parse("http://example.com/foo/bar", "http").unwrap()));
        assert!(!allowed
            .allows(&OutboundUrl::parse("http://example.com:8383/foo/bar", "http").unwrap()));
        assert!(
            !allowed.allows(&OutboundUrl::parse("http://myexample.com/foo/bar", "http").unwrap())
        );
    }

    #[test]
    fn test_hash_char_in_db_password() {
        let allowed = AllowedHostsConfig::parse(&["mysql://xyz.com"], &dummy_resolver()).unwrap();
        assert!(
            allowed.allows(&OutboundUrl::parse("mysql://user:pass#word@xyz.com", "mysql").unwrap())
        );
        assert!(allowed
            .allows(&OutboundUrl::parse("mysql://user%3Apass%23word@xyz.com", "mysql").unwrap()));
        assert!(allowed.allows(&OutboundUrl::parse("user%3Apass%23word@xyz.com", "mysql").unwrap()));
    }

    #[test]
    fn test_cidr() {
        let allowed =
            AllowedHostsConfig::parse(&["*://127.0.0.1/24:63551"], &dummy_resolver()).unwrap();
        assert!(allowed.allows(&OutboundUrl::parse("tcp://127.0.0.1:63551", "tcp").unwrap()));
    }

    #[tokio::test]
    async fn validate_service_chaining_for_components_fails() {
        let manifest = toml::toml! {
            spin_manifest_version = 2

            [application]
            name = "test-app"

            [[trigger.test-trigger]]
            component = "empty"

            [component.empty]
            source = "does-not-exist.wasm"
            allowed_outbound_hosts = ["http://another.spin.internal"]

            [[trigger.another-trigger]]
            component = "another"

            [component.another]
            source = "does-not-exist.wasm"

            [[trigger.third-trigger]]
            component = "third"

            [component.third]
            source = "does-not-exist.wasm"
            allowed_outbound_hosts = ["http://*.spin.internal"]
        };
        let locked_app = spin_factors_test::build_locked_app(&manifest)
            .await
            .expect("could not build locked app");
        let app = App::new("unused", locked_app);
        let Err(e) = validate_service_chaining_for_components(&app, &["empty"]) else {
            panic!("Expected service chaining to non-retained component error");
        };
        assert_eq!(
            e.to_string(),
            "Selected component 'empty' cannot use service chaining to unselected component: allowed_outbound_hosts = [\"http://another.spin.internal\"]"
        );
        let Err(e) = validate_service_chaining_for_components(&app, &["third", "another"]) else {
            panic!("Expected wildcard service chaining error");
        };
        assert_eq!(
            e.to_string(),
            "Selected component 'third' cannot use wildcard service chaining: allowed_outbound_hosts = [\"http://*.spin.internal\"]"
        );
        assert!(validate_service_chaining_for_components(&app, &["another"]).is_ok());
    }

    #[tokio::test]
    async fn validate_service_chaining_for_components_with_templated_host_passes() {
        let manifest = toml::toml! {
            spin_manifest_version = 2

            [application]
            name = "test-app"

            [variables]
            host = { default = "test" }

            [[trigger.test-trigger]]
            component = "empty"

            [component.empty]
            source = "does-not-exist.wasm"

            [[trigger.another-trigger]]
            component = "another"

            [component.another]
            source = "does-not-exist.wasm"

            [[trigger.third-trigger]]
            component = "third"

            [component.third]
            source = "does-not-exist.wasm"
            allowed_outbound_hosts = ["http://{{ host }}.spin.internal"]
        };
        let locked_app = spin_factors_test::build_locked_app(&manifest)
            .await
            .expect("could not build locked app");
        let app = App::new("unused", locked_app);
        assert!(validate_service_chaining_for_components(&app, &["empty", "third"]).is_ok());
    }
}
