pub use deny::apply_deny_adapter;
mod deny;

/// Specifies which host capabilities a component dependency is allowed to inherit
/// from its parent component.
///
/// When a dependency is composed into a parent component, it may need access to
/// host-provided interfaces such as networking, key-value stores, or environment
/// variables. This enum controls which of those capability sets are "allowed through"
/// (i.e. not denied by the deny adapter).
pub enum InheritConfiguration {
    /// Inherit all capabilities from the parent component.
    All,
    /// Inherit no capabilities; the deny adapter blocks every host interface.
    None,
    /// Inherit only the named capability sets (e.g. `"allowed_outbound_hosts"`,
    /// `"key_value_stores"`). Unrecognized names are silently ignored.
    Some(Vec<String>),
}

const CAPABILITY_SETS: &[(&str, &[&str])] = &[
    ("ai_models", AI_MODELS),
    ("allowed_outbound_hosts", ALLOWED_OUTBOUND_HOSTS),
    ("environment", ENVIRONMENT),
    ("files", FILES),
    ("key_value_stores", KEY_VALUE_STORES),
    ("sqlite_databases", SQLITE_DATABASES),
    ("variables", VARIABLES),
];

const AI_MODELS: &[&str] = &["fermyon:spin/llm@2.0.0"];

const ALLOWED_OUTBOUND_HOSTS: &[&str] = &[
    "fermyon:spin/mqtt@2.0.0",
    "fermyon:spin/redis@2.0.0",
    "spin:mqtt/mqtt@3.0.0",
    "spin:redis/redis@3.0.0",
    "wasi:http/client@0.3.0-rc-2026-03-15",
    "wasi:http/outgoing-handler@0.2.6",
    "wasi:sockets/ip-name-lookup@0.2.6",
    "wasi:sockets/ip-name-lookup@0.3.0-rc-2026-03-15",
    "wasi:sockets/tcp-create-socket@0.2.6",
    "wasi:sockets/tcp@0.2.6",
    "wasi:sockets/udp-create-socket@0.2.6",
    "wasi:sockets/udp@0.2.6",
];

const ENVIRONMENT: &[&str] = &[
    "wasi:cli/environment@0.2.6",
    "wasi:cli/environment@0.3.0-rc-2026-03-15",
];

const FILES: &[&str] = &[
    "wasi:filesystem/preopens@0.2.6",
    "wasi:filesystem/preopens@0.3.0-rc-2026-03-15",
];

const KEY_VALUE_STORES: &[&str] = &[
    "fermyon:spin/key-value@2.0.0",
    "spin:key-value/key-value@3.0.0",
    "wasi:keyvalue/store@0.2.0-draft2",
];

const SQLITE_DATABASES: &[&str] = &[
    "fermyon:spin/mysql@2.0.0",
    "fermyon:spin/postgres@2.0.0",
    "fermyon:spin/sqlite@2.0.0",
    "spin:postgres/postgres@3.0.0",
    "spin:postgres/postgres@4.2.0",
    "spin:sqlite/sqlite@3.1.0",
];

const VARIABLES: &[&str] = &[
    "fermyon:spin/variables@2.0.0",
    "spin:variables/variables@3.0.0",
];
