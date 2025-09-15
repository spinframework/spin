//! Route matching for the HTTP trigger.

#![deny(missing_docs)]

use anyhow::{anyhow, Result};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, collections::HashMap, fmt};

/// The prefix for well-known routes.
pub const WELL_KNOWN_PREFIX: &str = "/.well-known/spin/";

/// Router for the HTTP trigger.
#[derive(Clone, Debug)]
pub struct Router {
    /// Resolves paths to routing information - specifically component IDs
    /// but also recording about the original route.
    router: std::sync::Arc<routefinder::Router<RouteHandler>>,
}

/// What a route maps to
#[derive(Clone, Debug)]
struct RouteHandler {
    /// The handler identifier (typically component ID) that the route maps to.
    lookup_key: TriggerLookupKey,
    /// The route, including any application base.
    based_route: Cow<'static, str>,
    /// The route, not including any application base.
    raw_route: Cow<'static, str>,
    /// The route, including any application base and capturing information about whether it has a trailing wildcard.
    /// (This avoids re-parsing the route string.)
    parsed_based_route: ParsedRoute,
}

/// An identifier that can be returned from a RouteMatch and used to look up the trigger
/// that handles the route.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub enum TriggerLookupKey {
    /// The route is handled by the specified Wasm component.
    Component(String),
    /// The route is handled directly within the specified trigger.
    Trigger(String),
}

impl std::fmt::Display for TriggerLookupKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Component(id) => f.write_str(id),
            Self::Trigger(id) => f.write_str(id),
        }
    }
}

/// A detected duplicate route.
#[derive(Debug)] // Needed to call `expect_err` on `Router::build`
pub struct DuplicateRoute {
    /// The duplicated route pattern.
    route: String,
    /// The raw route that was duplicated.
    pub replaced_id: String,
    /// The component ID corresponding to the duplicated route.
    pub effective_id: String,
}

impl Router {
    /// Builds a router based on application configuration.
    ///
    /// `duplicate_routes` is an optional mutable reference to a vector of `DuplicateRoute`
    /// that will be populated with any duplicate routes found during the build process.
    pub fn build<'a>(
        base: &str,
        trigger_routes: impl IntoIterator<Item = (&'a TriggerLookupKey, &'a HttpTriggerRouteConfig)>,
        mut duplicate_routes: Option<&mut Vec<DuplicateRoute>>,
    ) -> Result<Self> {
        // Some information we need to carry between stages of the builder.
        struct RoutingEntry<'a> {
            based_route: String,
            raw_route: &'a str,
            lookup_key: &'a TriggerLookupKey,
        }

        let mut routes: IndexMap<&str, RoutingEntry> = IndexMap::new();

        // Filter out private endpoints and capture the routes.
        let routes_iter = trigger_routes
            .into_iter()
            .filter_map(|(lookup_key, route)| {
                match route {
                    HttpTriggerRouteConfig::Route(raw_route) => {
                        let based_route = sanitize_with_base(base, raw_route);
                        Some(Ok(RoutingEntry { based_route, raw_route, lookup_key }))
                    }
                    HttpTriggerRouteConfig::Private(endpoint) => if endpoint.private {
                        None
                    } else {
                        Some(Err(anyhow!("route must be a string pattern or '{{ private = true }}': '{lookup_key}' has {{ private = false }}")))
                    }
                }
            });

        // Remove duplicates.
        for re in routes_iter {
            let re = re?;
            if let Some(replaced) = routes.insert(re.raw_route, re) {
                if let Some(duplicate_routes) = &mut duplicate_routes {
                    let effective_id = routes
                        .get(replaced.raw_route)
                        .unwrap() // Safe because we just inserted it
                        .lookup_key
                        .to_string();
                    duplicate_routes.push(DuplicateRoute {
                        route: replaced.based_route,
                        replaced_id: replaced.lookup_key.to_string(),
                        effective_id,
                    });
                }
            }
        }

        // Build a `routefinder` from the remaining routes.

        let mut rf = routefinder::Router::new();

        for re in routes.into_values() {
            let (rfroute, parsed) = Self::parse_route(&re.based_route).map_err(|e| {
                anyhow!(
                    "Error parsing route {} associated with component {}: {e}",
                    re.based_route,
                    re.lookup_key,
                )
            })?;

            let handler = RouteHandler {
                lookup_key: re.lookup_key.clone(),
                based_route: re.based_route.into(),
                raw_route: re.raw_route.to_string().into(),
                parsed_based_route: parsed,
            };

            rf.add(rfroute, handler).map_err(|e| anyhow!("{e}"))?;
        }

        let router = Self {
            router: std::sync::Arc::new(rf),
        };

        Ok(router)
    }

    fn parse_route(based_route: &str) -> Result<(routefinder::RouteSpec, ParsedRoute), String> {
        if let Some(wild_suffixed) = based_route.strip_suffix("/...") {
            let rs = format!("{wild_suffixed}/*").try_into()?;
            let parsed = ParsedRoute::trailing_wildcard(wild_suffixed);
            Ok((rs, parsed))
        } else if let Some(wild_suffixed) = based_route.strip_suffix("/*") {
            let rs = based_route.try_into()?;
            let parsed = ParsedRoute::trailing_wildcard(wild_suffixed);
            Ok((rs, parsed))
        } else {
            let rs = based_route.try_into()?;
            let parsed = ParsedRoute::exact(based_route);
            Ok((rs, parsed))
        }
    }

    /// Returns the constructed routes.
    pub fn routes(
        &self,
    ) -> impl Iterator<Item = (&(impl fmt::Display + fmt::Debug), &TriggerLookupKey)> {
        self.router
            .iter()
            .map(|(_spec, handler)| (&handler.parsed_based_route, &handler.lookup_key))
    }

    /// true if one or more routes is under the reserved `/.well-known/spin/*`
    /// prefix; otherwise false.
    pub fn contains_reserved_route(&self) -> bool {
        self.router
            .iter()
            .any(|(_spec, handler)| handler.based_route.starts_with(crate::WELL_KNOWN_PREFIX))
    }

    /// This returns the component ID that should handle the given path, or an error
    /// if no component matches.
    ///
    /// If multiple components could potentially handle the same request based on their
    /// defined routes, components with matching exact routes take precedence followed
    /// by matching wildcard patterns with the longest matching prefix.
    pub fn route<'path, 'router: 'path>(
        &'router self,
        path: &'path str,
    ) -> Result<RouteMatch<'router, 'path>> {
        let best_match = self
            .router
            .best_match(path)
            .ok_or_else(|| anyhow!("Cannot match route for path {path}"))?;

        let route_handler = best_match.handler();
        let captures = best_match.captures();

        Ok(RouteMatch {
            inner: RouteMatchKind::Real {
                route_handler,
                captures,
                path,
            },
        })
    }
}

impl DuplicateRoute {
    /// The duplicated route pattern.
    pub fn route(&self) -> &str {
        if self.route.is_empty() {
            "/"
        } else {
            &self.route
        }
    }
}

#[derive(Clone, Debug)]
enum ParsedRoute {
    Exact(String),
    TrailingWildcard(String),
}

impl ParsedRoute {
    fn exact(route: impl Into<String>) -> Self {
        Self::Exact(route.into())
    }

    fn trailing_wildcard(route: impl Into<String>) -> Self {
        Self::TrailingWildcard(route.into())
    }
}

impl fmt::Display for ParsedRoute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self {
            ParsedRoute::Exact(path) => write!(f, "{path}"),
            ParsedRoute::TrailingWildcard(pattern) => write!(f, "{pattern} (wildcard)"),
        }
    }
}

/// A routing match for a URL.
pub struct RouteMatch<'router, 'path> {
    inner: RouteMatchKind<'router, 'path>,
}

impl RouteMatch<'_, '_> {
    /// A synthetic match as if the given path was matched against the wildcard route.
    /// Used in service chaining; always directs to a component.
    pub fn synthetic(component_id: String, path: String) -> Self {
        Self {
            inner: RouteMatchKind::Synthetic {
                route_handler: RouteHandler {
                    lookup_key: TriggerLookupKey::Component(component_id),
                    based_route: "/...".into(),
                    raw_route: "/...".into(),
                    parsed_based_route: ParsedRoute::TrailingWildcard(String::new()),
                },
                trailing_wildcard: path,
            },
        }
    }

    /// An identifier for looking up the matched handler.
    pub fn lookup_key(&self) -> &TriggerLookupKey {
        &self.inner.route_handler().lookup_key
    }

    /// The matched route, as originally written in the manifest, combined with the base.
    pub fn based_route(&self) -> &str {
        &self.inner.route_handler().based_route
    }

    /// The matched route, excluding any trailing wildcard, combined with the base.
    pub fn based_route_or_prefix(&self) -> &str {
        self.inner
            .route_handler()
            .based_route
            .strip_suffix("/...")
            .unwrap_or(&self.inner.route_handler().based_route)
    }

    /// The matched route, as originally written in the manifest.
    pub fn raw_route(&self) -> &str {
        &self.inner.route_handler().raw_route
    }

    /// The matched route, excluding any trailing wildcard.
    pub fn raw_route_or_prefix(&self) -> &str {
        self.inner
            .route_handler()
            .raw_route
            .strip_suffix("/...")
            .unwrap_or(&self.inner.route_handler().raw_route)
    }

    /// The named wildcards captured from the path, if any
    pub fn named_wildcards(&self) -> HashMap<&str, &str> {
        self.inner.named_wildcards()
    }

    /// The trailing wildcard part of the path, if any
    pub fn trailing_wildcard(&self) -> Cow<'_, str> {
        self.inner.trailing_wildcard()
    }
}

/// The kind of route match that was made.
///
/// Can either be real based on the routefinder or synthetic based on hardcoded results.
enum RouteMatchKind<'router, 'path> {
    /// A synthetic match as if the given path was matched against the wildcard route.
    Synthetic {
        /// The route handler that matched the path.
        route_handler: RouteHandler,
        /// The trailing wildcard part of the path
        trailing_wildcard: String,
    },
    /// A real match.
    Real {
        /// The route handler that matched the path.
        route_handler: &'router RouteHandler,
        /// The best match for the path.
        captures: routefinder::Captures<'router, 'path>,
        /// The path that was matched.
        path: &'path str,
    },
}

impl RouteMatchKind<'_, '_> {
    /// The route handler that matched the path.
    fn route_handler(&self) -> &RouteHandler {
        match self {
            RouteMatchKind::Synthetic { route_handler, .. } => route_handler,
            RouteMatchKind::Real { route_handler, .. } => route_handler,
        }
    }

    /// The named wildcards captured from the path, if any
    pub fn named_wildcards(&self) -> HashMap<&str, &str> {
        let Self::Real { captures, .. } = &self else {
            return HashMap::new();
        };
        captures.iter().collect()
    }

    /// The trailing wildcard part of the path, if any
    pub fn trailing_wildcard(&self) -> Cow<'_, str> {
        let (captures, path) = match self {
            // If we have a synthetic match, we already have the trailing wildcard.
            Self::Synthetic {
                trailing_wildcard, ..
            } => return trailing_wildcard.into(),
            Self::Real { captures, path, .. } => (captures, path),
        };

        captures
            .wildcard()
            .map(|s|
            // Backward compatibility considerations - Spin has traditionally
            // captured trailing slashes, but routefinder does not.
            match (s.is_empty(), path.ends_with('/')) {
                // route: /foo/..., path: /foo
                (true, false) => s.into(),
                // route: /foo/..., path: /foo/
                (true, true) => "/".into(),
                // route: /foo/..., path: /foo/bar
                (false, false) => format!("/{s}").into(),
                // route: /foo/..., path: /foo/bar/
                (false, true) => format!("/{s}/").into(),
            })
            .unwrap_or_default()
    }
}

/// Sanitizes the base and path and return a formed path.
fn sanitize_with_base<S: Into<String>>(base: S, path: S) -> String {
    let path = absolutize(path);

    format!("{}{}", sanitize(base.into()), sanitize(path))
}

fn absolutize<S: Into<String>>(s: S) -> String {
    let s = s.into();
    if s.starts_with('/') {
        s
    } else {
        format!("/{s}")
    }
}

/// Strips the trailing slash from a string.
fn sanitize<S: Into<String>>(s: S) -> String {
    let s = s.into();
    // TODO
    // This only strips a single trailing slash.
    // Should we attempt to strip all trailing slashes?
    match s.strip_suffix('/') {
        Some(s) => s.into(),
        None => s,
    }
}

/// An HTTP trigger route
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum HttpTriggerRouteConfig {
    /// A route that is routable.
    Route(String),
    /// A route that is not routable, but indicates a private endpoint.
    Private(HttpPrivateEndpoint),
}

/// Indicates that a trigger is a private endpoint (not routable).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HttpPrivateEndpoint {
    /// Whether the private endpoint is private. This must be true.
    pub private: bool,
}

impl Default for HttpTriggerRouteConfig {
    fn default() -> Self {
        Self::Route(Default::default())
    }
}

impl<T: Into<String>> From<T> for HttpTriggerRouteConfig {
    fn from(value: T) -> Self {
        Self::Route(value.into())
    }
}

#[cfg(test)]
mod route_tests {
    use super::*;

    fn component_key(value: &str) -> TriggerLookupKey {
        TriggerLookupKey::Component(value.to_string())
    }

    fn trigger_key(value: &str) -> TriggerLookupKey {
        TriggerLookupKey::Trigger(value.to_string())
    }

    impl TriggerLookupKey {
        fn component_id(&self) -> &str {
            match self {
                TriggerLookupKey::Component(id) => id,
                TriggerLookupKey::Trigger(_) => {
                    panic!("expected component ref but was trigger ref")
                }
            }
        }
    }

    /// Produces a router using component routes only
    fn component_router<'a>(
        base: &str,
        components: impl IntoIterator<Item = (&'a str, &'a str)>,
        duplicate_routes: Option<&mut Vec<DuplicateRoute>>,
    ) -> anyhow::Result<Router> {
        let owned_routes = components
            .into_iter()
            .map(|(cid, path)| (component_key(cid), HttpTriggerRouteConfig::from(path)))
            .collect::<Vec<_>>();
        let routes = owned_routes.iter().map(|(k, v)| (k, v)); // Yes, I'm afraid this is necessary

        Router::build(base, routes, duplicate_routes)
    }

    impl RouteMatch<'_, '_> {
        fn component_id(&self) -> &str {
            self.lookup_key().component_id()
        }
    }

    #[test]
    fn test_router_exact() -> Result<()> {
        let r = component_router("/", [("foo", "/foo"), ("foobar", "/foo/bar")], None)?;

        assert_eq!(r.route("/foo")?.component_id(), "foo");
        assert_eq!(r.route("/foo/bar")?.component_id(), "foobar");
        Ok(())
    }

    #[test]
    fn router_returns_trigger_or_component() -> Result<()> {
        let r = Router::build(
            "/",
            [
                (&component_key("compy"), &"/foo".into()),
                (&trigger_key("triggy"), &"/foo/bar".into()),
            ],
            None,
        )?;

        assert!(
            matches!(r.route("/foo")?.lookup_key(), TriggerLookupKey::Component(c) if c == "compy")
        );
        assert!(
            matches!(r.route("/foo/bar")?.lookup_key(), TriggerLookupKey::Trigger(t) if t == "triggy")
        );
        Ok(())
    }

    #[test]
    fn test_router_respects_base() -> Result<()> {
        let r = component_router("/base", [("foo", "/foo"), ("foobar", "/foo/bar")], None)?;

        assert_eq!(r.route("/base/foo")?.component_id(), "foo");
        assert_eq!(r.route("/base/foo/bar")?.component_id(), "foobar");
        Ok(())
    }

    #[test]
    fn test_router_wildcard() -> Result<()> {
        let r = component_router("/", [("all", "/...")], None)?;

        assert_eq!(r.route("/foo/bar")?.component_id(), "all");
        assert_eq!(r.route("/abc/")?.component_id(), "all");
        assert_eq!(r.route("/")?.component_id(), "all");
        assert_eq!(
            r.route("/this/should/be/captured?abc=def")?.component_id(),
            "all"
        );
        Ok(())
    }

    #[test]
    fn wildcard_routes_use_custom_display() {
        let routes = component_router("/", vec![("comp", "/whee/...")], None).unwrap();

        let (route, rh) = routes.routes().next().unwrap();

        assert_eq!("comp", rh.component_id());
        assert_eq!("/whee (wildcard)", format!("{route}"));
    }

    #[test]
    fn test_router_respects_longest_match() -> Result<()> {
        let r = component_router(
            "/",
            [
                ("one_wildcard", "/one/..."),
                ("onetwo_wildcard", "/one/two/..."),
                ("onetwothree_wildcard", "/one/two/three/..."),
            ],
            None,
        )?;

        assert_eq!(
            r.route("/one/two/three/four")?.component_id(),
            "onetwothree_wildcard"
        );

        // ...regardless of order
        let r = component_router(
            "/",
            [
                ("onetwothree_wildcard", "/one/two/three/..."),
                ("onetwo_wildcard", "/one/two/..."),
                ("one_wildcard", "/one/..."),
            ],
            None,
        )?;

        assert_eq!(
            r.route("/one/two/three/four")?.component_id(),
            "onetwothree_wildcard"
        );
        Ok(())
    }

    #[test]
    fn test_router_exact_beats_wildcard() -> Result<()> {
        let r = component_router("/", [("one_exact", "/one"), ("wildcard", "/...")], None)?;

        assert_eq!(r.route("/one")?.component_id(), "one_exact");

        Ok(())
    }

    #[test]
    fn sensible_routes_are_reachable() {
        let mut duplicates = Vec::new();
        let routes = component_router(
            "/",
            [
                ("/", "/"),
                ("/foo", "/foo"),
                ("/bar", "/bar"),
                ("/whee/...", "/whee/..."),
            ],
            Some(&mut duplicates),
        )
        .unwrap();

        assert_eq!(4, routes.routes().count());
        assert_eq!(0, duplicates.len());
    }

    #[test]
    fn order_of_reachable_routes_is_preserved() {
        let routes = component_router(
            "/",
            [
                ("comp-/", "/"),
                ("comp-/foo", "/foo"),
                ("comp-/bar", "/bar"),
                ("comp-/whee/...", "/whee/..."),
            ],
            None,
        )
        .unwrap();

        assert_eq!("comp-/", routes.routes().next().unwrap().1.component_id());
        assert_eq!(
            "comp-/foo",
            routes.routes().nth(1).unwrap().1.component_id()
        );
        assert_eq!(
            "comp-/bar",
            routes.routes().nth(2).unwrap().1.component_id()
        );
        assert_eq!(
            "comp-/whee/...",
            routes.routes().nth(3).unwrap().1.component_id()
        );
    }

    #[test]
    fn duplicate_routes_are_unreachable() {
        let mut duplicates = Vec::new();
        let routes = component_router(
            "/",
            [
                ("comp-/", "/"),
                ("comp-first /foo", "/foo"),
                ("comp-second /foo", "/foo"),
                ("comp-/whee/...", "/whee/..."),
            ],
            Some(&mut duplicates),
        )
        .unwrap();

        assert_eq!(3, routes.routes().count());
        assert_eq!(1, duplicates.len());
    }

    #[test]
    fn duplicate_routes_last_one_wins() {
        let mut duplicates = Vec::new();
        let routes = component_router(
            "/",
            [
                ("comp-/", "/"),
                ("comp-first /foo", "/foo"),
                ("comp-second /foo", "/foo"),
                ("comp-/whee/...", "/whee/..."),
            ],
            Some(&mut duplicates),
        )
        .unwrap();

        assert_eq!(
            "comp-second /foo",
            routes.routes().nth(1).unwrap().1.component_id()
        );
        assert_eq!("comp-first /foo", duplicates[0].replaced_id);
        assert_eq!("comp-second /foo", duplicates[0].effective_id);
    }

    #[test]
    fn duplicate_routes_reporting_is_faithful() {
        let mut duplicates = Vec::new();
        let _ = component_router(
            "/",
            [
                ("comp-first /", "/"),
                ("comp-second /", "/"),
                ("comp-first /foo", "/foo"),
                ("comp-second /foo", "/foo"),
                ("comp-first /...", "/..."),
                ("comp-second /...", "/..."),
                ("comp-first /whee/...", "/whee/..."),
                ("comp-second /whee/...", "/whee/..."),
            ],
            Some(&mut duplicates),
        )
        .unwrap();

        assert_eq!("comp-first /", duplicates[0].replaced_id);
        assert_eq!("/", duplicates[0].route());

        assert_eq!("comp-first /foo", duplicates[1].replaced_id);
        assert_eq!("/foo", duplicates[1].route());

        assert_eq!("comp-first /...", duplicates[2].replaced_id);
        assert_eq!("/...", duplicates[2].route());

        assert_eq!("comp-first /whee/...", duplicates[3].replaced_id);
        assert_eq!("/whee/...", duplicates[3].route());
    }

    #[test]
    fn unroutable_routes_are_skipped() {
        let routes = Router::build(
            "/",
            [
                (&component_key("comp-/"), &"/".into()),
                (&component_key("comp-/foo"), &"/foo".into()),
                (
                    &component_key("comp-private"),
                    &HttpTriggerRouteConfig::Private(HttpPrivateEndpoint { private: true }),
                ),
                (&component_key("comp-/whee/..."), &"/whee/...".into()),
            ],
            None,
        )
        .unwrap();

        assert_eq!(3, routes.routes().count());
        assert!(!routes
            .routes()
            .any(|(_r, tcr)| tcr.component_id() == "comp-private"));
    }

    #[test]
    fn unroutable_routes_have_to_be_unroutable_thats_just_common_sense() {
        let e = Router::build(
            "/",
            vec![
                (&component_key("comp-/"), &"/".into()),
                (&component_key("comp-/foo"), &"/foo".into()),
                (
                    &component_key("comp-bad component"),
                    &HttpTriggerRouteConfig::Private(HttpPrivateEndpoint { private: false }),
                ),
                (&component_key("comp-/whee/..."), &"/whee/...".into()),
            ],
            None,
        )
        .expect_err("should not have accepted a 'route = true'");

        assert!(e.to_string().contains("comp-bad component"));
    }

    #[test]
    fn trailing_wildcard_is_captured() {
        let routes = component_router("/", [("comp", "/...")], None).unwrap();
        let m = routes.route("/1/2/3").expect("/1/2/3 should have matched");
        assert_eq!("/1/2/3", m.trailing_wildcard());

        let routes = component_router("/", [("comp", "/1/...")], None).unwrap();
        let m = routes.route("/1/2/3").expect("/1/2/3 should have matched");
        assert_eq!("/2/3", m.trailing_wildcard());
    }

    #[test]
    fn trailing_wildcard_respects_trailing_slash() {
        // We test this because it is the existing Spin behaviour but is *not*
        // how routefinder behaves by default (routefinder prefers to ignore trailing
        // slashes).
        let routes = component_router("/", [("comp", "/test/...")], None).unwrap();
        let m = routes.route("/test").expect("/test should have matched");
        assert_eq!("", m.trailing_wildcard());
        let m = routes.route("/test/").expect("/test/ should have matched");
        assert_eq!("/", m.trailing_wildcard());
        let m = routes
            .route("/test/hello")
            .expect("/test/hello should have matched");
        assert_eq!("/hello", m.trailing_wildcard());
        let m = routes
            .route("/test/hello/")
            .expect("/test/hello/ should have matched");
        assert_eq!("/hello/", m.trailing_wildcard());
    }

    #[test]
    fn named_wildcard_is_captured() {
        let routes = component_router("/", [("comp", "/1/:two/3")], None).unwrap();
        let m = routes.route("/1/2/3").expect("/1/2/3 should have matched");
        assert_eq!("2", m.named_wildcards()["two"]);

        let routes = component_router("/", [("comp", "/1/:two/...")], None).unwrap();
        let m = routes.route("/1/2/3").expect("/1/2/3 should have matched");
        assert_eq!("2", m.named_wildcards()["two"]);
    }

    #[test]
    fn reserved_routes_are_reserved() {
        let routes = component_router("/", [("comp", "/.well-known/spin/...")], None).unwrap();
        assert!(routes.contains_reserved_route());

        let routes = component_router("/", [("comp", "/.well-known/spin/fie")], None).unwrap();
        assert!(routes.contains_reserved_route());
    }

    #[test]
    fn unreserved_routes_are_unreserved() {
        let routes = component_router("/", [("comp", "/.well-known/spindle/...")], None).unwrap();
        assert!(!routes.contains_reserved_route());

        let routes = component_router("/", [("comp", "/.well-known/spi/...")], None).unwrap();
        assert!(!routes.contains_reserved_route());

        let routes = component_router("/", [("comp", "/.well-known/spin")], None).unwrap();
        assert!(!routes.contains_reserved_route());
    }
}
