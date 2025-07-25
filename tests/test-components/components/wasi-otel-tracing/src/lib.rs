use std::time;

use opentelemetry::{
    global::{self, BoxedTracer, ObjectSafeSpan},
    trace::{TraceContextExt, Tracer},
    Array, Context, ContextGuard, KeyValue, Value,
};
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_wasi::WasiPropagator;
use spin_sdk::{
    http::{IntoResponse, Method, Params, Request, Response, Router},
    http_component,
};

#[http_component]
fn handle(req: Request) -> anyhow::Result<impl IntoResponse> {
    let mut router = Router::new();
    router.get("/nested-spans", nested_spans);
    router.get("/setting-attributes", setting_attributes);
    router.get_async("/host-guest-host", host_guest_host);
    router.get("/events", events);
    router.get("/links", links);
    router.get_async("/root-span", root_span);
    Ok(router.handle(req))
}

fn setup_tracer(propagate_context: bool) -> (BoxedTracer, Option<ContextGuard>) {
    // Set up a tracer using the WASI processor
    let wasi_processor = opentelemetry_wasi::WasiProcessor::new();
    let tracer_provider = SdkTracerProvider::builder()
        .with_span_processor(wasi_processor)
        .build();
    global::set_tracer_provider(tracer_provider);
    let tracer = global::tracer("wasi-otel-tracing");

    if propagate_context {
        let wasi_propagator = opentelemetry_wasi::TraceContextPropagator::new();
        (
            tracer,
            Some(wasi_propagator.extract(&Context::current()).attach()),
        )
    } else {
        (tracer, None)
    }
}

fn nested_spans(_req: Request, _params: Params) -> Response {
    let (tracer, _ctx) = setup_tracer(true);
    tracer.in_span("outer_func", |_| {
        tracer.in_span("inner_func", |_| {});
    });
    Response::new(200, "")
}

fn setting_attributes(_req: Request, _params: Params) -> Response {
    let (tracer, _ctx) = setup_tracer(true);
    tracer.in_span("setting_attributes", |cx| {
        let span = cx.span();
        span.set_attribute(KeyValue::new("foo", "bar"));
        span.set_attribute(KeyValue::new("foo", "baz"));
        span.set_attribute(KeyValue::new(
            "qux",
            Value::Array(Array::String(vec!["qaz".into(), "thud".into()])),
        ));
    });

    Response::new(200, "")
}

async fn host_guest_host(_req: Request, _params: Params) -> Response {
    let (tracer, _ctx) = setup_tracer(true);
    let mut span = tracer.start("guest");
    make_request().await;
    span.end();

    Response::new(200, "")
}

fn events(_req: Request, _params: Params) -> Response {
    let (tracer, _ctx) = setup_tracer(true);
    tracer.in_span("events", |cx| {
        let span = cx.span();
        span.add_event("basic-event".to_string(), vec![]);
        span.add_event(
            "event-with-attributes".to_string(),
            vec![KeyValue::new("foo", true)],
        );
        let time = time::SystemTime::now()
            .duration_since(time::UNIX_EPOCH)
            .unwrap();
        let time = time.as_secs_f64();
        let time = time::Duration::from_secs_f64(time + 1.0);
        let time = time::SystemTime::UNIX_EPOCH + time;
        span.add_event_with_timestamp("event-with-timestamp", time, vec![]);
    });
    Response::new(200, "")
}

fn links(_req: Request, _params: Params) -> Response {
    let (tracer, _ctx) = setup_tracer(true);
    let mut first = tracer.start("first");
    first.end();
    let mut second = tracer.start("second");
    second.add_link(
        first.span_context().clone(),
        vec![KeyValue::new("foo", "bar")],
    );
    second.end();
    Response::new(200, "")
}

async fn root_span(_req: Request, _params: Params) -> Response {
    let (tracer, _ctx) = setup_tracer(false);
    let mut span = tracer.start("root");
    make_request().await;
    span.end();
    Response::new(200, "")
}

async fn make_request() {
    let req = Request::builder()
        .method(Method::Get)
        .uri("https://asdf.com")
        .build();
    let _res: Response = spin_sdk::http::send(req).await.unwrap();
}

// TODO: Test what happens if start is called but not end
// TODO: Test what happens if end is called but not start
// TODO: What happens if child span outlives parent
