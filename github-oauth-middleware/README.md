# github-oauth-example

An example [Spin](https://spinframework.dev) application that gates an HTTP component behind GitHub OAuth using a reusable HTTP middleware component.

Requests to the app are intercepted by the `github-oauth` middleware. Unauthenticated users are redirected through GitHub's OAuth flow; once authenticated, the request is forwarded to the application with the verified GitHub login attached as a header.

## Components

| Component | Description |
| --- | --- |
| `exampleapp` ([src/lib.rs](src/lib.rs)) | The gated application. Currently returns a simple `Hello World!` response. |
| `github-oauth` ([github-oauth/src/lib.rs](github-oauth/src/lib.rs)) | HTTP middleware that performs the GitHub OAuth login flow and forwards authenticated requests. |

## How it works

The middleware runs ahead of `exampleapp` on every request:

1. **Callback** — Requests to `/auth/callback` complete the OAuth flow. The middleware verifies the anti-CSRF `state`, exchanges the authorization `code` for an access token, and stores it in the `gh_token` cookie.
2. **Authenticated** — If a `gh_token` cookie is present, the middleware validates it against the GitHub API (`GET /user`). On success it sets the `x-github-user` header (overwriting any client-supplied value) and forwards the request to `exampleapp`.
3. **Unauthenticated** — Otherwise the user is redirected to GitHub's authorize endpoint to start the login flow, with a random `state` value stored in a short-lived cookie for CSRF protection.

Security details:

- Session and state cookies are set `HttpOnly`, `Secure`, and `SameSite=Lax`.
- The OAuth `state` is compared in constant time to avoid timing side channels.
- The forwarded `x-github-user` header is always overwritten so clients cannot spoof it.
- The OAuth scope requested is `read:user`.

## Prerequisites

- The [Spin CLI](https://spinframework.dev/install)
- A Rust toolchain with the `wasm32-wasip2` target:

  ```sh
  rustup target add wasm32-wasip2
  ```

## Configure a GitHub OAuth app

Create an OAuth app at <https://github.com/settings/developers> and set its **Authorization callback URL** to match `github_redirect_uri` below (for local development, `http://127.0.0.1:3000/auth/callback`).

The application requires the following variables:

| Variable | Description |
| --- | --- |
| `github_client_id` | OAuth app client ID |
| `github_client_secret` | OAuth app client secret (secret) |
| `github_redirect_uri` | Callback URL, ending in `/auth/callback` |

Provide them via environment variables using the `SPIN_VARIABLE_` prefix:

```sh
export SPIN_VARIABLE_GITHUB_CLIENT_ID="your-client-id"
export SPIN_VARIABLE_GITHUB_CLIENT_SECRET="your-client-secret"
export SPIN_VARIABLE_GITHUB_REDIRECT_URI="http://127.0.0.1:3000/auth/callback"
```

## Build and run

```sh
spin build
spin up
```

Then open <http://127.0.0.1:3000> in your browser. You'll be redirected to GitHub to authorize the app, and after granting access you'll be sent back to the application.