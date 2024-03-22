use spin_sdk::{
    http::{Method, Params, Request, Response, Router},
    http_component,
};

#[http_component]
fn handle(req: http::Request<()>) -> Response {
    let mut router = Router::new();
    router.get_async("/one", one);
    router.get_async("/two", two);
    router.handle(req)
}

async fn one(_req: Request, _params: Params) -> Response {
    let req = Request::builder().method(Method::Get).uri("/two").build();
    let _res: Response = spin_sdk::http::send(req).await.unwrap();
    Response::new(200, "")
}

async fn two(_req: Request, _params: Params) -> Response {
    Response::new(201, "")
}
