use super::{
    Context, TestConfig,
    http_types::{Method, Request, Response},
};
use anyhow::{anyhow, ensure};
use wasmtime::{Engine, component::InstancePre};

pub(crate) async fn test(
    engine: &Engine,
    test_config: TestConfig,
    pre: &InstancePre<Context>,
) -> Result<(), String> {
    super::run(async {
        let mut store = super::create_store(engine, test_config);
        let instance = pre.instantiate_async(&mut store).await?;

        let func = instance
            .get_export_index(&mut store, None, "fermyon:spin/inbound-http")
            .and_then(|i| instance.get_export_index(&mut store, Some(&i), "handle-request"))
            .ok_or_else(|| {
                anyhow!("no fermyon:spin/inbound-http/handle-request function was found")
            })?;
        let func = instance.get_typed_func::<(Request,), (Response,)>(&mut store, &func)?;

        let (response,) = func
            .call_async(
                store,
                (Request {
                    method: Method::Post,
                    uri: "/foo".into(),
                    headers: vec![("foo".into(), "bar".into())],
                    params: vec![],
                    body: Some(b"Hello, SpinHttp!".to_vec()),
                },),
            )
            .await?;

        ensure!(
            response.status == 200,
            "expected response status 200, got {} (body: {:?})",
            response.status,
            response
                .body
                .as_ref()
                .map(|body| String::from_utf8_lossy(body))
        );

        ensure!(
            response
                .headers
                .as_ref()
                .map(|v| v.len() == 1 && "lorem" == &v[0].0.to_lowercase() && "ipsum" == &v[0].1)
                .unwrap_or(false),
            "expected a single response header, \"lorem: ipsum\", got {:?}",
            response.headers
        );

        let expected_body = "dolor sit amet";

        ensure!(
            response.body == Some(expected_body.as_bytes().to_vec()),
            "expected a response body containing the string {expected_body:?}, got {:?}",
            response
                .body
                .as_ref()
                .map(|body| String::from_utf8_lossy(body))
        );

        Ok(())
    })
    .await
}
