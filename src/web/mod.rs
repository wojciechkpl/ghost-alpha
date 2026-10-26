pub mod assets;
pub mod charts;
pub mod concentration;
pub mod dashboard_routes;
pub mod diversification_report;
pub mod freshness;
pub mod quarterly_report;
pub mod routes;
pub mod shell;

use std::sync::Arc;

use axum::http::{header, HeaderValue};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::{extract::Request, Router};
use tower_http::compression::CompressionLayer;

use crate::config::AppConfig;
use crate::data::edgar::EdgarClient;
use crate::storage::db::SqliteRepository;

/// Shared state injected into every route handler.
#[derive(Clone)]
pub struct AppState {
    pub repo: Arc<SqliteRepository>,
    pub config: Arc<AppConfig>,
    /// Shared SEC EDGAR client; `None` when no User-Agent is configured.
    pub edgar: Option<Arc<EdgarClient>>,
    /// Per-ETF evaluation timeout for the concentration overlap; `None` means
    /// [`concentration::ETF_TIMEOUT`].
    pub etf_timeout: Option<std::time::Duration>,
}

/// Content-Security-Policy sent on every response. All scripts and styles are
/// served from `/static`; `style-src` allows inline `style=""` attributes
/// (and inline SVG styling) only.
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self'; \
style-src 'self' 'unsafe-inline'; img-src 'self' data:; object-src 'none'; \
base-uri 'self'; frame-ancestors 'none'; form-action 'self'";

/// Attach the CSP and basic hardening headers to every response.
async fn security_headers(req: Request, next: Next) -> Response {
    let mut resp = next.run(req).await;
    resp.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CONTENT_SECURITY_POLICY),
    );
    resp.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    resp.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    resp
}

/// Build the Axum router with all dashboard routes, CSP and gzip middleware.
pub fn create_router(state: AppState) -> Router {
    Router::new()
        // Full pages
        .route("/", axum::routing::get(routes::home))
        .route("/alerts", axum::routing::get(routes::alerts_page))
        .route("/watchlist", axum::routing::get(routes::watchlist_page))
        .route("/regime", axum::routing::get(routes::regime_page))
        .route("/flagged", axum::routing::get(routes::flagged_page))
        .route("/analytics", axum::routing::get(routes::analytics_page))
        .route("/strategies", axum::routing::get(routes::strategies_page))
        .route("/settings", axum::routing::get(routes::settings_page))
        // Dashboard pages (diversification advisory)
        .route(
            "/portfolio",
            axum::routing::get(dashboard_routes::portfolio_page),
        )
        .route(
            "/correlation",
            axum::routing::get(dashboard_routes::correlation_page),
        )
        .route(
            "/valuation",
            axum::routing::get(dashboard_routes::valuation_page),
        )
        .route("/tax", axum::routing::get(dashboard_routes::tax_page))
        .route(
            "/optimize",
            axum::routing::get(dashboard_routes::optimize_page),
        )
        .route("/etf", axum::routing::get(dashboard_routes::etf_page))
        .route(
            "/selling",
            axum::routing::get(dashboard_routes::selling_page),
        )
        .route(
            "/decisions",
            axum::routing::get(dashboard_routes::decisions_page),
        )
        .route(
            "/fundamentals",
            axum::routing::get(dashboard_routes::fundamentals_page),
        )
        .route(
            "/concentration-decay",
            axum::routing::get(dashboard_routes::concentration_decay_page),
        )
        .route(
            "/allocation",
            axum::routing::get(dashboard_routes::allocation_page),
        )
        .route(
            "/geo-risk",
            axum::routing::get(dashboard_routes::geo_risk_page),
        )
        .route(
            "/rotation",
            axum::routing::get(dashboard_routes::rotation_page),
        )
        .route("/sizing", axum::routing::get(dashboard_routes::sizing_page))
        .route(
            "/portfolio/concentration",
            axum::routing::get(concentration::concentration_page),
        )
        // htmx partial endpoints (fragments only)
        .route(
            "/partials/screen-table",
            axum::routing::get(routes::screen_table_partial),
        )
        .route(
            "/partials/alert-list",
            axum::routing::get(routes::alert_list_partial),
        )
        .route(
            "/partials/watchlist-rows",
            axum::routing::get(routes::watchlist_rows_partial),
        )
        .route(
            "/partials/regime-strip",
            axum::routing::get(routes::regime_strip_partial),
        )
        .route(
            "/partials/score-heatmap",
            axum::routing::get(routes::score_heatmap_partial),
        )
        .route(
            "/partials/sentiment-heatmap",
            axum::routing::get(routes::sentiment_heatmap_partial),
        )
        .route(
            "/partials/grade-dist",
            axum::routing::get(routes::grade_dist_partial),
        )
        .route(
            "/partials/rs-scatter",
            axum::routing::get(routes::rs_scatter_partial),
        )
        .route(
            "/partials/alert-calendar",
            axum::routing::get(routes::alert_calendar_partial),
        )
        .route(
            "/partials/regime-history",
            axum::routing::get(routes::regime_history_partial),
        )
        .route(
            "/partials/macro-gauges",
            axum::routing::get(routes::macro_gauges_partial),
        )
        .route(
            "/partials/etf-overlap",
            axum::routing::get(concentration::etf_overlap_partial),
        )
        // Watchlist mutations (return fragments)
        .route("/watchlist/add", axum::routing::post(routes::watchlist_add))
        .route(
            "/watchlist/remove",
            axum::routing::post(routes::watchlist_remove),
        )
        // Embedded static assets
        .route("/static/*path", axum::routing::get(assets::static_handler))
        .layer(middleware::from_fn(security_headers))
        .layer(CompressionLayer::new().gzip(true))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AlertsConfig, FinnhubConfig, PluginsConfig, StorageConfig};

    pub(crate) fn make_state() -> AppState {
        let repo =
            SqliteRepository::open_in_memory().expect("in-memory DB should open without error");
        let config = AppConfig {
            finnhub: FinnhubConfig::default(),
            telegram: None,
            discord: None,
            fred: None,
            sentiment: None,
            backtest: None,
            alerts: AlertsConfig::default(),
            storage: StorageConfig::default(),
            plugins: PluginsConfig::default(),
            cache: Default::default(),
            universe: Vec::new(),
            edgar: None,
            profile: None,
        };
        AppState {
            repo: Arc::new(repo),
            config: Arc::new(config),
            edgar: None,
            etf_timeout: None,
        }
    }

    #[test]
    fn router_creation_does_not_panic() {
        let state = make_state();
        // create_router must not panic — if it does the test fails
        let _router = create_router(state);
    }
}
