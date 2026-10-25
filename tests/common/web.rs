//! Helpers for in-process web dashboard tests (no network, in-memory SQLite).
//!
//! Contract relied upon: `ghost_alpha::web::{create_router, AppState}` with
//! public `repo` / `config` fields (already the case today).

use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::Router;
use ghost_alpha::config::AppConfig;
use ghost_alpha::storage::db::SqliteRepository;
use ghost_alpha::web::{create_router, AppState};
use tower::ServiceExt;

pub struct Resp {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl Resp {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    pub fn header(&self, name: &str) -> Option<String> {
        self.headers
            .get(name)
            .map(|v| v.to_str().unwrap().to_string())
    }
}

pub fn app() -> Router {
    let repo = SqliteRepository::open_in_memory().expect("in-memory db");
    let config: AppConfig = toml::from_str("").expect("empty config must deserialize");
    create_router(AppState {
        repo: Arc::new(repo),
        config: Arc::new(config),
    })
}

pub async fn get_with(app: &Router, path: &str, headers: &[(&str, &str)]) -> Resp {
    let mut req = Request::get(path);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let resp = app
        .clone()
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    Resp {
        status,
        headers,
        body,
    }
}

pub async fn get(app: &Router, path: &str) -> Resp {
    get_with(app, path, &[]).await
}

/// All `.route("<path>", ...get(...))` registrations in `src/web/mod.rs`,
/// excluding `/static/*` and `/partials/*`. These are the full-page routes.
/// (Source-parsed on purpose: the router exposes no route table.)
pub fn registered_routes() -> Vec<(String, String)> {
    let src =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/web/mod.rs")).unwrap();
    // Only the non-test part of the file.
    let src = src.split("#[cfg(test)]").next().unwrap().to_string();
    let mut out = Vec::new();
    let mut rest = src.as_str();
    while let Some(i) = rest.find(".route(") {
        rest = &rest[i + ".route(".len()..];
        let rest_trim = rest.trim_start();
        let Some(stripped) = rest_trim.strip_prefix('"') else {
            continue;
        };
        let end = stripped.find('"').unwrap();
        let path = stripped[..end].to_string();
        let after = &stripped[end + 1..];
        let after = after.trim_start().trim_start_matches(',').trim_start();
        let method = if after.starts_with("axum::routing::post(") || after.starts_with("post(") {
            "POST"
        } else {
            "GET"
        };
        out.push((method.to_string(), path));
    }
    out
}

pub fn page_routes() -> Vec<String> {
    registered_routes()
        .into_iter()
        .filter(|(m, p)| m == "GET" && !p.starts_with("/static") && !p.starts_with("/partials"))
        .map(|(_, p)| p)
        .collect()
}

pub fn partial_routes() -> Vec<String> {
    registered_routes()
        .into_iter()
        .filter(|(m, p)| m == "GET" && p.starts_with("/partials"))
        .map(|(_, p)| p)
        .collect()
}

/// Count non-overlapping occurrences of `needle`.
pub fn count(hay: &str, needle: &str) -> usize {
    hay.matches(needle).count()
}

/// Return every opening tag (`<name ...>`) for the given tag name.
pub fn open_tags(html: &str, name: &str) -> Vec<String> {
    let pat = format!("<{name}");
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find(&pat) {
        let after = &rest[i + pat.len()..];
        let boundary = after
            .chars()
            .next()
            .map(|c| c.is_whitespace() || c == '>' || c == '/')
            .unwrap_or(false);
        if boundary {
            let end = after.find('>').map(|e| e + 1).unwrap_or(after.len());
            out.push(format!("{pat}{}", &after[..end]));
        }
        rest = after;
    }
    out
}

/// Extract the value of attribute `attr` from an opening tag string.
pub fn attr(tag: &str, attr: &str) -> Option<String> {
    for q in ['"', '\''] {
        let pat = format!(" {attr}={q}");
        if let Some(i) = tag.find(&pat) {
            let v = &tag[i + pat.len()..];
            return v.find(q).map(|e| v[..e].to_string());
        }
    }
    None
}

/// Every opening/closing tag string in document order (`<...>`).
pub fn all_tags(html: &str) -> Vec<String> {
    let mut v = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find('<') {
        let a = &rest[i..];
        match a.find('>') {
            Some(e) => {
                v.push(a[..=e].to_string());
                rest = &a[e..];
            }
            None => break,
        }
    }
    v
}

/// Text between `<tag ...>` and `</tag>` for the first occurrence, with nested tags stripped.
pub fn first_text(html: &str, tag: &str) -> Option<String> {
    let s = html.find(&format!("<{tag}"))?;
    let after = &html[s..];
    let open_end = after.find('>')? + 1;
    let close = after.find(&format!("</{tag}>"))?;
    Some(strip_tags(&after[open_end..close]))
}

pub fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut depth = false;
    for c in s.chars() {
        match c {
            '<' => depth = true,
            '>' => depth = false,
            _ if !depth => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `/static/...` URLs referenced via `href=` or `src=` (query string included).
pub fn static_urls(html: &str) -> Vec<String> {
    all_tags(html)
        .iter()
        .flat_map(|t| {
            ["href", "src"]
                .iter()
                .filter_map(|a| attr(t, a))
                .filter(|u| u.starts_with("/static/"))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Lowercase hex SHA-256 of an embedded static asset (name relative to web/static/).
pub fn asset_sha256_hex(name: &str) -> String {
    let f = ghost_alpha::web::assets::StaticAssets::get(name)
        .unwrap_or_else(|| panic!("embedded asset {name} missing"));
    f.metadata
        .sha256_hash()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
