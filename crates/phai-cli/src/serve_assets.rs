//! Static serving of the embedded LiveStore web app.
//!
//! The React/Vite bundle is built into `web/dist` and embedded into the `phai`
//! binary at compile time (`include_dir!`), so `phai serve` ships the whole UI
//! with no runtime file dependency — keeping the single-binary install
//! (ADR-0001). The JS build step lives only in CI, never on the user's machine.

use axum::{
    http::{header, HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use include_dir::{include_dir, Dir};

static DIST: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/web/dist");

const INDEX: &str = "index.html";

/// Serve an embedded asset, falling back to `index.html` for client-side
/// routes (anything without a matching file and without a file extension).
pub async fn static_handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { INDEX } else { path };

    match DIST.get_file(path) {
        Some(file) => asset_response(path, file.contents()),
        // SPA fallback: serve index.html so client routing can take over.
        None if !has_extension(path) => match DIST.get_file(INDEX) {
            Some(index) => asset_response(INDEX, index.contents()),
            None => not_found(),
        },
        None => not_found(),
    }
}

fn asset_response(path: &str, bytes: &'static [u8]) -> Response {
    let mut resp = (StatusCode::OK, bytes).into_response();
    let headers = resp.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type(path)),
    );
    // Cross-origin isolation: LiveStore's OPFS worker + wa-sqlite need a
    // crossOriginIsolated context (SharedArrayBuffer). We use `require-corp`
    // (not `credentialless`) because WebKit/WKWebView — the engine the native
    // desktop shell uses — does not honor `credentialless`, so it would leave
    // the context non-isolated and break wa-sqlite. `require-corp` demands all
    // subresources be same-origin or carry CORP; fonts are self-hosted via
    // @fontsource, so there are no cross-origin subresources to allow. ADR-0039.
    headers.insert(
        "cross-origin-opener-policy",
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        "cross-origin-embedder-policy",
        HeaderValue::from_static("require-corp"),
    );
    // Vite emits content-hashed asset filenames → safe to cache immutably.
    // index.html must stay fresh so new bundles are picked up.
    let cache = if path == INDEX {
        "no-cache"
    } else if path.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    resp
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "404").into_response()
}

fn has_extension(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .map(|seg| seg.contains('.'))
        .unwrap_or(false)
}

fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("png") => "image/png",
        Some("woff2") => "font/woff2",
        Some("woff") => "font/woff",
        Some("map") => "application/json",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_is_embedded() {
        assert!(
            DIST.get_file(INDEX).is_some(),
            "web/dist/index.html must be built + committed"
        );
    }

    #[test]
    fn content_type_maps_known_extensions() {
        assert_eq!(
            content_type("assets/index-abc.js"),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(content_type("assets/wa-sqlite.wasm"), "application/wasm");
        assert_eq!(content_type("index.html"), "text/html; charset=utf-8");
    }

    #[test]
    fn extension_detection() {
        assert!(has_extension("assets/index-abc.js"));
        assert!(!has_extension("review"));
        assert!(!has_extension("forecasts/2026"));
    }

    #[test]
    fn cross_origin_isolation_uses_require_corp() {
        // WebKit/WKWebView (native desktop shell) ignores `credentialless`, so
        // the isolated context — and thus wa-sqlite's SharedArrayBuffer — only
        // works with `require-corp`. Regression guard for ADR-0039.
        let resp = asset_response(INDEX, b"<html></html>");
        let h = resp.headers();
        assert_eq!(h.get("cross-origin-opener-policy").unwrap(), "same-origin");
        assert_eq!(
            h.get("cross-origin-embedder-policy").unwrap(),
            "require-corp"
        );
    }
}
