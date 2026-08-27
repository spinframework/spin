use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use spin_serde::KebabId;

use super::{Component, Map, TriggerDependency, json_schema, one_or_many};

/// Trigger configuration. A trigger maps an event of the trigger's type (e.g.
/// an HTTP request on route `/shop`, a Redis message on channel `orders`) to
/// a Spin component.
///
/// The trigger manifest contains additional fields which depend on the trigger
/// type. For the `http` type, these additional fields are `route` (required) and
/// `executor` (optional). For the `redis` type, the additional fields are
/// `channel` (required) and `address` (optional). For other types, see the trigger
/// documentation.
///
/// Learn more: https://spinframework.dev/http-trigger, https://spinframework.dev/redis-trigger
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Trigger {
    /// Optional identifier for the trigger.
    ///
    /// Example: `id = "trigger-id"`
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// The component that Spin should run when the trigger occurs. For HTTP triggers,
    /// this is the HTTP request handler for the trigger route. This is typically
    /// the ID of an entry in the `[component]` table, although you can also write
    /// the component out as the value of this field.
    ///
    /// Example: `component = "shop-handler"`
    ///
    /// Learn more: https://spinframework.dev/triggers#triggers-and-components
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<ComponentSpec>,
    /// Additional components used when the trigger occurs.
    /// The meaning of entries in this table is trigger-specific.
    ///
    /// `components = { ... }`
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub components: Map<String, OneOrManyComponentSpecs>,
    /// Additional components to be invoked during trigger processing.
    /// The meaning of entries in this table is trigger-specific.
    ///
    /// `dependencies = { ... }`
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub dependencies: Map<String, TriggerDependencies>,
    /// Opaque trigger-type-specific config
    #[serde(flatten)]
    pub config: toml::Table,
}

/// One or many `ComponentSpec`(s)
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct OneOrManyComponentSpecs(
    #[serde(with = "one_or_many")]
    #[schemars(schema_with = "json_schema::one_or_many::<ComponentSpec>")]
    pub Vec<ComponentSpec>,
);

/// One or many `ComponentSpec`(s)
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct TriggerDependencies(pub Vec<TriggerDependency>);

/// Component reference or inline definition
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, untagged, try_from = "toml::Value")]
#[schemars(schema_with = "json_schema::id_or_component")]
pub enum ComponentSpec {
    /// `"component-id"`
    Reference(KebabId),
    /// `{ ... }`
    Inline(Box<Component>),
}

impl TryFrom<toml::Value> for ComponentSpec {
    type Error = toml::de::Error;

    fn try_from(value: toml::Value) -> Result<Self, Self::Error> {
        if value.is_str() {
            Ok(ComponentSpec::Reference(KebabId::deserialize(value)?))
        } else {
            Ok(ComponentSpec::Inline(Box::new(Component::deserialize(
                value,
            )?)))
        }
    }
}
