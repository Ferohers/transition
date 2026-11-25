//! Host, Origin, and CSRF isolation middleware for loopback `md-server`.
//!
//! Blocks DNS-rebinding (`Host` check) and cross-origin browser requests
//! (`Origin` / `Sec-Fetch-Site` check) with `403 Forbidden`.

use axum::{
    extract::Request,
    http::{HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};

fn is_loopback_host(raw: &str) -> bool {
    let host_only = raw
        .trim()
        .strip_prefix('[')
        .and_then(|s| s.split(']').next())
        .or_else(|| raw.trim().split(':').next())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(host_only.as_str(), "127.0.0.1" | "localhost" | "::1")
}

fn is_loopback_origin(raw: &str) -> bool {
    let lower = raw.trim().to_ascii_lowercase();
    if lower == "null" {
        return false;
    }
    let Some(rest) = lower
        .strip_prefix("http://")
        .or_else(|| lower.strip_prefix("https://"))
    else {
        return false;
    };
    let authority = rest.split('/').next().unwrap_or("");
    is_loopback_host(authority)
}

pub async fn loopback_origin_guard(req: Request, next: Next) -> Response {
    let path = req.uri().path();
    // External webhook/share endpoints use their own secret/password auth.
    let skip_origin = path.starts_with("/webhooks/") || path.starts_with("/share/");

    if let Some(host_val) = req.headers().get(header::HOST).and_then(|v| v.to_str().ok())
        && !is_loopback_host(host_val)
    {
        return (StatusCode::FORBIDDEN, "forbidden host").into_response();
    }

    if !skip_origin {
        if let Some(origin_val) = req
            .headers()
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            && !is_loopback_origin(origin_val)
        {
            return (StatusCode::FORBIDDEN, "forbidden cross-origin request").into_response();
        }

        let is_mutating = matches!(
            *req.method(),
            Method::POST | Method::PUT | Method::PATCH | Method::DELETE
        );
        if is_mutating
            && let Some(fetch_site) = req
                .headers()
                .get("sec-fetch-site")
                .and_then(|v| v.to_str().ok())
            && fetch_site.eq_ignore_ascii_case("cross-site")
        {
            return (StatusCode::FORBIDDEN, "forbidden cross-site mutation").into_response();
        }
    }

    let mut res = next.run(req).await;
    res.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    res
}
