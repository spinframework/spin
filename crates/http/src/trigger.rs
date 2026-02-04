use serde::{Deserialize, Serialize};
use spin_factor_outbound_http::wasi_2023_10_18::ProxyIndices as ProxyIndices2023_10_18;
use spin_factor_outbound_http::wasi_2023_11_10::ProxyIndices as ProxyIndices2023_11_10;
use wasmtime::component::InstancePre;
use wasmtime_wasi::p2::bindings::CommandIndices;
use wasmtime_wasi_http::bindings::ProxyIndices;
use wasmtime_wasi_http::handler::{HandlerState, ProxyHandler, ProxyPre};
use wasmtime_wasi_http::p3::bindings::ServicePre as P3ProxyPre;

use crate::config::HttpExecutorType;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    // The based url
    #[serde(default = "default_base")]
    pub base: String,
}

pub fn default_base() -> String {
    "/".into()
}

/// The type of http handler export used by a component.
pub enum HandlerType<S: HandlerState> {
    Spin,
    Wagi(CommandIndices),
    Wasi0_2(ProxyIndices),
    Wasi0_3(ProxyHandler<S>),
    Wasi2023_11_10(ProxyIndices2023_11_10),
    Wasi2023_10_18(ProxyIndices2023_10_18),
}

/// The `incoming-handler` export for `wasi:http` version rc-2023-10-18
const WASI_HTTP_EXPORT_2023_10_18: &str = "wasi:http/incoming-handler@0.2.0-rc-2023-10-18";
/// The `incoming-handler` export for `wasi:http` version rc-2023-11-10
const WASI_HTTP_EXPORT_2023_11_10: &str = "wasi:http/incoming-handler@0.2.0-rc-2023-11-10";
/// The `incoming-handler` export prefix for all `wasi:http` 0.2 versions
const WASI_HTTP_EXPORT_0_2_PREFIX: &str = "wasi:http/incoming-handler@0.2";
/// The `handler` export `wasi:http` version 0.3.0-rc-2025-08-15
const WASI_HTTP_EXPORT_0_3_0_UNSTABLE: &str = "wasi:http/handler@0.3.0-rc-2026-01-06";
/// The `inbound-http` export for `fermyon:spin`
const SPIN_HTTP_EXPORT: &str = "fermyon:spin/inbound-http";

impl<T, S: HandlerState<StoreData = T>> HandlerType<S> {
    /// Determine the handler type from the exports of a component.
    pub fn from_instance_pre(pre: &InstancePre<T>, handler_state: S) -> anyhow::Result<Self> {
        let mut candidates = Vec::new();
        if let Ok(indices) = ProxyIndices::new(pre) {
            candidates.push(HandlerType::Wasi0_2(indices));
        }
        if let Ok(pre) = P3ProxyPre::new(pre.clone()) {
            candidates.push(HandlerType::Wasi0_3(ProxyHandler::new(
                handler_state,
                ProxyPre::P3(pre),
            )));
        }
        if let Ok(indices) = ProxyIndices2023_10_18::new(pre) {
            candidates.push(HandlerType::Wasi2023_10_18(indices));
        }
        if let Ok(indices) = ProxyIndices2023_11_10::new(pre) {
            candidates.push(HandlerType::Wasi2023_11_10(indices));
        }
        if pre
            .component()
            .get_export_index(None, SPIN_HTTP_EXPORT)
            .is_some()
        {
            candidates.push(HandlerType::Spin);
        }

        match candidates.len() {
            0 => {
                anyhow::bail!(
                    "Expected component to export one of \
                    `{WASI_HTTP_EXPORT_2023_10_18}`, \
                    `{WASI_HTTP_EXPORT_2023_11_10}`, \
                    `{WASI_HTTP_EXPORT_0_2_PREFIX}.*`, \
                    `{WASI_HTTP_EXPORT_0_3_0_UNSTABLE}`, \
                     or `{SPIN_HTTP_EXPORT}` but it exported none of those. \
                     This may mean the component handles a different trigger, or that its `wasi:http` export is newer then those supported by Spin. \
                     If you're sure this is an HTTP module, check if a Spin upgrade is available: this may handle the newer version."
                )
            }
            1 => Ok(candidates.pop().unwrap()),
            _ => anyhow::bail!(
                "component exports multiple different handlers but \
                     it's expected to export only one"
            ),
        }
    }

    /// Validate that the [`HandlerType`] is compatible with the [`HttpExecutorType`].
    pub fn validate_executor(&self, executor: &Option<HttpExecutorType>) -> anyhow::Result<()> {
        match (self, executor) {
            (HandlerType::Wasi0_3(_), Some(HttpExecutorType::Wasip3Unstable)) => {
                terminal::warn!(
                    "You’re using wasip3-unstable, an experimental executor for the \
                    WASI Preview 3 RC interfaces, which will be removed in a future Spin release."
                );
                Ok(())
            }
            (HandlerType::Wasi0_3(_), Some(_) | None) => {
                anyhow::bail!(
                    "`{WASI_HTTP_EXPORT_0_3_0_UNSTABLE}` is currently unstable and will be \
                    removed in a future Spin release. You can opt-in to this unstable interface \
                    by adding `executor = {{ type = \"wasip3-unstable\" }}` to the appropriate \
                    `[[trigger.http]]` section of your spin.toml file."
                );
            }
            (handler_type, Some(HttpExecutorType::Wasip3Unstable)) => {
                anyhow::bail!(
                    "The wasip3-unstable trigger executor expected a component that \
                    exports `{WASI_HTTP_EXPORT_0_3_0_UNSTABLE}` but found a \
                    component with type {name}",
                    name = handler_type.name(),
                )
            }
            (_, _) => Ok(()),
        }
    }

    fn name(&self) -> &str {
        match self {
            HandlerType::Spin => SPIN_HTTP_EXPORT,
            HandlerType::Wasi0_2(_) => WASI_HTTP_EXPORT_0_2_PREFIX,
            HandlerType::Wasi0_3(_) => WASI_HTTP_EXPORT_0_3_0_UNSTABLE,
            HandlerType::Wasi2023_11_10(_) => WASI_HTTP_EXPORT_2023_11_10,
            HandlerType::Wasi2023_10_18(_) => WASI_HTTP_EXPORT_2023_10_18,
            _ => unreachable!(), // WAGI variant will never appear here
        }
    }
}
