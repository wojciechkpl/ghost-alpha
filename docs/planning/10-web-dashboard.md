# Plan 10: Embedded Web Dashboard

**Effort:** Large (7-10 days)
**Impact:** Very High
**Priority:** 10

## Problem

Ghost Alpha is CLI-only. While powerful for technical users, most potential users expect a visual interface to browse screening results, view alert history, and monitor market regime. The CLI output is ephemeral — results disappear after the terminal scrolls.

## Solution

Add a `ghost-alpha serve` subcommand that starts a lightweight, single-binary web server serving an interactive dashboard. No npm, no JavaScript build step, no external assets — everything embedded in the Rust binary.

## Technology choices

| Component | Choice | Rationale |
|-----------|--------|-----------|
| HTTP server | `axum` | Already proven in Rust ecosystem, async, minimal |
| Templating | `askama` | Compile-time templates, zero runtime cost |
| Interactivity | `htmx` | Server-rendered HTML with AJAX — no JS framework |
| Styling | `pico.css` | Classless CSS, ~10KB, beautiful defaults |
| Charts | Inline SVG | No JavaScript charting library needed |
| Assets | `rust-embed` | All CSS/JS/images compiled into binary |

**Why htmx instead of React/Vue:** Keeps the single-binary promise. No node_modules, no build step, no CORS. The server renders HTML fragments; htmx swaps them in-place. Total JS: ~14KB (htmx minified).

## Dashboard pages

### 1. Home / Screening Results

```
┌─────────────────────────────────────────────────────────────┐
│  Ghost Alpha Dashboard              Regime: 🟢 Bull         │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  Last screen: 2026-03-07 18:30 UTC | 503 stocks | 12 hits  │
│                                                             │
│  ┌─────┬────────┬───────┬───────┬─────┬──────┬──────────┐  │
│  │Rank │ Ticker │ Score │ Grade │ RS% │VolZ │ Minervini│  │
│  ├─────┼────────┼───────┼───────┼─────┼──────┼──────────┤  │
│  │  1  │ NVDA   │ 94.2  │  A+   │ 97  │ 2.8  │   8/8   │  │
│  │  2  │ PLTR   │ 87.1  │  A    │ 91  │ 1.9  │   7/8   │  │
│  │  3  │ META   │ 82.5  │  A-   │ 85  │ 1.2  │   8/8   │  │
│  │ ... │        │       │       │     │      │         │  │
│  └─────┴────────┴───────┴───────┴─────┴──────┴──────────┘  │
│                                                             │
│  [Run Screen Now]  [Export CSV]  [Export JSON]               │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

### 2. Alert History

- Timeline of past alerts with severity color-coding
- Filter by ticker, severity, date range
- Click to expand alert details + narrative

### 3. Watchlist

- Current watchlist with live scores
- Add/remove tickers via UI
- Score trend sparklines (last 30 days)

### 4. Market Regime

- Current regime indicator (Bull/Sideways/Bear)
- SPY chart with 200-SMA overlay (SVG)
- Macro risk indicators (VIX, OAS, yield curve)
- Regime history timeline

### 5. Flagged Stocks

- Active flagged positions with entry date, entry score
- Exit signal indicators
- Conviction decay tracking

### 6. Backtest Results

- Embed the HTML report from Plan 07
- Or link to the generated file

## Implementation

### 1. Web server setup

Create `src/web/mod.rs`:

```rust
pub mod routes;
pub mod templates;
pub mod assets;

use axum::{Router, routing::get};
use std::sync::Arc;
use crate::storage::db::SqliteRepository;

pub struct AppState {
    pub repo: Arc<SqliteRepository>,
    pub config: Arc<AppConfig>,
}

pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(routes::home))
        .route("/alerts", get(routes::alerts))
        .route("/watchlist", get(routes::watchlist))
        .route("/regime", get(routes::regime))
        .route("/flagged", get(routes::flagged))
        .route("/assets/*path", get(routes::static_assets))
        // htmx partial endpoints
        .route("/partials/screen-table", get(routes::screen_table_partial))
        .route("/partials/alert-list", get(routes::alert_list_partial))
        .with_state(Arc::new(state))
}
```

### 2. Embedded assets

```rust
#[derive(RustEmbed)]
#[folder = "web/static/"]
struct Assets;

// Includes:
// - htmx.min.js (~14KB)
// - pico.min.css (~10KB)
// - ghost-alpha.css (custom overrides)
```

### 3. Templates (askama)

```
templates/
├── base.html          # Layout with nav, head, footer
├── home.html          # Screening results table
├── alerts.html        # Alert history timeline
├── watchlist.html     # Watchlist management
├── regime.html        # Market regime display
├── flagged.html       # Flagged stocks tracker
└── partials/
    ├── screen_table.html   # htmx partial for table refresh
    └── alert_list.html     # htmx partial for alert filtering
```

### 4. htmx interactions

```html
<!-- Trigger a screen run from the dashboard -->
<button hx-post="/api/screen"
        hx-target="#results-table"
        hx-swap="innerHTML"
        hx-indicator="#spinner">
    Run Screen Now
</button>

<!-- Filter alerts by severity -->
<select hx-get="/partials/alert-list"
        hx-target="#alert-list"
        hx-trigger="change"
        name="severity">
    <option value="all">All</option>
    <option value="critical">Critical</option>
    <option value="warning">Warning</option>
</select>
```

### 5. SVG charts

Server-generated SVG for:
- Score distribution histogram
- RS percentile distribution
- Regime timeline bar
- SPY price with 200-SMA

```rust
fn regime_timeline_svg(history: &[RegimeEntry]) -> String {
    let width = 800;
    let height = 40;
    let mut svg = format!("<svg width='{width}' height='{height}'>");
    for entry in history {
        let color = match entry.regime {
            Regime::Bull => "#51cf66",
            Regime::Sideways => "#ffd43b",
            Regime::Bear => "#ff6b6b",
        };
        svg += &format!("<rect x='{}' width='{}' height='40' fill='{}'/>",
            entry.x, entry.width, color);
    }
    svg += "</svg>";
    svg
}
```

### 6. CLI integration

```bash
# Start dashboard on default port
ghost-alpha serve

# Custom port and bind address
ghost-alpha serve --port 8080 --bind 0.0.0.0

# Open browser automatically
ghost-alpha serve --open
```

## Files to create/modify

| File | Action |
|------|--------|
| `src/web/mod.rs` | Create — web module root |
| `src/web/routes.rs` | Create — route handlers |
| `src/web/templates.rs` | Create — askama template structs |
| `src/web/assets.rs` | Create — embedded static files |
| `src/web/charts.rs` | Create — SVG chart generators |
| `src/main.rs` | Modify — add `Serve` subcommand |
| `src/lib.rs` | Modify — add `pub mod web` |
| `templates/*.html` | Create — 6 page templates + partials |
| `web/static/` | Create — htmx.min.js, pico.min.css |
| `Cargo.toml` | Modify — add axum, rust-embed, tower-http |

## Dependencies

```toml
[dependencies]
axum = "0.7"           # HTTP framework
tower-http = "0.5"     # Static file serving, CORS
rust-embed = "8"       # Embed static assets in binary
askama = "0.12"        # HTML templates (shared with Plan 07)
```

## Binary size impact

- axum + tower-http + hyper: ~1MB
- htmx.min.js + pico.css: ~25KB (embedded)
- Templates: negligible (compiled in)
- **Total: ~1MB** (binary goes from ~5MB to ~6MB)

## Testing

- Unit test: All routes return 200 for GET requests
- Unit test: Templates render without panic
- Unit test: SVG charts produce valid SVG
- Integration test: Start server, fetch home page, verify table
- Integration test: htmx partials return HTML fragments

## Acceptance criteria

- [ ] `ghost-alpha serve` starts a web server on port 3000
- [ ] Dashboard shows screening results, alerts, regime, watchlist
- [ ] No external CDN calls — all assets embedded in binary
- [ ] htmx provides interactivity without a JS build step
- [ ] Tables are sortable and filterable
- [ ] [Run Screen Now] button triggers a real screen and updates the table
- [ ] Responsive on mobile (pico.css handles this)
- [ ] Binary size stays under 7MB with dashboard included
