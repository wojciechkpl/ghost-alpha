use axum::extract::Path;
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "web/static/"]
pub struct StaticAssets;

/// Serve an embedded static file at `/static/{path}`.
///
/// Returns the file with the correct `Content-Type` (guessed from extension)
/// and a long-lived `Cache-Control` header for browser caching.
pub async fn static_handler(Path(path): Path<String>) -> impl IntoResponse {
    match StaticAssets::get(&path) {
        Some(file) => {
            let mime = mime_guess::from_path(&path).first_or_octet_stream();
            (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, mime.as_ref().to_string()),
                    (header::CACHE_CONTROL, "public, max-age=86400".to_string()),
                ],
                file.data.into_owned(),
            )
                .into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
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
        let req = Request::get("/static/htmx.min.js")
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
