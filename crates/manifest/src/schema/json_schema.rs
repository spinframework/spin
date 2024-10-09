use crate::schema::v2::{ComponentSpec, Map, OneOrManyComponentSpecs};
use schemars::JsonSchema;

// The structs here allow dead code because they exist only
// to represent JSON schemas, and are never instantiated.

#[allow(dead_code)]
#[derive(JsonSchema)]
pub struct TriggerSchema {
    /// HTTP triggers
    #[schemars(default)]
    http: Vec<HttpTriggerSchema>,
    /// Redis triggers
    #[schemars(default)]
    redis: Vec<RedisTriggerSchema>,
}

#[allow(dead_code)]
#[derive(JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct HttpTriggerSchema {
    /// `id = "trigger-id"`
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// `component = ...`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<ComponentSpec>,
    /// `components = { ... }`
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub components: Map<String, OneOrManyComponentSpecs>,
    /// `route = "/user/:name/..."`
    route: HttpRouteSchema,
    /// `executor = { type = "wagi" }
    #[schemars(default, schema_with = "toml_table")]
    executor: Option<toml::Table>,
}

#[allow(dead_code)]
#[derive(JsonSchema)]
#[schemars(untagged)]
pub enum HttpRouteSchema {
    /// The HTTP route that the trigger accepts. The route must begin with a `/``.
    /// The route may contain:
    ///
    /// - Any number of single-segment wildcards, using the syntax `:name`. It matches only a single segment of a path, and allows further matching on segments beyond it.
    ///
    /// - A trailing wildcard, using the syntax `/...`. This matches the given route and any route under it.
    ///
    /// In particular, the route `/...` matches _all_ paths.
    ///
    /// Example: `route = "/user/:name/..."`
    ///
    /// Learn more: https://spinframework.dev/v3/http-trigger#http-trigger-routes
    Route(String),
    /// The trigger does not response to any external HTTP request, but only to requests
    /// via local service chaining.
    ///
    /// Example: `route = { private = true }`
    ///
    /// Learn more: https://spinframework.dev/v3/http-trigger#private-endpoints
    Private(HttpPrivateEndpoint),
}

#[allow(dead_code)]
#[derive(JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct HttpPrivateEndpoint {
    /// Whether the private endpoint is private. This must be true.
    pub private: bool,
}

#[allow(dead_code)]
#[derive(JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct RedisTriggerSchema {
    /// `id = "trigger-id"`
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// `component = ...`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<ComponentSpec>,
    /// `components = { ... }`
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub components: Map<String, OneOrManyComponentSpecs>,
    /// `channel = "my-messages"`
    channel: String,
    /// `address = "redis://redis.example.com:6379"`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    address: Option<String>,
}

/// The SQLite databases which the component is allowed to access. Databases are identified
/// by label e.g. "default" or "analytics". Databases other than "default" must be mapped
/// to a backing store in the runtime config. Use "spin up --sqlite" to run database setup scripts.
///
/// Example: `sqlite_databases = ["default", "my-database"]`
///
/// Learn more: https://spinframework.dev/sqlite-api-guide#preparing-an-sqlite-database
#[allow(dead_code)]
#[derive(JsonSchema)]
#[serde(untagged)]
pub enum SqliteDatabase {
    Label(String),
}

/// The key-value stores which the component is allowed to access. Stores are identified
/// by label e.g. "default" or "customer". Stores other than "default" must be mapped
/// to a backing store in the runtime config.
///
/// Example: `key_value_stores = ["default", "my-store"]`
///
/// Learn more: https://spinframework.dev/kv-store-api-guide#custom-key-value-stores
#[allow(dead_code)]
#[derive(JsonSchema)]
#[serde(untagged)]
pub enum KeyValueStore {
    Label(String),
}

/// The network destinations which the component is allowed to access.
/// Each entry is in the form "(scheme)://(host)[:port]". Each element
/// allows * as a wildcard e.g. "https://\*" (HTTPS on the default port
/// to any destination) or "\*://localhost:\*" (any protocol to any port on
/// localhost). The host part allows segment wildcards for subdomains
/// e.g. "https://\*.example.com". Application variables are allowed using
/// `{{ my_var }}`` syntax.
///
/// Example: `allowed_outbound_hosts = ["redis://myredishost.com:6379"]`
///
/// Learn more: https://spinframework.dev/http-outbound#granting-http-permissions-to-components
#[allow(dead_code)]
#[derive(JsonSchema)]
#[serde(untagged)]
pub enum AllowedOutboundHost {
    Host(String),
}

/// The blob storage containers which the component is allowed to access. Containers are identified
/// by label e.g. "default" or "analytics". Containers must be mapped
/// to a backing store in the runtime config.
///
/// Example: `blob_containers = ["cache", "my-container"]`
#[allow(dead_code)]
#[derive(JsonSchema)]
#[serde(untagged)]
pub enum BlobContainer {
    Label(String),
}

/// The AI models which the component is allowed to access. For local execution, you must
/// download all models; for hosted execution, you should check which models are available
/// in your target environment.
///
/// Example: `ai_models = ["llama2-chat"]`
///
/// Learn more: https://spinframework.dev/serverless-ai-api-guide#using-serverless-ai-from-applications
#[allow(dead_code)]
#[derive(JsonSchema)]
#[serde(untagged)]
pub enum AIModel {
    Label(String),
}

/// Source files to use in `spin watch`. This is a set of paths or glob patterns (relative
/// to the build working directory). A change to any matching file causes
/// `spin watch` to rebuild the application before restarting the application.
///
/// Example: `watch = ["src/**/*.rs"]`
///
/// Learn more: https://spinframework.dev/running-apps#monitoring-applications-for-changes
#[allow(dead_code)]
#[derive(JsonSchema)]
#[serde(untagged)]
pub enum WatchCommand {
    Command(String),
}

pub fn toml_table(_gen: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
    schemars::schema::Schema::Object(schemars::schema::SchemaObject {
        instance_type: Some(schemars::schema::SingleOrVec::Single(Box::new(
            schemars::schema::InstanceType::Object,
        ))),
        ..Default::default()
    })
}

pub fn map_of_toml_tables(_gen: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
    schemars::schema::Schema::Object(schemars::schema::SchemaObject {
        instance_type: Some(schemars::schema::SingleOrVec::Single(Box::new(
            schemars::schema::InstanceType::Object,
        ))),
        ..Default::default()
    })
}

pub fn one_or_many<T: schemars::JsonSchema>(
    gen: &mut schemars::gen::SchemaGenerator,
) -> schemars::schema::Schema {
    schemars::schema::Schema::Object(schemars::schema::SchemaObject {
        subschemas: Some(Box::new(schemars::schema::SubschemaValidation {
            one_of: Some(vec![
                gen.subschema_for::<T>(),
                gen.subschema_for::<Vec<T>>(),
            ]),
            ..Default::default()
        })),
        ..Default::default()
    })
}
