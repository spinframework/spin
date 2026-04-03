use super::{
    http,
    http_types::{HttpError, Request, Response},
    Context, TestConfig,
};
use anyhow::{ensure, Result};
use std::collections::HashMap;
use wasmtime::{component::InstancePre, Engine};

#[derive(Default)]
pub(crate) struct Http {
    map: HashMap<String, String>,
}

impl http::Host for Http {
    async fn send_request(
        &mut self,
        req: Request,
    ) -> wasmtime::Result<Result<Response, HttpError>> {
        Ok(self
            .map
            .remove(&req.uri)
            .map(|body| Response {
                status: 200,
                headers: None,
                body: Some(body.into_bytes()),
            })
            .ok_or(HttpError::InvalidUrl))
    }
}

pub(crate) async fn test(
    engine: &Engine,
    test_config: TestConfig,
    pre: &InstancePre<Context>,
) -> Result<(), String> {
    let mut store = super::create_store_with_context(engine, test_config, |context| {
        context
            .http
            .map
            .insert("http://127.0.0.1/test".into(), "Jabberwocky".into());
    });

    super::run_command(
        &mut store,
        pre,
        &["http", "http://127.0.0.1/test"],
        |store| {
            ensure!(
                store.data().http.map.is_empty(),
                "expected module to call `wasi-outbound-http::request` exactly once"
            );

            Ok(())
        },
    )
    .await
}
