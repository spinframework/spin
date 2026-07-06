use serde::{Deserialize, Serialize};
use spin_factor_outbound_http::wasi_2023_10_18::ProxyIndices as ProxyIndices2023_10_18;
use spin_factor_outbound_http::wasi_2023_11_10::ProxyIndices as ProxyIndices2023_11_10;
use spin_factor_outbound_http::wasi_2026_03_15::ServiceIndices as ServiceIndices2026_03_15;
use wasmtime::component::InstancePre;
use wasmtime_wasi::p2::bindings::CommandIndices;
use wasmtime_wasi_http::handler::{HandlerState, ProxyHandler};
use wasmtime_wasi_http::p2::bindings::ProxyIndices;
use wasmtime_wasi_http::p3::bindings::ServicePre;

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
    Wasi2026_03_15(ServiceIndices2026_03_15),
}

impl<S: HandlerState> Clone for HandlerType<S> {
    fn clone(&self) -> Self {
        match self {
            Self::Spin => Self::Spin,
            Self::Wagi(indices) => Self::Wagi(indices.clone()),
            Self::Wasi0_2(indices) => Self::Wasi0_2(indices.clone()),
            Self::Wasi2023_11_10(indices) => Self::Wasi2023_11_10(indices.clone()),
            Self::Wasi2023_10_18(indices) => Self::Wasi2023_10_18(indices.clone()),
            Self::Wasi2026_03_15(indices) => Self::Wasi2026_03_15(indices.clone()),
            Self::Wasi0_3(handler) => Self::Wasi0_3(handler.clone()),
        }
    }
}

/// The `incoming-handler` export for `wasi:http` version 0.2.0-rc-2023-10-18
const WASI_HTTP_EXPORT_2023_10_18: &str = "wasi:http/incoming-handler@0.2.0-rc-2023-10-18";
/// The `incoming-handler` export for `wasi:http` version 0.2.0-rc-2023-11-10
const WASI_HTTP_EXPORT_2023_11_10: &str = "wasi:http/incoming-handler@0.2.0-rc-2023-11-10";
/// The `incoming-handler` export prefix for all `wasi:http` 0.2 versions
const WASI_HTTP_EXPORT_0_2_PREFIX: &str = "wasi:http/incoming-handler@0.2";
/// The `handler` export `wasi:http` version 0.3.0-rc-2025-08-15
const WASI_HTTP_EXPORT_0_3_0_RC_03_15: &str = "wasi:http/handler@0.3.0-rc-2026-03-15";
/// The `handler` export prefix for all `wasi:http` 0.3 versions
const WASI_HTTP_EXPORT_0_3_PREFIX: &str = "wasi:http/handler@0.3";
/// The `inbound-http` export for `fermyon:spin`
const SPIN_HTTP_EXPORT: &str = "fermyon:spin/inbound-http";

impl<T, S: HandlerState<StoreData = T>> HandlerType<S> {
    /// Determine the handler type from the exports of a component.
    pub fn from_instance_pre(pre: &InstancePre<T>, handler_state: S) -> anyhow::Result<Self> {
        let mut candidates = Vec::new();
        if let Ok(indices) = ProxyIndices::new(pre) {
            candidates.push(HandlerType::Wasi0_2(indices));
        }
        if ServicePre::new(pre.clone()).is_ok() {
            candidates.push(HandlerType::Wasi0_3(ProxyHandler::new(handler_state)));
        }
        if let Ok(indices) = ProxyIndices2023_10_18::new(pre) {
            candidates.push(HandlerType::Wasi2023_10_18(indices));
        }
        if let Ok(indices) = ProxyIndices2023_11_10::new(pre) {
            candidates.push(HandlerType::Wasi2023_11_10(indices));
        }
        if let Ok(indices) = ServiceIndices2026_03_15::new(pre) {
            candidates.push(HandlerType::Wasi2026_03_15(indices));
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
                    `{WASI_HTTP_EXPORT_0_3_0_RC_03_15}`, \
                    `{WASI_HTTP_EXPORT_0_3_PREFIX}.*`, \
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
}
