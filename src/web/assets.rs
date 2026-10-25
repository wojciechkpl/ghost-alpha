use axum::extract::{Path, Query};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use rust_embed::Embed;
use serde::Deserialize;

#[derive(Embed)]
#[folder = "web/static/"]
pub struct StaticAssets;

const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const REVALIDATE: &str = "no-cache";

/// Query parameters accepted on `/static/*` (cache-busting version).
#[derive(Deserialize)]
pub struct StaticQuery {
    v: Option<String>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `/static/{name}?v={first 8 hex chars of the embedded file's sha256}`.
///
/// Falls back to the bare URL if the asset is not embedded.
pub fn versioned(name: &str) -> String {
    match StaticAssets::get(name) {
        Some(f) => format!("/static/{name}?v={}", &hex(&f.metadata.sha256_hash())[..8]),
        None => format!("/static/{name}"),
    }
}

/// Serve an embedded static file at `/static/{path}`.
///
/// Sends the guessed `Content-Type` and a weak, content-derived `ETag`.
/// URLs carrying the current `?v=` hash prefix are `immutable` for a year;
/// bare URLs must revalidate. A matching `If-None-Match` yields `304`.
pub async fn static_handler(
    Path(path): Path<String>,
    Query(q): Query<StaticQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let Some(file) = StaticAssets::get(&path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let digest = hex(&file.metadata.sha256_hash());
    let etag = format!("W/\"{}\"", &digest[..32]);
    let current =
        q.v.as_deref()
            .is_some_and(|v| v.len() >= 8 && digest.starts_with(v));
    let cache = (
        header::CACHE_CONTROL,
        if current { IMMUTABLE } else { REVALIDATE }.to_string(),
    );

    if if_none_match_hits(&headers, &etag) {
        return (
            StatusCode::NOT_MODIFIED,
            [
                (header::ETAG, etag),
                cache,
                (header::VARY, "accept-encoding".to_string()),
            ],
        )
            .into_response();
    }

    let mime = mime_guess::from_path(&path).first_or_octet_stream();
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, mime.as_ref().to_string()),
            cache,
            (header::ETAG, etag),
        ],
        file.data.into_owned(),
    )
        .into_response()
}

/// True when the request's `If-None-Match` matches `etag` (weak comparison) or is `*`.
fn if_none_match_hits(headers: &HeaderMap, etag: &str) -> bool {
    let core = |t: &str| t.strip_prefix("W/").unwrap_or(t).to_string();
    let want = core(etag);
    headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(',')
                .map(str::trim)
                .any(|c| c == "*" || core(c) == want)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use tower::ServiceExt;

    fn test_router() -> Router {
        Router::new().route("/static/*path", axum::routing::get(static_handler))
    }

    #[tokio::test]
    async fn static_asset_htmx_js_returns_200() {
        let app = test_router();
        let req = Request::get("/static/htmx.min.js")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(
            ct.contains("javascript"),
            "expected javascript content-type, got {ct}"
        );
    }

    #[tokio::test]
    async fn static_asset_pico_css_returns_200() {
        let app = test_router();
        let req = Request::get("/static/pico.min.css")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(ct.contains("css"), "expected css content-type, got {ct}");
    }

    #[tokio::test]
    async fn static_asset_ghost_alpha_css_returns_200() {
        let app = test_router();
        let req = Request::get("/static/ghost-alpha.css")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn static_asset_nonexistent_returns_404() {
        let app = test_router();
        let req = Request::get("/static/doesnotexist.txt")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn static_assets_have_cache_control() {
        let app = test_router();
        let req = Request::get(versioned("htmx.min.js"))
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let cc = resp
            .headers()
            .get("cache-control")
            .expect("response should include cache-control header")
            .to_str()
            .unwrap();
        assert!(
            cc.contains("max-age"),
            "cache-control should contain max-age, got {cc}"
        );
    }
}
