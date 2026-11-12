//! The embedded page: the Vite build output, compiled into the binary by the
//! manifest `build.rs` writes, so `rust-loc-web.exe` ships as one file.
//!
//! When `web/dist` has no build yet, the manifest holds a single placeholder
//! page that says so, and the binary still builds and still starts.

// The manifest `build.rs` generates: `(path, bytes)` pairs, path relative to
// the dist root with forward slashes. `INDEX_HTML` names the fallback page.
include!(concat!(env!("OUT_DIR"), "/web_dist.rs"));

/// Media type by extension; the handful of types a built page contains.
fn mime(path: &str) -> &'static str {
    let extension = path.rsplit('.').next().unwrap_or("");
    match extension {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// One embedded asset by path, with its media type.
pub fn lookup(path: &str) -> Option<(&'static str, &'static [u8])> {
    ASSETS
        .iter()
        .find(|(name, _)| *name == path)
        .map(|(name, bytes)| (mime(name), *bytes))
}

/// The page a browser gets when it asks for `/` or for a path the SPA owns.
pub fn index() -> (&'static str, &'static [u8]) {
    lookup("index.html").unwrap_or((mime("index.html"), PLACEHOLDER))
}

/// Serve the embedded page: the exact path first, then `index.html`, so a
/// refresh on a client-side route still lands on the app.
pub async fn static_fallback(
    uri: axum::http::Uri,
) -> axum::response::Response {
    use axum::http::header;
    use axum::response::IntoResponse;

    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    let (mime, bytes) = match lookup(path) {
        Some(found) => found,
        None => {
            // A path with an asset extension but no asset is a missing file,
            // not a route: 404 it instead of serving the page.
            if path.contains('.') && !path.ends_with(".html") {
                return (axum::http::StatusCode::NOT_FOUND, "not found").into_response();
            }
            index()
        }
    };
    (
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        bytes,
    )
        .into_response()
}
