//! RED-phase contract for dashboard quick win Q9: compression, CSP, no permissive CORS,
//! cacheable static assets.
//!
//! Pinned decisions:
//! * `Content-Security-Policy` is sent on EVERY response (pages, htmx partials, static
//!   assets). Required directives: `default-src 'self'`, `script-src 'self'` (NO
//!   'unsafe-inline' / 'unsafe-eval' - inline JS must be moved to /static files),
//!   `style-src` containing `'self'` (and may contain 'unsafe-inline' because pages still use
//!   style="" attributes), `object-src 'none'`, `base-uri 'self'`, `frame-ancestors 'none'`,
//!   `form-action 'self'`. `img-src` may add `data:`. No remote origins (no `http:`/`https:`).
//! * gzip via tower-http `CompressionLayer` when the client sends `Accept-Encoding: gzip`;
//!   never when it does not.
//! * `CorsLayer::permissive()` removed: no `access-control-allow-origin` header at all.
//! * Static assets: `ETag` header, `If-None-Match` hit => 304 with empty body, and
//!   `Cache-Control` with `max-age` >= 3600 (existing 86400 is fine).

mod common;

use common::web::*;

const SAMPLE_PAGE: &str = "/regime";
const SAMPLE_PARTIAL: &str = "/partials/alert-list";

fn directives(csp: &str) -> std::collections::BTreeMap<String, Vec<String>> {
    csp.split(';')
        .filter_map(|d| {
            let mut it = d.split_whitespace();
            let name = it.next()?.to_string();
            Some((name, it.map(str::to_string).collect()))
        })
        .collect()
}

#[tokio::test]
async fn test_csp_when_any_response_should_be_present_on_pages_partials_and_static() {
    let app = app();
    for p in [
        SAMPLE_PAGE,
        "/",
        SAMPLE_PARTIAL,
        "/static/ghost-alpha.css",
        "/static/htmx.min.js",
        "/static/tokens.css",
    ] {
        let r = get(&app, p).await;
        assert_eq!(r.status, 200, "{p}");
        assert!(
            r.header("content-security-policy").is_some(),
            "{p} lacks Content-Security-Policy"
        );
    }
}

#[tokio::test]
async fn test_csp_when_parsed_should_lock_down_scripts_objects_frames_and_forms() {
    let app = app();
    let csp = get(&app, SAMPLE_PAGE)
        .await
        .header("content-security-policy")
        .unwrap();
    let d = directives(&csp);
    assert_eq!(d["default-src"], vec!["'self'"], "{csp}");
    assert_eq!(d["script-src"], vec!["'self'"], "{csp}");
    assert_eq!(d["object-src"], vec!["'none'"], "{csp}");
    assert_eq!(d["base-uri"], vec!["'self'"], "{csp}");
    assert_eq!(d["frame-ancestors"], vec!["'none'"], "{csp}");
    assert_eq!(d["form-action"], vec!["'self'"], "{csp}");
    assert!(d["style-src"].contains(&"'self'".to_string()), "{csp}");
    assert!(!csp.contains("unsafe-eval"), "{csp}");
    assert!(!csp.contains("http:") && !csp.contains("https:"), "{csp}");
    if let Some(img) = d.get("img-src") {
        assert!(img.iter().all(|s| s == "'self'" || s == "data:"), "{csp}");
    }
}

#[tokio::test]
async fn test_gzip_when_client_accepts_should_compress_pages_and_large_assets() {
    let app = app();
    for p in [
        SAMPLE_PAGE,
        "/",
        "/static/pico.min.css",
        "/static/htmx.min.js",
    ] {
        let plain = get(&app, p).await;
        let gz = get_with(&app, p, &[("accept-encoding", "gzip")]).await;
        assert_eq!(gz.status, 200, "{p}");
        assert_eq!(
            gz.header("content-encoding").as_deref(),
            Some("gzip"),
            "{p}"
        );
        assert!(gz.body.starts_with(&[0x1f, 0x8b]), "{p} body is not gzip");
        assert!(gz.body.len() < plain.body.len(), "{p} not smaller");
        assert!(
            gz.header("vary")
                .unwrap_or_default()
                .to_lowercase()
                .contains("accept-encoding"),
            "{p} missing Vary: Accept-Encoding"
        );
    }
}

#[tokio::test]
async fn test_gzip_when_client_does_not_accept_should_send_identity() {
    let app = app();
    let r = get(&app, "/static/pico.min.css").await;
    assert!(r.header("content-encoding").is_none());
    let r = get_with(&app, SAMPLE_PAGE, &[("accept-encoding", "identity")]).await;
    assert!(r.header("content-encoding").is_none());
}

#[tokio::test]
async fn test_cors_when_cross_origin_request_should_not_get_allow_origin() {
    let app = app();
    for p in [SAMPLE_PAGE, SAMPLE_PARTIAL, "/static/ghost-alpha.css"] {
        let r = get_with(&app, p, &[("origin", "https://evil.example")]).await;
        assert!(
            r.header("access-control-allow-origin").is_none(),
            "{p} must not send access-control-allow-origin"
        );
    }
}

#[tokio::test]
async fn test_cors_when_preflight_should_not_be_granted() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;
    let app = app();
    let req = Request::builder()
        .method("OPTIONS")
        .uri("/watchlist/add")
        .header("origin", "https://evil.example")
        .header("access-control-request-method", "POST")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert!(resp.headers().get("access-control-allow-origin").is_none());
    assert!(resp.headers().get("access-control-allow-methods").is_none());
}

fn max_age(cc: &str) -> u64 {
    cc.split(',')
        .find_map(|d| d.trim().strip_prefix("max-age="))
        .unwrap_or_else(|| panic!("no max-age in {cc}"))
        .parse()
        .unwrap()
}

/// Contract (round 2): unversioned asset URLs revalidate (`no-cache` + ETag/304);
/// URLs carrying `?v=<hash prefix>` are immutable for a year or more. The ETag is WEAK
/// (`W/"..."`) so it stays valid across gzip/identity representations.
#[tokio::test]
async fn test_static_when_unversioned_should_revalidate_with_weak_etag() {
    let app = app();
    for p in [
        "/static/ghost-alpha.css",
        "/static/tokens.css",
        "/static/htmx.min.js",
    ] {
        let r = get(&app, p).await;
        assert_eq!(r.status, 200, "{p}");
        let etag = r.header("etag").unwrap_or_else(|| panic!("{p} lacks ETag"));
        assert!(
            etag.starts_with("W/\""),
            "{p}: ETag must be weak, got {etag}"
        );
        let cc = r.header("cache-control").expect("cache-control");
        assert!(
            cc.contains("no-cache"),
            "{p}: unversioned needs no-cache, got {cc}"
        );
        assert!(!cc.contains("immutable"), "{p}: {cc}");
        assert_eq!(get(&app, p).await.header("etag").unwrap(), etag);
    }
    let a = get(&app, "/static/ghost-alpha.css").await.header("etag");
    let b = get(&app, "/static/tokens.css").await.header("etag");
    assert_ne!(a, b, "different files must have different ETags");
}

#[tokio::test]
async fn test_static_when_versioned_should_be_immutable_for_a_year() {
    let app = app();
    for name in ["ghost-alpha.css", "tokens.css", "htmx.min.js", "nav.js"] {
        let v = &asset_sha256_hex(name)[..8];
        let r = get(&app, &format!("/static/{name}?v={v}")).await;
        assert_eq!(r.status, 200, "{name}");
        let cc = r.header("cache-control").expect("cache-control");
        assert!(cc.contains("immutable"), "{name}: {cc}");
        assert!(max_age(&cc) >= 31_536_000, "{name}: {cc}");
        assert!(r.header("etag").is_some(), "{name} keeps ETag");
    }
}

#[tokio::test]
async fn test_shell_when_rendering_static_urls_should_version_them_by_content_hash() {
    let app = app();
    for page in ["/", "/regime", "/portfolio"] {
        let html = get(&app, page).await.text();
        let urls = static_urls(&html);
        assert!(urls.len() >= 5, "{page}: {urls:?}");
        for u in urls {
            let (path, q) = u
                .split_once('?')
                .unwrap_or_else(|| panic!("{page}: unversioned {u}"));
            let v = q
                .strip_prefix("v=")
                .unwrap_or_else(|| panic!("{page}: bad query in {u}"));
            assert!(v.len() >= 8, "{u}");
            assert!(v.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')), "{u}");
            let name = path.trim_start_matches("/static/");
            assert!(
                asset_sha256_hex(name).starts_with(v),
                "{u} is not the sha256 prefix"
            );
            assert_eq!(get(&app, &u).await.status, 200, "{u}");
        }
    }
}

#[tokio::test]
async fn test_static_when_if_none_match_hits_should_return_304_with_empty_body() {
    let app = app();
    let first = get(&app, "/static/ghost-alpha.css").await;
    let etag = first.header("etag").expect("etag");
    let r = get_with(&app, "/static/ghost-alpha.css", &[("if-none-match", &etag)]).await;
    assert_eq!(r.status, 304);
    assert!(r.body.is_empty());
    // a stale validator still returns the asset
    let r = get_with(
        &app,
        "/static/ghost-alpha.css",
        &[("if-none-match", "\"stale\"")],
    )
    .await;
    assert_eq!(r.status, 200);
    assert!(!r.body.is_empty());
}

#[tokio::test]
async fn test_static_when_tokens_css_requested_should_be_served_as_css() {
    let app = app();
    let r = get(&app, "/static/tokens.css").await;
    assert_eq!(r.status, 200);
    assert!(r.header("content-type").unwrap().contains("css"));
    assert!(r.text().contains("--ga-bg"));
}

#[tokio::test]
async fn test_static_when_missing_should_still_404_with_csp() {
    let app = app();
    let r = get(&app, "/static/nope.txt").await;
    assert_eq!(r.status, 404);
    assert!(r.header("content-security-policy").is_some());
}

#[tokio::test]
async fn test_304_when_gzip_negotiated_should_carry_vary_accept_encoding() {
    let app = app();
    let etag = get(&app, "/static/tokens.css")
        .await
        .header("etag")
        .unwrap();
    let r = get_with(
        &app,
        "/static/tokens.css",
        &[("if-none-match", &etag), ("accept-encoding", "gzip")],
    )
    .await;
    assert_eq!(r.status, 304);
    assert!(r
        .header("vary")
        .unwrap_or_default()
        .to_lowercase()
        .contains("accept-encoding"));
}

#[tokio::test]
async fn test_hardening_headers_when_any_response_should_have_nosniff_and_referrer_policy() {
    let app = app();
    let etag = get(&app, "/static/tokens.css")
        .await
        .header("etag")
        .unwrap();
    let mut resps = vec![
        get(&app, SAMPLE_PAGE).await,
        get(&app, SAMPLE_PARTIAL).await,
        get(&app, "/static/tokens.css").await,
        get(&app, "/static/nope.txt").await,
    ];
    resps.push(get_with(&app, "/static/tokens.css", &[("if-none-match", &etag)]).await);
    assert_eq!(resps[4].status, 304);
    for (i, r) in resps.iter().enumerate() {
        assert_eq!(
            r.header("x-content-type-options").as_deref(),
            Some("nosniff"),
            "#{i}"
        );
        assert_eq!(
            r.header("referrer-policy").as_deref(),
            Some("same-origin"),
            "#{i}"
        );
    }
}
