use serde::{Deserialize, Serialize};
use spin_http_routes::HttpTriggerRouteConfig;

/// Configuration for the HTTP trigger
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HttpTriggerConfig {
    /// HTTP route the component will be invoked for
    pub route: HttpTriggerRouteConfig,
    /// How to handle requests to this route
    #[serde(flatten)]
    pub handler: HttpTriggerHandler,
}

/// Configuration for the HTTP trigger
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", untagged, deny_unknown_fields)]
pub enum HttpTriggerHandler {
    Component {
        /// Component ID to invoke
        component: String,
        /// The HTTP executor the component requires
        #[serde(default)]
        executor: Option<HttpExecutorType>,
    },
    StaticResponse {
        /// Static response to send
        static_response: StaticResponse,
    },
}

impl HttpTriggerHandler {
    pub fn id(&self, trigger_id: &str) -> String {
        match self {
            HttpTriggerHandler::Component { component, .. } => component.clone(),
            HttpTriggerHandler::StaticResponse { .. } => format!("{trigger_id}-static-response"),
        }
    }
}

/// The executor for the HTTP component.
/// The component can either implement the Spin HTTP interface,
/// the `wasi-http` interface, or the Wagi CGI interface.
///
/// If an executor is not specified, the inferred default is `HttpExecutor::Spin`.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "lowercase", tag = "type")]
pub enum HttpExecutorType {
    /// The component implements an HTTP based interface.
    ///
    /// This can be either `fermyon:spin/inbound-http` or `wasi:http/incoming-handler`
    #[default]
    #[serde(alias = "spin")]
    Http,
    /// The component implements the Wagi CGI interface.
    Wagi(WagiTriggerConfig),
}

/// Wagi specific configuration for the http executor.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct WagiTriggerConfig {
    /// The name of the entrypoint. (DEPRECATED)
    #[serde(skip_serializing)]
    pub entrypoint: String,

    /// A string representation of the argv array.
    ///
    /// This should be a space-separate list of strings. The value
    /// ${SCRIPT_NAME} will be replaced with the Wagi SCRIPT_NAME,
    /// and the value ${ARGS} will be replaced with the query parameter
    /// name/value pairs presented as args. For example,
    /// `param1=val1&param2=val2` will become `param1=val1 param2=val2`,
    /// which will then be presented to the program as two arguments
    /// in argv.
    pub argv: String,
}

impl Default for WagiTriggerConfig {
    fn default() -> Self {
        /// This is the default Wagi entrypoint.
        const WAGI_DEFAULT_ENTRYPOINT: &str = "_start";
        const WAGI_DEFAULT_ARGV: &str = "${SCRIPT_NAME} ${ARGS}";

        Self {
            entrypoint: WAGI_DEFAULT_ENTRYPOINT.to_owned(),
            argv: WAGI_DEFAULT_ARGV.to_owned(),
        }
    }
}

/// A static response to be served directly by the host
/// without instantiating a component.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StaticResponse {
    #[serde(default)]
    status_code: Option<u16>,
    #[serde(default)]
    headers: indexmap::IndexMap<String, String>,
    #[serde(default)]
    body: Option<String>,
}

impl StaticResponse {
    pub fn status(&self) -> u16 {
        self.status_code.unwrap_or(200)
    }

    pub fn headers(&self) -> impl Iterator<Item = (&String, &String)> {
        self.headers.iter()
    }

    pub fn body(&self) -> Option<&String> {
        self.body.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wagi_config_smoke_test() {
        let HttpExecutorType::Wagi(config) = toml::toml! { type = "wagi" }.try_into().unwrap()
        else {
            panic!("wrong type");
        };
        assert_eq!(config.entrypoint, "_start");
        assert_eq!(config.argv, "${SCRIPT_NAME} ${ARGS}");
    }
}
