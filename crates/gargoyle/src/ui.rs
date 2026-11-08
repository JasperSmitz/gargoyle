//! The embedded frontend (ADR 0004).
//!
//! `web/dist` is compiled into the binary. When it has not been built, the binary still compiles
//! (`allow_missing`) and `/` serves a placeholder page instead.

use axum::extract::Path;
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};

#[derive(rust_embed::Embed)]
#[folder = "../../web/dist"]
#[allow_missing = true]
struct Assets;

const INDEX: &str = "index.html";
const NO_CACHE: &str = "no-cache";
/// Vite content-hashes everything it emits under `assets/`, so those can be cached forever.
/// Other files (index.html, copies of `web/public/` such as the favicon) keep their names
/// across releases and must be revalidated.
const IMMUTABLE: &str = "public, max-age=31536000, immutable";

const PLACEHOLDER: &str = "<!doctype html>
<html lang=\"en\">
<head><meta charset=\"utf-8\"><title>Gargoyle</title></head>
<body>
<h1>Gargoyle</h1>
<p>Frontend not built &mdash; run <code>make web</code> and rebuild. The API is available under
<a href=\"/api/v1/health\">/api/v1/health</a>.</p>
</body>
</html>
";

pub async fn index() -> Response {
    serve(INDEX)
}

pub async fn asset(Path(path): Path<String>) -> Response {
    serve(&path)
}

fn serve(path: &str) -> Response {
    match Assets::get(path) {
        Some(file) => (
            [
                (header::CONTENT_TYPE, file.metadata.mimetype().to_owned()),
                (header::CACHE_CONTROL, cache_control(path).to_owned()),
            ],
            file.data,
        )
            .into_response(),
        None if path == INDEX => {
            ([(header::CACHE_CONTROL, NO_CACHE)], Html(PLACEHOLDER)).into_response()
        }
        None => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

fn cache_control(path: &str) -> &'static str {
    if path.starts_with("assets/") {
        IMMUTABLE
    } else {
        NO_CACHE
    }
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use tower::ServiceExt;

    use super::*;
    use crate::http::router;

    async fn get(uri: &str) -> Response {
        let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
        router().oneshot(req).await.unwrap()
    }

    #[tokio::test]
    async fn root_serves_html_without_caching() {
        for uri in ["/", "/index.html"] {
            let resp = get(uri).await;
            assert_eq!(resp.status(), StatusCode::OK, "{uri}");
            assert_eq!(resp.headers()[header::CACHE_CONTROL], NO_CACHE, "{uri}");
            let ct = resp.headers()[header::CONTENT_TYPE].to_str().unwrap();
            assert!(ct.starts_with("text/html"), "{uri}: {ct}");
            let body = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
            assert!(String::from_utf8_lossy(&body).contains("<html"), "{uri}");
        }
    }

    #[tokio::test]
    async fn unknown_paths_are_404() {
        for uri in [
            "/nope.js",
            "/assets/missing-123.css",
            "/../Cargo.toml",
            "/timers/x",
        ] {
            assert_eq!(get(uri).await.status(), StatusCode::NOT_FOUND, "{uri}");
        }
    }

    #[tokio::test]
    async fn embedded_files_get_their_cache_policy() {
        // Only meaningful when web/dist was built; without it there are no files to check.
        for path in Assets::iter() {
            let resp = get(&format!("/{path}")).await;
            assert_eq!(resp.status(), StatusCode::OK, "{path}");
            assert_eq!(
                resp.headers()[header::CACHE_CONTROL],
                cache_control(&path),
                "{path}"
            );
        }
    }

    #[test]
    fn cache_policy() {
        assert_eq!(cache_control("index.html"), NO_CACHE);
        assert_eq!(cache_control("assets/index-3f9a1c.js"), IMMUTABLE);
        assert_eq!(cache_control("favicon.svg"), NO_CACHE);
        assert_eq!(cache_control("robots.txt"), NO_CACHE);
    }
}
