use bytes::Bytes;
use spin_sdk::http::body::IncomingBodyExt;
use spin_sdk::http::{self, EmptyBody, FullBody, IntoResponse, Request, Response};
use spin_sdk::http_service;
use spin_sdk::variables;

/// Path that GitHub redirects back to after the user authorizes the app.
const CALLBACK_PATH: &str = "/auth/callback";
/// Cookie that stores the GitHub access token once the user is authenticated.
const TOKEN_COOKIE: &str = "gh_token";
/// Cookie that stores the anti-CSRF `state` value during the login round-trip.
const STATE_COOKIE: &str = "gh_oauth_state";
/// Header used to pass the authenticated GitHub login to the downstream component.
const FORWARDED_USER_HEADER: &str = "x-github-user";

const GITHUB_AUTHORIZE_URL: &str = "https://github.com/login/oauth/authorize";
const GITHUB_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const GITHUB_USER_URL: &str = "https://api.github.com/user";
const OAUTH_SCOPE: &str = "read:user";
const USER_AGENT: &str = "spin-github-oauth-middleware";

#[derive(serde::Deserialize)]
struct AccessTokenResponse {
    access_token: Option<String>,
    error: Option<String>,
}

#[derive(serde::Deserialize)]
struct GithubUser {
    login: String,
}

#[http_service]
async fn handle(mut req: Request) -> http::Result<Response> {
    // Request runs on the way in, before the next handler.
    eprintln!("[middleware] --> {} {}", req.method(), req.uri().path());

    // Strip runtime-managed headers so forwarding doesn't hit `Forbidden`.
    strip_forbidden_headers(&mut req);

    let path = req.uri().path().to_owned();

    // 1. Handle the OAuth callback coming back from GitHub.
    if path == CALLBACK_PATH {
        return handle_callback(&req).await;
    }

    // 2. If a session token is present, validate it against GitHub and, when
    //    valid, forward the request with the authenticated user attached.
    if let Some(token) = cookie_value(&req, TOKEN_COOKIE) {
        match fetch_github_login(&token).await {
            Ok(login) => {
                eprintln!("[middleware] authenticated GitHub user: {login}");
                // Overwrite (never append) so a client can't spoof the header.
                if let Ok(value) = http::HeaderValue::from_str(&login) {
                    req.headers_mut()
                        .insert(http::HeaderName::from_static(FORWARDED_USER_HEADER), value);
                }

                let resp = http::next(req).await?;
                eprintln!("[middleware] <-- {}", resp.status());
                return Ok(resp);
            }
            Err(_) => {
                eprintln!("[middleware] stored token rejected; restarting login flow");
            }
        }
    }

    // 3. Not authenticated: kick off the GitHub OAuth login flow.
    start_login().await
}

/// Start the OAuth flow by redirecting the user to GitHub's authorize endpoint.
///
/// A random `state` value is generated for CSRF protection and stored in a
/// short-lived cookie so it can be verified when GitHub calls back.
async fn start_login() -> http::Result<Response> {
    let client_id = match get_var("github_client_id").await {
        Ok(value) => value,
        Err(status) => return deny(status),
    };
    let redirect_uri = match get_var("github_redirect_uri").await {
        Ok(value) => value,
        Err(status) => return deny(status),
    };

    let state = random_state();
    let authorize_url = format!(
        "{GITHUB_AUTHORIZE_URL}?client_id={}&redirect_uri={}&scope={}&state={}",
        urlencode(&client_id),
        urlencode(&redirect_uri),
        urlencode(OAUTH_SCOPE),
        urlencode(&state),
    );

    let response = http::Response::builder()
        .status(http::StatusCode::FOUND)
        .header("location", authorize_url)
        .header(
            "set-cookie",
            format!("{STATE_COOKIE}={state}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=600"),
        )
        .body(EmptyBody::new())?;

    short_circuit(response)
}

/// Handle GitHub's redirect back to `CALLBACK_PATH`, exchanging the `code` for
/// an access token and storing it in a session cookie.
async fn handle_callback(req: &Request) -> http::Result<Response> {
    let query = req.uri().query().unwrap_or_default();

    let Some(code) = query_param(query, "code") else {
        eprintln!("[middleware] callback missing `code`");
        return deny(http::StatusCode::BAD_REQUEST);
    };
    let Some(state) = query_param(query, "state") else {
        eprintln!("[middleware] callback missing `state`");
        return deny(http::StatusCode::BAD_REQUEST);
    };

    // CSRF protection: the returned state must match the cookie we set earlier.
    let Some(expected_state) = cookie_value(req, STATE_COOKIE) else {
        eprintln!("[middleware] callback missing state cookie");
        return deny(http::StatusCode::BAD_REQUEST);
    };
    if !constant_time_eq(state.as_bytes(), expected_state.as_bytes()) {
        eprintln!("[middleware] OAuth state mismatch");
        return deny(http::StatusCode::BAD_REQUEST);
    }

    let token = match exchange_code(&code).await {
        Ok(token) => token,
        Err(status) => return deny(status),
    };

    // Persist the token in a cookie, clear the one-time state cookie, and send
    // the user back to the application root.
    let response = http::Response::builder()
        .status(http::StatusCode::FOUND)
        .header("location", "/")
        .header(
            "set-cookie",
            format!("{TOKEN_COOKIE}={token}; Path=/; HttpOnly; Secure; SameSite=Lax"),
        )
        .header(
            "set-cookie",
            format!("{STATE_COOKIE}=; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=0"),
        )
        .body(EmptyBody::new())?;

    short_circuit(response)
}

/// Exchange an authorization `code` for a GitHub access token.
async fn exchange_code(code: &str) -> Result<String, http::StatusCode> {
    let client_id = get_var("github_client_id").await?;
    let client_secret = get_var("github_client_secret").await?;
    let redirect_uri = get_var("github_redirect_uri").await?;

    let form = format!(
        "client_id={}&client_secret={}&code={}&redirect_uri={}",
        urlencode(&client_id),
        urlencode(&client_secret),
        urlencode(code),
        urlencode(&redirect_uri),
    );

    let outgoing = Request::post(GITHUB_TOKEN_URL)
        .header("accept", "application/json")
        .header("content-type", "application/x-www-form-urlencoded")
        .header("user-agent", USER_AGENT)
        .body(FullBody::new(Bytes::from(form)))
        .map_err(|e| {
            eprintln!("[middleware] failed to build token request: {e}");
            http::StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let response = http::send(outgoing).await.map_err(|e| {
        eprintln!("[middleware] token request failed: {e}");
        http::StatusCode::BAD_GATEWAY
    })?;

    if !response.status().is_success() {
        eprintln!("[middleware] token endpoint returned {}", response.status());
        return Err(http::StatusCode::BAD_GATEWAY);
    }

    let body = response.into_body().bytes().await.map_err(|e| {
        eprintln!("[middleware] failed to read token response: {e}");
        http::StatusCode::BAD_GATEWAY
    })?;

    let parsed: AccessTokenResponse = serde_json::from_slice(&body).map_err(|e| {
        eprintln!("[middleware] failed to parse token response: {e}");
        http::StatusCode::BAD_GATEWAY
    })?;

    if let Some(err) = parsed.error {
        eprintln!("[middleware] GitHub OAuth error: {err}");
        return Err(http::StatusCode::UNAUTHORIZED);
    }

    parsed.access_token.ok_or_else(|| {
        eprintln!("[middleware] token response missing access_token");
        http::StatusCode::BAD_GATEWAY
    })
}

/// Look up the authenticated user's login by calling the GitHub API. A failure
/// here means the token is missing, expired, or revoked.
async fn fetch_github_login(token: &str) -> Result<String, http::StatusCode> {
    let outgoing = Request::get(GITHUB_USER_URL)
        .header("accept", "application/vnd.github+json")
        .header("authorization", format!("Bearer {token}"))
        .header("user-agent", USER_AGENT)
        .body(EmptyBody::new())
        .map_err(|e| {
            eprintln!("[middleware] failed to build user request: {e}");
            http::StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let response = http::send(outgoing).await.map_err(|e| {
        eprintln!("[middleware] user request failed: {e}");
        http::StatusCode::BAD_GATEWAY
    })?;

    if !response.status().is_success() {
        eprintln!("[middleware] user endpoint returned {}", response.status());
        return Err(http::StatusCode::UNAUTHORIZED);
    }

    let body = response.into_body().bytes().await.map_err(|e| {
        eprintln!("[middleware] failed to read user response: {e}");
        http::StatusCode::BAD_GATEWAY
    })?;

    let user: GithubUser = serde_json::from_slice(&body).map_err(|e| {
        eprintln!("[middleware] failed to parse user response: {e}");
        http::StatusCode::BAD_GATEWAY
    })?;

    Ok(user.login)
}

/// Fetch an application variable, mapping any failure to a `500` status.
async fn get_var(name: &str) -> Result<String, http::StatusCode> {
    variables::get(name).await.map_err(|e| {
        eprintln!("[middleware] missing application variable `{name}`: {e}");
        http::StatusCode::INTERNAL_SERVER_ERROR
    })
}

/// Return a prepared response (e.g. a redirect) to the caller instead of
/// forwarding to the next handler.
fn short_circuit(resp: http::Response<EmptyBody>) -> http::Result<Response> {
    Err(resp.into_response()?.into())
}

/// Short-circuit the middleware chain with a bare status code.
fn deny(status: http::StatusCode) -> http::Result<Response> {
    Err(status.into_response()?.into())
}

/// Read the value of a single cookie from the request's `Cookie` header.
fn cookie_value(req: &Request, name: &str) -> Option<String> {
    let header = req.headers().get("cookie")?.to_str().ok()?;
    header.split(';').find_map(|pair| {
        let (key, value) = pair.trim().split_once('=')?;
        (key == name).then(|| value.to_string())
    })
}

/// Extract a single query-string parameter, percent-decoding its value.
fn query_param(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| urldecode(value))
    })
}

/// Percent-encode a string for use in a URL query component.
fn urlencode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for &byte in input.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Percent-decode a URL query component.
fn urldecode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                (Some(hi), Some(lo)) => {
                    out.push((hi << 4) | lo);
                    i += 3;
                }
                _ => {
                    out.push(bytes[i]);
                    i += 1;
                }
            },
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parse a single hex digit into its numeric value.
fn hex_val(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Generate a random, hex-encoded `state` value for CSRF protection.
fn random_state() -> String {
    let mut buf = [0u8; 16];
    getrandom::fill(&mut buf).expect("failed to gather random bytes for OAuth state");
    let mut state = String::with_capacity(buf.len() * 2);
    for byte in buf {
        state.push_str(&format!("{byte:02x}"));
    }
    state
}

/// Compare two byte slices in constant time to avoid timing side channels.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Headers that WASI HTTP manages itself and forbids setting on an outgoing
/// request. They must be stripped from the incoming request before it is
/// forwarded, otherwise the runtime rejects it with `HeaderError::Forbidden`.
const FORBIDDEN_HEADERS: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-connection",
    "transfer-encoding",
    "upgrade",
    "host",
    "content-length",
];

/// Strip headers that WASI HTTP manages itself and forbids setting on an
/// outgoing request, so forwarding the incoming request doesn't hit
/// `HeaderError::Forbidden`.
///
/// NOTE: This is a temporary workaround and will be removed in the future.
fn strip_forbidden_headers(req: &mut Request) {
    let headers = req.headers_mut();
    for name in FORBIDDEN_HEADERS {
        headers.remove(*name);
    }
}