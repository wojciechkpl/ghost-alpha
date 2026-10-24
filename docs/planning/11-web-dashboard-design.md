I now have everything needed to write a thorough design document. Let me compose it.

---

# Ghost Alpha Web Dashboard — Comprehensive Design Document

## 1. Orientation: What Exists and What It Lacks

The current implementation in `src/web/` is a solid skeleton but it has several concrete weaknesses that must be corrected in the redesign:

- All pages do a full `hx-target="body" hx-swap="outerHTML"` after mutations, meaning every htmx interaction re-renders and replaces the entire page including the `<html>` root. This is incorrect htmx usage — it causes a flash, loses scroll position, and defeats the purpose of partial updates.
- Assets are still loaded from unpkg/jsDelivr CDN. This violates the single-binary requirement stated in the planning doc.
- The layout function inlines all CSS as a `<style>` block inside every full-page render. With askama templates and a proper asset module this becomes a single embedded file reference.
- The home page only shows today's results and has no date picker or historical browsing.
- The screener result table exposes only 7 columns out of the 18 fields in `ScreenResult`. The richer data (grade, close price, pct from 52-week high, ATR, R/R, liquidity, VCP flag) is invisible.
- There are no routes for flagged stocks, backtest results, strategies, stock detail, or settings — all of which are either in the planning doc or already have data in the DB.

---

## 2. Architecture

### 2.1 Template Strategy: Askama over raw format! strings

The planning doc already specifies askama. The current raw `format!()` approach in `routes.rs` should be replaced. The reasons are concrete, not stylistic:

- Type-checked at compile time: askama templates fail the build if a variable referenced in a template does not exist in the corresponding struct. Raw format strings catch nothing.
- Escaping: askama auto-escapes HTML by default. The current code has a hand-rolled `html_escape()` function that is called inconsistently — some fields (e.g. `r.ticker`, `r.composite_score`) are interpolated without escaping. If a ticker ever contained a `<` character (possible in malformed DB data) the current code would emit broken HTML.
- Separation of concerns: template files live in a `templates/` directory and can be read and edited without touching Rust code. This is important for iterating on the visual design.
- Partials: askama's `block` and `include` macros allow the navigation and `<head>` to live in a single `base.html` that all page templates extend. Changing the nav link order currently requires editing `routes.rs`.

The one trade-off: askama adds a compile-time dependency and makes the build slightly slower. Given the project's binary size constraint and the absence of a JS build step, this is a net positive — the templates are compiled to `&str` concatenation with no runtime overhead.

### 2.2 Asset Embedding

Add `rust-embed` to embed the static directory. The static directory should contain exactly four files:

```
web/static/
  htmx.min.js        (1.9.x, ~14KB minified)
  pico.min.css       (v2, ~35KB minified)
  ghost-alpha.css    (project custom overrides, ~2KB)
  favicon.svg        (tiny inline SVG, optional)
```

Add a new `src/web/assets.rs` module:

```rust
#[derive(rust_embed::RustEmbed)]
#[folder = "web/static/"]
pub struct StaticAssets;
```

Add a route `/static/:path` that serves these files with proper `Content-Type` and a long `Cache-Control: max-age=31536000, immutable` header. Because the files are embedded at compile time, the hash is stable per build — a query string version like `/static/htmx.min.js?v=CARGO_PKG_VERSION` is enough cache busting.

The `<head>` in `base.html` then becomes:

```html
<link rel="stylesheet" href="/static/pico.min.css">
<link rel="stylesheet" href="/static/ghost-alpha.css">
<script src="/static/htmx.min.js" defer></script>
```

No CDN calls. Binary size impact: ~50KB added to the embed, which is within the <7MB budget.

### 2.3 Router Structure

The router should be reorganised into three categories: full page routes, htmx partial routes, and API/action routes. This separation is important because partials must never emit a full HTML document — they emit only an HTML fragment that htmx swaps into a target element.

```
Full page routes (return complete HTML documents):
  GET  /                      -> home (screening results)
  GET  /stock/:ticker         -> stock detail
  GET  /alerts                -> alert history
  GET  /watchlist             -> watchlist management
  GET  /regime                -> market regime
  GET  /flagged               -> flagged stocks
  GET  /backtest              -> backtest results list
  GET  /backtest/:id          -> single backtest run (iframe embed)
  GET  /strategies            -> strategy comparison
  GET  /settings              -> config viewer

htmx partial routes (return HTML fragments, no <html> wrapper):
  GET  /partials/screen-table -> re-render results table (date + filter params)
  GET  /partials/alert-list   -> re-render alert rows (severity + ticker filter)
  GET  /partials/watchlist-rows -> re-render watchlist table body
  GET  /partials/regime-strip -> re-render regime header bar
  GET  /partials/flagged-rows -> re-render flagged stocks table body
  GET  /partials/stock-candles/:ticker -> SVG chart for stock detail

API/action routes (perform mutations, return 204 or redirect):
  POST /api/watchlist/add     -> add ticker, returns updated /partials/watchlist-rows
  POST /api/watchlist/remove  -> remove ticker, returns updated /partials/watchlist-rows
  POST /api/screen/run        -> trigger screen run (SSE stream), returns job ID
  GET  /api/screen/status/:id -> SSE stream of progress events
  GET  /api/export/csv        -> download today's results as CSV
  GET  /api/export/json       -> download today's results as JSON

Static assets:
  GET  /static/:path          -> serve embedded files
```

The current `watchlist_add` and `watchlist_remove` handlers return the full watchlist page via a re-render of `watchlist_page()`. They should instead return only the `<tbody>` fragment so htmx can swap just the table body. The form's `hx-target` becomes `#watchlist-tbody` and `hx-swap` becomes `innerHTML`.

### 2.4 htmx Interaction Patterns

**Pattern 1 — Table refresh with polling**

The home page table auto-refreshes every 5 minutes by polling the partial endpoint:

```html
<tbody id="screen-tbody"
       hx-get="/partials/screen-table"
       hx-trigger="load, every 300s"
       hx-swap="innerHTML">
  <!-- rows rendered server-side on initial load -->
</tbody>
```

This avoids a blank flash on load (server renders initial rows inline) while keeping the table live.

**Pattern 2 — Filter controls swap the same partial**

The date picker and filter dropdowns on the home page all target the same `#screen-tbody`:

```html
<input type="date" name="date"
       hx-get="/partials/screen-table"
       hx-trigger="change"
       hx-target="#screen-tbody"
       hx-swap="innerHTML"
       hx-include="[name='strategy'],[name='min-score'],[name='minervini-only']">
```

The partial handler reads `date`, `strategy`, `min-score`, and `minervini-only` as query parameters and filters the DB results accordingly. All filter controls use `hx-include` to bundle their values into the request automatically, so no JavaScript is needed to coordinate them.

**Pattern 3 — Mutation returns the updated fragment**

When the user adds a watchlist ticker, the handler upserts the DB row and returns the updated `<tbody>` HTML. htmx swaps it in-place. The outer table, the add form, and the page title remain untouched. This is what `hx-target="#watchlist-tbody"` with `hx-swap="innerHTML"` achieves.

**Pattern 4 — Screen run via SSE**

The "Run Screen Now" button triggers a POST that creates a background job and returns a job ID, then the browser opens an SSE connection to stream progress:

```html
<button hx-post="/api/screen/run"
        hx-target="#run-feedback"
        hx-swap="innerHTML">
  Run Screen Now
</button>
<div id="run-feedback"></div>
```

The POST handler returns:

```html
<div hx-ext="sse"
     sse-connect="/api/screen/status/JOB_ID"
     sse-swap="message"
     hx-target="#screen-tbody"
     hx-swap="innerHTML">
  Running... <progress></progress>
</div>
```

The SSE stream sends progress events (`event: message`, `data: <html fragment>`) and a final `event: done` that htmx can use to close the connection. This is the cleanest approach for a potentially 30–60 second operation that depends on Finnhub API calls.

Note: SSE requires `hx-ext="sse"` which is a separate extension file (`htmx-ext-sse.js`, ~2KB). Add it to `web/static/` and embed it alongside the main htmx file.

**Pattern 5 — Inline details expansion**

On the alerts page, clicking a row expands a details panel below it:

```html
<tr hx-get="/partials/alert-detail/ID"
    hx-target="next tr.alert-detail"
    hx-swap="outerHTML"
    hx-trigger="click">
```

The next `<tr class="alert-detail">` starts hidden and gets replaced with the populated detail row. This avoids a navigation away from the page.

---

## 3. Pages and Components

### 3.1 Dashboard Home — `/`

**Data sources:**
- `repo.get_screen_results(date)` — primary table data
- `repo.latest_regime()` — regime badge in header
- `repo.get_active_flagged_stocks()` — count for the flagged badge in nav
- Available dates from screen_results table for the date picker (query: `SELECT DISTINCT run_date FROM screen_results ORDER BY run_date DESC LIMIT 60`)

**Columns (expanded from current 7 to 12):**

| # | Ticker | Score | Grade | Minervini | RS% | Vol-Z | ADX | Price | % from High | R/R | Liquidity |

The Grade column uses colored badges (A+ = deep green, A = green, B = teal, C = yellow, D = orange, F = red). This is more useful than the raw composite score alone.

The VCP column is optional — show a small "VCP" badge only when `vcp.is_some()` and the VCP passed its quality checks. This is a rare flag and should not occupy a full column.

**ASCII wireframe:**

```
┌──────────────────────────────────────────────────────────────────────────┐
│ Ghost Alpha   Home  Alerts(3)  Watchlist  Regime  Flagged(2)  Backtest   │
│              ─────────────────── [Regime: BULL  SPY $522 / SMA200 $481] ─│
├──────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  Screening Results                         Last run: 2026-03-07 18:30   │
│                                                                          │
│  Date: [2026-03-07 v]  Strategy: [Minervini v]  Min Score: [50 ─────]   │
│  [x] Minervini pass only                     [Run Screen Now] [CSV] [JSON]│
│                                                                          │
│  ┌────┬──────┬───────┬───────┬───────────┬─────┬──────┬─────────────┐   │
│  │ #  │Ticker│ Score │ Grade │ Minervini │ RS% │ Vol-Z│   Actions   │   │
│  ├────┼──────┼───────┼───────┼───────────┼─────┼──────┼─────────────┤   │
│  │  1 │ NVDA │  94.2 │ [A+]  │  8/8  [✓] │  97 │  2.8 │ [Detail][+W]│   │
│  │  2 │ PLTR │  87.1 │ [A]   │  7/8  [✓] │  91 │  1.9 │ [Detail][+W]│   │
│  │  3 │ META │  82.5 │ [A]   │  8/8  [✓] │  85 │  1.2 │ [Detail][+W]│   │
│  │ .. │  ... │  ...  │  ...  │    ...    │ ... │  ... │    ...      │   │
│  └────┴──────┴───────┴───────┴───────────┴─────┴──────┴─────────────┘   │
│                                                                          │
│  [< Prev Page]  Showing 1–20 of 47 results  [Next Page >]               │
│                                                                          │
│  ● Progress: ──────────────────────── (shown only during Run Screen Now) │
└──────────────────────────────────────────────────────────────────────────┘
```

**htmx interactions:**
- Date picker change → GET `/partials/screen-table?date=X&strategy=Y&min_score=Z&minervini_only=W` → swap `#screen-tbody`
- Strategy dropdown change → same partial with updated strategy param
- Min score slider (input type="range") → debounced (htmx's `hx-trigger="change delay:300ms"`) → same partial
- Minervini checkbox → same partial
- [Run Screen Now] → POST `/api/screen/run` → SSE stream → swap `#screen-tbody` on completion
- [CSV] / [JSON] → plain anchor `href="/api/export/csv?date=X"` — browser download, no htmx needed
- [+W] (add to watchlist) → POST `/api/watchlist/add` → swap inline feedback span in that row
- [Detail] → navigate to `/stock/:ticker` (full page, no htmx)
- Page N / prev / next → GET `/partials/screen-table?page=N&...` → swap `#screen-tbody`

**Pagination strategy:**
The DB query uses `LIMIT 20 OFFSET (page * 20)`. The partial returns both the rows and a pagination footer as a single fragment. The partial endpoint reads `page` from the query string, defaulting to 0.

### 3.2 Stock Detail — `/stock/:ticker`

This page does not currently exist. It is the most important missing page.

**Data sources:**
- `repo.get_candles(ticker, 252)` — 252 days of OHLCV
- `repo.get_screen_results()` — filter by ticker, last 30 occurrences (score history)
- `repo.get_cached_news(ticker, since: 30 days ago)` — recent news articles
- `repo.get_active_flagged_stocks()` — check if ticker is currently flagged
- `repo.watchlist_list()` — check if ticker is on watchlist

**ASCII wireframe:**

```
┌──────────────────────────────────────────────────────────────────────────┐
│ < Back to Results         NVDA — NVIDIA Corporation          [+ Watchlist]│
│                           Sector: Technology / Semiconductors             │
├──────────────────────────────────────────────────────────────────────────┤
│  ┌─────────────────────────────────────────────────────────────────────┐ │
│  │                   SVG price chart (252 days)                        │ │
│  │   Close ──── SMA50 ---- SMA150 ···· SMA200                         │ │
│  │                                                                     │ │
│  └─────────────────────────────────────────────────────────────────────┘ │
│                                                                          │
│  Score Breakdown          Minervini Checklist                            │
│  ┌──────────────────┐     ┌──────────────────────────────────────┐       │
│  │ Composite:  94.2 │     │ ✓ Close > SMA 150 and SMA 200        │       │
│  │ Technical:  88.0 │     │ ✓ SMA 150 > SMA 200                  │       │
│  │ Volume:     95.0 │     │ ✓ SMA 200 trending up ≥ 1 month      │       │
│  │ Sentiment:  70.0 │     │ ✓ SMA 50 > SMA 150 and SMA 200       │       │
│  │ Grade:      A+   │     │ ✓ Close > SMA 50                     │       │
│  │ ADX:        38.4 │     │ ✓ Close ≥ 30% above 52-week low      │       │
│  │ RS%:         97  │     │ ✗ Close within 25% of 52-week high   │       │
│  │ Vol-Z:      2.8  │     │ ✓ RS rank in top 30% of universe     │       │
│  └──────────────────┘     └──────────────────────────────────────┘       │
│                                                                          │
│  Score History (last 30 screens)                                         │
│  [sparkline SVG bar chart]                                               │
│                                                                          │
│  Recent News                                                             │
│  2026-03-07  Headline text here...                    [Source]           │
│  2026-03-06  Another headline...                      [Source]           │
└──────────────────────────────────────────────────────────────────────────┘
```

**SVG chart for price history:**
This is the most technically interesting piece. The server renders it as inline SVG. The chart needs five data series: close, SMA50, SMA150, SMA200, and volume bars. The approach is:

1. Fetch 252 candles from DB.
2. Compute SMA50, SMA150, SMA200 using the existing `sma()` function from `src/analysis/indicators.rs`.
3. Normalise all prices to a 0–1 range for the SVG coordinate space.
4. Map each candle to an x-coordinate: `x = (i as f64 / candle_count as f64) * svg_width`.
5. Emit a `<polyline>` for each MA and the close price series.
6. Emit `<rect>` elements for volume bars in a lower panel.
7. Emit horizontal `<line>` elements for the 52-week high and low.

The SVG has a fixed viewBox (e.g. `0 0 800 300`) with the price panel occupying the top 220 pixels and volume bars the bottom 60 pixels. `preserveAspectRatio="xMidYMid meet"` ensures it scales on mobile.

A separate partial route `/partials/stock-candles/:ticker` returns just the SVG fragment, allowing the chart to be refreshed independently.

**Minervini checklist rendering:**
The `check_minervini_conditions()` function in `indicators.rs` returns a bitmask or a `Vec<bool>` of the 8 conditions. The stock detail page iterates through all 8 and shows a checkmark or cross for each, with the full human-readable description. This is far more informative than the current `7/8` summary.

### 3.3 Alert History — `/alerts`

**Data sources:**
- `repo.get_recent_alerts(limit)` — primary data, paginated
- Filter parameters: severity (CRITICAL/WARNING/INFO/ALL), ticker substring, date range

**ASCII wireframe:**

```
┌──────────────────────────────────────────────────────────────────────────┐
│ Alert History                                                            │
│                                                                          │
│  Severity: [All v]  Ticker: [_________]  From: [______]  To: [______]   │
│                                                                [Filter]  │
│                                                                          │
│  ┌──────────┬──────────┬──────────┬───────────────────────────┬────────┐ │
│  │  Ticker  │   Type   │ Severity │         Message           │  When  │ │
│  ├──────────┼──────────┼──────────┼───────────────────────────┼────────┤ │
│  │  NVDA    │ BREAKOUT │[CRITICAL]│ New 52-week high breakout │ 18:31  │ │
│  │ ─ detail─│──────────────────────────────────────────────── expanded ─│ │
│  │  Score: 94.2 │ Vol-Z: 2.8 │ Conditions: 8/8               │        │ │
│  │  Narrative: "NVDA shows strong momentum..."               │        │ │
│  ├──────────┼──────────┼──────────┼───────────────────────────┼────────┤ │
│  │  PLTR    │ ENTRY    │[WARNING] │ Score crossed 70 threshold │ 18:31 │ │
│  └──────────┴──────────┴──────────┴───────────────────────────┴────────┘ │
│                                                                          │
│  [< Prev]  Page 1 of 4  [Next >]                                         │
└──────────────────────────────────────────────────────────────────────────┘
```

**htmx interactions:**
- All filter controls → GET `/partials/alert-list?severity=X&ticker=Y&from=Z&to=W&page=N` → swap `#alert-tbody`
- Row click → GET `/partials/alert-detail/:id` → swap `next tr.alert-detail` (inline expansion)
- Pagination links → GET same partial with page param → swap `#alert-tbody`

The DB query for the partial needs to support `WHERE severity = ? AND ticker LIKE ? AND sent_at BETWEEN ? AND ?`. The current `get_recent_alerts()` on the `Repository` trait only takes a limit. A new method `get_alerts_filtered(severity: Option<&str>, ticker: Option<&str>, from: Option<NaiveDate>, to: Option<NaiveDate>, limit: usize, offset: usize)` must be added to the trait and implemented. This is a targeted enhancement, not a redesign of the storage layer.

### 3.4 Watchlist — `/watchlist`

**Data sources:**
- `repo.watchlist_list()` — primary
- For each ticker, join with the most recent screen result to show the current score

**ASCII wireframe:**

```
┌──────────────────────────────────────────────────────────────────────────┐
│ Watchlist  (6 tickers)                                                   │
│                                                                          │
│  Add ticker: [________] [optional note_______________] [Add]            │
│                                                                          │
│  ┌──────────┬──────────┬───────┬───────┬──────────────────┬──────────┐  │
│  │  Ticker  │  Added   │ Score │ Grade │   Score Trend    │  Action  │  │
│  ├──────────┼──────────┼───────┼───────┼──────────────────┼──────────┤  │
│  │  NVDA    │ 2026-03-01│ 94.2 │ [A+]  │ [sparkline SVG]  │ [Detail][X]│ │
│  │  PLTR    │ 2026-02-15│ 87.1 │ [A]   │ [sparkline SVG]  │ [Detail][X]│ │
│  └──────────┴──────────┴───────┴───────┴──────────────────┴──────────┘  │
│                                                                          │
│  Notes: (click ticker to expand)                                         │
└──────────────────────────────────────────────────────────────────────────┘
```

**Score trend sparklines:**
For each watchlist ticker, query `SELECT run_date, composite_score FROM screen_results WHERE ticker = ? ORDER BY run_date DESC LIMIT 30`. Render a tiny SVG polyline (e.g. 80×20 px, no axes) showing the score over time. Colour the line green if the last point is above the first (improving), red if declining.

**htmx interactions:**
- Add form submit → POST `/api/watchlist/add` → returns updated `#watchlist-tbody` fragment
- [X] button → POST `/api/watchlist/remove` (with `hx-confirm="Remove NVDA?"`) → returns updated `#watchlist-tbody`
- [Detail] → navigate to `/stock/:ticker`

The important fix over the current implementation: the add/remove handlers must target `#watchlist-tbody` with `hx-swap="innerHTML"`, not `hx-target="body" hx-swap="outerHTML"`. The full page is only needed for the initial GET.

### 3.5 Market Regime — `/regime`

**Data sources:**
- `repo.latest_regime()` — current regime
- `repo.latest_macro_risk()` — VIX, HY OAS, yield curve
- Historical regime data: new DB query `SELECT date, regime, spy_close, spy_sma200, breadth_pct FROM regime_state ORDER BY date DESC LIMIT 252` for the timeline

**ASCII wireframe:**

```
┌──────────────────────────────────────────────────────────────────────────┐
│ Market Regime                                                            │
│                                                                          │
│  Current: [BULL] as of 2026-03-07                                        │
│  SPY: $522.14  /  SMA200: $481.20  (+8.5% above)                        │
│  Market Breadth: 72.5% stocks above 200-day MA                           │
│                                                                          │
│  Regime History (last 12 months)                                         │
│  ┌──────────────────────────────────────────────────────────────────┐    │
│  │[████ BULL ████████████████████▓▓▓▓SIDE▓▓▓██ BULL ████████████]  │    │
│  │ Jan 2025            Jun 2025            Dec 2025     Mar 2026    │    │
│  └──────────────────────────────────────────────────────────────────┘    │
│                                                                          │
│  SPY Price Chart (with SMA200)                                           │
│  ┌──────────────────────────────────────────────────────────────────┐    │
│  │   [SVG line chart: close + SMA200, 252 days]                     │    │
│  └──────────────────────────────────────────────────────────────────┘    │
│                                                                          │
│  Macro Risk Indicators                                                    │
│  ┌─────────────┬────────────┬──────────────────┬──────────────────┐     │
│  │  Risk Level │    VIX     │    HY OAS (bps)  │ Yield Curve 2s10s│     │
│  │   [NORMAL]  │   16.8     │       385        │    +0.42%        │     │
│  └─────────────┴────────────┴──────────────────┴──────────────────┘     │
└──────────────────────────────────────────────────────────────────────────┘
```

**Regime timeline SVG:**
The timeline is a horizontal bar chart where each segment is a contiguous block of days under the same regime. Width is proportional to duration. Use `#51cf66` for Bull, `#ffd43b` for Sideways, `#ff6b6b` for Bear. Render tick labels at Jan 1 of each year and today. The SVG viewBox is `0 0 900 60`.

**htmx interactions:**
- The regime page is mostly static — data changes only when a screen run completes. Use `hx-trigger="load, every 600s"` on the main content `<div>` to poll a partial.
- A "Refresh" button triggers an immediate partial reload.

**Required new DB method:**
`fn get_regime_history(&self, limit: usize) -> Result<Vec<RegimeState>, StorageError>` — the current schema stores regime in `regime_state` table and the existing `latest_regime()` only returns one row. The historical query requires `SELECT * FROM regime_state ORDER BY date DESC LIMIT ?`.

### 3.6 Flagged Stocks — `/flagged`

This page currently does not exist in the router. The data model is fully implemented in `FlaggedStock` and the DB methods (`get_active_flagged_stocks`, `mark_exited`, `expire_old_flags`) exist.

**Data sources:**
- `repo.get_active_flagged_stocks()` — primary
- Join with latest screen result for current score (to compute conviction decay: `latest_score / entry_score`)

**ASCII wireframe:**

```
┌──────────────────────────────────────────────────────────────────────────┐
│ Flagged Stocks  (Active Positions)                                       │
│                                                                          │
│  ┌──────┬────────────┬────────┬────────┬────────┬────────┬────────────┐  │
│  │Ticker│  Flagged   │ Entry  │ Entry  │Current │Conviction│  Status  │  │
│  │      │   Date     │ Score  │ Price  │ Score  │  Decay  │          │  │
│  ├──────┼────────────┼────────┼────────┼────────┼──────────┼──────────┤  │
│  │ NVDA │ 2026-02-20 │  91.0  │ $498.20│  94.2  │[+3.5%][^]│[ACTIVE]│  │
│  │ PLTR │ 2026-02-15 │  88.0  │ $32.10 │  72.1  │[-18.1%][v]│[WATCH]│  │
│  └──────┴────────────┴────────┴────────┴────────┴──────────┴──────────┘  │
│                                                                          │
│  Exit Signals                                                            │
│  [WARNING] PLTR: Conviction decay -18% — below warning threshold        │
│                                                                          │
│  Exited / Expired (last 30 days)                                         │
│  ┌──────┬──────────┬─────────┬────────────────────────────────────────┐  │
│  │Ticker│ Exit Date│ Exit Px │ Exit Reason                            │  │
│  │ TSLA │2026-03-01│ $245.80 │ Exit signal: stop-loss breach          │  │
│  └──────┴──────────┴─────────┴────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────────────┘
```

**Conviction decay visualisation:**
The `latest_score / entry_score - 1` gives the percentage change. Render a coloured badge: green if > +5%, yellow if -5% to +5%, orange if -5% to -15%, red if < -15%. The badges map directly to the alert thresholds already in `AlertConfig`.

**htmx interactions:**
- Table rows auto-refresh every 60 seconds using `hx-trigger="every 60s"` on `#flagged-tbody`.
- No inline mutations — exit signals are generated by the screener, not the UI.

### 3.7 Backtest Results — `/backtest`

**Data sources:**
- `repo.get_backtest_runs(10)` — list of recent runs (id, date, signal_count, kpis_json, duration_secs)
- Individual run: `report_json` stored as the full serialised `BacktestReport`

**Page structure — list view (`/backtest`):**

```
┌──────────────────────────────────────────────────────────────────────────┐
│ Backtest Results                                                         │
│                                                                          │
│  ┌────┬─────────────┬─────────┬──────────┬──────────┬────────────────┐  │
│  │ ID │ Run Date    │ Signals │ Hit Rate │  Sharpe  │    Actions     │  │
│  ├────┼─────────────┼─────────┼──────────┼──────────┼────────────────┤  │
│  │  3 │ 2026-03-05  │   1240  │  63.2%   │   1.42   │ [View] [HTML]  │  │
│  │  2 │ 2026-02-15  │   1180  │  61.8%   │   1.31   │ [View] [HTML]  │  │
│  └────┴─────────────┴─────────┴──────────┴──────────┴────────────────┘  │
└──────────────────────────────────────────────────────────────────────────┘
```

**Individual run view (`/backtest/:id`):**

The `report_json` field contains the full `BacktestReport`. The handler deserialises it and passes it to the existing `generate_html_report()` function from `src/backtest/html_report.rs`. The result is served directly as `text/html`. This renders the full self-contained report within the browser but not wrapped in the Ghost Alpha chrome — that is intentional, because the backtest report is itself a complete document.

The [HTML] button on the list view provides a download: `GET /api/backtest/:id/download` returns the same content with `Content-Disposition: attachment; filename="backtest_YYYY-MM-DD.html"`.

The [View] button navigates to `/backtest/:id` which serves the report inline in the browser.

**Pagination:** The `get_backtest_runs(limit)` already takes a limit. Add an `offset` parameter for pagination if needed, but 10 runs per page is likely sufficient for most users.

### 3.8 Strategies — `/strategies`

**Data sources:**
- Static data: strategy names, descriptions, and weight distributions come from `ScoringWeights::standard()`, `::breakout()`, `::mean_reversion()`, `::trend_strength()` in `src/analysis/screener.rs`.
- Per-strategy statistics: if backtest runs exist for multiple strategies, the relevant KPIs can be pulled from `kpis_json`.

**ASCII wireframe:**

```
┌──────────────────────────────────────────────────────────────────────────┐
│ Screening Strategies                                                     │
│                                                                          │
│  ┌──────────────┬──────────────┬──────────────┬──────────────────────┐  │
│  │  Minervini   │   Breakout   │Mean Reversion│   Trend Strength     │  │
│  │ (default)    │              │              │                      │  │
│  ├──────────────┼──────────────┼──────────────┼──────────────────────┤  │
│  │ Tech:    55% │ Tech:    45% │ Tech:    55% │ Tech:    75%         │  │
│  │ Volume:  30% │ Volume:  40% │ Volume:  20% │ Volume:  10%         │  │
│  │ Sentiment:15%│ Sentiment:15%│ Sentiment:25%│ Sentiment:15%        │  │
│  ├──────────────┼──────────────┼──────────────┼──────────────────────┤  │
│  │ Strong trend │ Near 52-week │ Oversold     │ ADX-driven           │  │
│  │ with volume  │ highs + vol  │ mean-rev     │ directional move     │  │
│  │ confirmation │ surge        │ candidates   │                      │  │
│  ├──────────────┼──────────────┼──────────────┼──────────────────────┤  │
│  │[Run with this│[Run with this│[Run with this│[Run with this strat] │  │
│  │  strategy]   │  strategy]   │  strategy]   │                      │  │
│  └──────────────┴──────────────┴──────────────┴──────────────────────┘  │
└──────────────────────────────────────────────────────────────────────────┘
```

**htmx interactions:**
- [Run with this strategy] → POST `/api/screen/run?strategy=breakout` → same SSE progress flow as the home page button → on completion, navigate to `/` with the strategy filter pre-selected.

### 3.9 Settings — `/settings`

**Data sources:**
- `state.config` — the loaded `AppConfig` struct

**Design principles:**
- This is read-only. Configuration is done via TOML file; the UI only shows what is currently loaded.
- Secrets (API keys) are masked: show `"ghst_****...****"` not the full key.
- The DB path, alert thresholds, active provider, and strategy defaults are shown in full.

**ASCII wireframe:**

```
┌──────────────────────────────────────────────────────────────────────────┐
│ Settings (read-only — edit config.toml to change)                        │
│                                                                          │
│  Data Provider                                                           │
│    Active provider:   finnhub                                            │
│    Finnhub API key:   ghst_****...****   Rate limit: 60 RPM             │
│                                                                          │
│  Alert Thresholds                                                        │
│    Critical:  score ≥ 90 OR (Minervini pass AND Vol-Z ≥ 2.0)           │
│    Warning:   score ≥ 70                                                 │
│    Info:      score ≥ 50                                                 │
│    Dedup cooldown:  7 days                                               │
│    Max daily alerts:  50                                                 │
│    Quiet hours:  22:00 – 07:00                                           │
│                                                                          │
│  Storage                                                                 │
│    DB path:  ghost_alpha.db                                              │
│    Candle retention:  (default)                                          │
│                                                                          │
│  Notification Channels                                                   │
│    Telegram:  [configured]  Chat ID: -100****                           │
│    Discord:   [not configured]                                           │
│                                                                          │
│  Universe: 503 tickers loaded                                            │
└──────────────────────────────────────────────────────────────────────────┘
```

No htmx interactions — the settings page is a static display. There is no auto-refresh needed.

---

## 4. Data Flow

### 4.1 Initial Page Load

Every full-page GET handler follows this pattern:

1. Clone `Arc<SqliteRepository>`.
2. Execute DB queries inside `tokio::task::spawn_blocking` (the repository is synchronous, using `Mutex<Connection>`).
3. Pass the resulting data structs to an askama template render call.
4. Return `Html<String>`.

The current implementation already follows this pattern correctly. The main change is replacing the raw `format!()` calls with askama template structs.

### 4.2 htmx Partial Requests

Partial handlers share the same pattern but return only an HTML fragment (no `<!DOCTYPE html>` wrapper). Their return type is `Html<String>` but the string content begins with a `<tr>`, `<tbody>`, or `<div>` element.

Partials must set the `HX-Trigger` response header when a mutation changes shared state that other parts of the page need to reflect. For example, after adding a watchlist ticker, the response should include `HX-Trigger: watchlist-updated` so that any element with `hx-trigger="watchlist-updated from:body"` can refresh itself.

### 4.3 Auto-Refresh Strategy

| Page / Component | Trigger | Interval |
|---|---|---|
| Home screen table | `hx-trigger="every 300s"` on `<tbody>` | 5 minutes |
| Regime header strip | `hx-trigger="every 600s"` on regime badge | 10 minutes |
| Flagged stocks table | `hx-trigger="every 60s"` on `<tbody>` | 1 minute |
| Alert count badge in nav | `hx-trigger="every 120s"` | 2 minutes |
| All other pages | No auto-refresh | — |

The alert count badge in the nav needs a small partial endpoint: `GET /partials/nav-alerts-count` → returns `<span id="nav-alerts-badge">N</span>`. This span is `hx-swap="outerHTML"` so the entire element (including the count) is replaced.

### 4.4 Screen Run Progress via SSE

The "Run Screen Now" flow is the most complex interaction. The architectural choice is to run the screening pipeline in a background Tokio task, stream progress events over SSE, and swap the results table when done.

The `AppState` needs one addition: an `Arc<Mutex<Option<tokio::sync::broadcast::Sender<String>>>>` for the current screen job. Alternatively use a `tokio::sync::watch` channel. A simpler approach that avoids shared state complexity: the screen run is long-polling. The POST returns a job ID immediately, and the browser polls `/api/screen/status/:id` every 2 seconds. When the job is done, the poll returns the completed HTML fragment.

The polling approach is simpler to implement correctly and debug, at the cost of up to 2 seconds of additional latency before the table updates. Given that screen runs take 30–120 seconds, 2 seconds of extra latency is unimportant.

For the SSE approach to work cleanly, axum's `axum::response::sse::Sse` type can be used with a `tokio::sync::broadcast` channel. The screen job broadcasts progress strings; the SSE handler subscribes and forwards them. This is the preferred approach for responsive feedback but adds ~30 lines of plumbing.

The recommendation is to start with polling (simpler, easier to test) and upgrade to SSE in a follow-up.

---

## 5. Visual Design

### 5.1 Color Palette

The current implementation defines badges inline. These should be extracted into CSS custom properties in `ghost-alpha.css`:

```css
:root {
  /* Severity */
  --color-critical:      #ff6b6b;
  --color-critical-text: #fff;
  --color-warning:       #ffd43b;
  --color-warning-text:  #1a1a1a;
  --color-info:          #74c0fc;
  --color-info-text:     #1a1a1a;

  /* Regime */
  --color-bull:          #51cf66;
  --color-bull-text:     #fff;
  --color-bear:          #ff6b6b;
  --color-bear-text:     #fff;
  --color-sideways:      #ffd43b;
  --color-sideways-text: #1a1a1a;

  /* Grades */
  --color-grade-aplus:   #2f9e44;
  --color-grade-a:       #51cf66;
  --color-grade-b:       #20c997;
  --color-grade-c:       #ffd43b;
  --color-grade-d:       #ff922b;
  --color-grade-f:       #ff6b6b;

  /* Conviction decay */
  --color-improving:     #51cf66;
  --color-stable:        #ffd43b;
  --color-declining:     #ff922b;
  --color-critical-decay:#ff6b6b;

  /* UI */
  --font-size-base:      14px;
  --nav-height:          3rem;
  --sidebar-width:       0;   /* no sidebar — full-width layout */
}
```

All `.badge-*` classes reference these variables. Dark mode support is handled by a `@media (prefers-color-scheme: dark)` block that overrides the light values — pico.css already handles most of this, but the custom badge colours need explicit dark mode overrides.

### 5.2 Typography and Spacing

Pico.css v2 uses `--pico-font-size` for the base font size. Set it to `14px` as the current implementation already does. The table is the primary UI surface; `font-size: 13px` for table cells keeps more columns visible. Ticker symbols should be `font-family: monospace` and `font-weight: bold` to stand out.

Column widths:
- `#` rank: 3rem (fixed)
- Ticker: 6rem (fixed, monospace)
- Score: 5rem (fixed, right-aligned)
- Grade: 4rem (fixed, centred)
- Numeric metrics (RS%, Vol-Z, ADX): 5rem each (right-aligned)
- Price: 6rem (right-aligned)
- % from High: 6rem (right-aligned)
- Actions: auto (left over)

Use `table-layout: fixed` with explicit widths on `<colgroup>` elements to prevent layout shift when the table body is swapped by htmx.

### 5.3 Dark Mode

Pico.css v2 has built-in dark mode via `data-theme="dark"` on `<html>`. Add a toggle button in the nav:

```html
<button id="theme-toggle" onclick="
  const html = document.documentElement;
  html.dataset.theme = html.dataset.theme === 'dark' ? 'light' : 'dark';
  localStorage.setItem('theme', html.dataset.theme);
" aria-label="Toggle dark mode">
  ☀/☾
</button>
```

This is 3 lines of inline JavaScript — the only JS written by hand in the entire dashboard. On initial page load, a small `<script>` in `<head>` reads `localStorage.getItem('theme')` and applies it before the page paints, preventing flash. This 5-line script is the only reason `defer` should not be used on the theme restoration script (all other scripts, including htmx, use `defer`).

SVG chart colours need to respect the current theme. Use CSS `currentColor` for lines and query `--pico-color` for the default text colour where possible. Chart backgrounds should be `transparent` so the pico.css body background shows through.

### 5.4 Mobile Responsiveness

Pico.css handles most of this. The table is the problem: 12 columns cannot fit on a 375px wide screen. Strategies:

1. On screens narrower than 768px, hide the lower-priority columns (ADX, Price, % from High, R/R, Liquidity) using `display: none` via a `@media` rule.
2. The remaining 6 columns (Rank, Ticker, Score, Grade, RS%, Actions) fit comfortably.
3. On the stock detail page, the score breakdown table and the Minervini checklist stack vertically below 768px (use pico.css's grid with `grid-template-columns: 1fr` on small screens).
4. The SVG charts use `width: 100%` with `viewBox` set for their natural size, so they scale automatically.

---

## 6. Performance

### 6.1 Queries That Need Caching

The `SqliteRepository` uses a `Mutex<Connection>`, which means all queries are serialised. For a single-user dashboard this is fine, but certain queries are slow enough to warrant an in-memory cache in `AppState`:

| Query | Cache Strategy | TTL |
|---|---|---|
| `get_screen_results(today)` | `Arc<RwLock<Option<(NaiveDate, Vec<ScreenResult>)>>>` in AppState | Invalidated when screen run completes |
| `latest_regime()` | Same pattern | 10 minutes |
| `get_regime_history(252)` | Same pattern | 10 minutes |
| Candle fetch for stock detail chart | Per-ticker `DashMap<String, (Instant, Vec<Candle>)>` | 5 minutes |

The backtest report is already stored as JSON in the DB and just needs to be deserialised and rendered — no additional caching needed since it's infrequently accessed.

Do not cache the alert count badge or the flagged stocks list — these need to be as fresh as possible.

### 6.2 Pagination

All table partials accept `?page=N&per_page=20` query parameters. The DB queries use `LIMIT ? OFFSET ?`. The pagination footer is rendered as part of the same partial response so the page controls update atomically with the rows.

The total count query (`SELECT COUNT(*) FROM screen_results WHERE run_date = ?`) is cheap and should be run alongside the page query to display "Showing 21–40 of 47 results".

### 6.3 SVG Chart Generation

The stock detail chart (252 candles × 5 data series) involves ~1260 coordinate calculations. This is fast (< 1ms in Rust) but should still be behind the partial route `/partials/stock-candles/:ticker` rather than inline in the full page render. This way the page loads immediately and the chart appears shortly after, avoiding a perceptible delay if the DB query is slow.

The regime page SPY chart uses the same mechanism: `/partials/regime-chart` is loaded lazily via `hx-trigger="load"` on the chart container `<div>`.

### 6.4 Lazy Loading for Expensive Sections

The stock detail page has three expensive sections: the price chart (DB read + SVG rendering), the score history sparkline (DB read), and the news list (DB read with full text). All three should be loaded as separate partials triggered on load:

```html
<div hx-get="/partials/stock-candles/NVDA" hx-trigger="load" hx-swap="innerHTML">
  <div aria-busy="true">Loading chart...</div>
</div>
```

This keeps the initial page paint fast (just the static KPI table and checklist) and fills in the heavier content progressively.

---

## 7. Module Structure

The final file layout for `src/web/`:

```
src/web/
  mod.rs          — AppState, create_router, (tests)
  routes.rs       — full-page handlers (GET /  GET /stock/:ticker  etc.)
  partials.rs     — htmx partial handlers (GET /partials/*)
  actions.rs      — mutation handlers (POST /api/*), export endpoints
  assets.rs       — rust-embed struct + static file serve handler
  charts.rs       — SVG generation: price_chart_svg(), sparkline_svg(),
                    regime_timeline_svg(), score_histogram_svg()
  filters.rs      — askama custom filters (e.g. format_grade_badge,
                    format_severity_badge, format_f64_1dp)

templates/
  base.html             — full page layout: <html><head><body><nav>
  home.html             — extends base, screen results table shell
  stock.html            — extends base, stock detail layout
  alerts.html           — extends base, alert table shell
  watchlist.html        — extends base, watchlist table shell
  regime.html           — extends base, regime display
  flagged.html          — extends base, flagged stocks table shell
  backtest_list.html    — extends base, runs table
  strategies.html       — extends base, strategy grid
  settings.html         — extends base, config display
  partials/
    screen_table.html   — <tbody> rows only
    alert_list.html     — <tbody> rows + pagination
    alert_detail.html   — single expanded row
    watchlist_rows.html — <tbody> rows only
    flagged_rows.html   — <tbody> rows only
    regime_chart.html   — SVG + surrounding markup
    nav_badge.html      — <span> count only
    stock_chart.html    — SVG only
    score_sparkline.html— sparkline SVG only

web/static/
  htmx.min.js
  htmx-ext-sse.js
  pico.min.css
  ghost-alpha.css
```

---

## 8. Key Implementation Risks and Mitigations

**Risk 1: The `Mutex<Connection>` bottleneck under SSE**

When the SSE screen-run job is running, it will acquire the `Mutex<Connection>` repeatedly to save results. Meanwhile, incoming page requests also need the connection. Under high load this serialises everything. Mitigation: the dashboard is single-user, so this is not a practical problem. Document it. If it becomes an issue, move to a `tokio::sync::RwLock` or separate the write connection from the read connections using WAL mode (`PRAGMA journal_mode=WAL`).

**Risk 2: Askama template compilation time**

Askama compiles templates at build time. With 15+ templates, this adds a few seconds to the first build. This is acceptable. Use `askama`'s `#[template(path = "...", escape = "html")]` consistently and ensure all template files are in the `templates/` directory at the crate root.

**Risk 3: Missing DB queries**

Several new DB methods are required that do not yet exist on the `Repository` trait:
- `get_alerts_filtered(...)` — for the alerts page filter
- `get_regime_history(limit)` — for the regime timeline
- `get_screen_dates(limit)` — list of distinct screen run dates for the date picker
- `get_ticker_score_history(ticker, limit)` — for watchlist sparklines and stock detail

These need to be added to both the trait definition and the `SqliteRepository` implementation. Each is a straightforward SQL query (no schema changes needed for the first three; `get_ticker_score_history` queries the existing `screen_results` table filtered by ticker). The fourth query is essentially `SELECT run_date, composite_score FROM screen_results WHERE ticker = ? ORDER BY run_date DESC LIMIT ?`.

**Risk 4: Backtest report JSON deserialization**

The `BacktestRunEntry.report_json` field is stored as a raw JSON string. To render it via `/backtest/:id`, the handler must deserialise it to a `BacktestReport` struct and call `generate_html_report()`. If the JSON schema has changed between backtest runs (e.g. a field was added), old rows may fail to deserialise. Use `#[serde(default)]` on all `BacktestReport` fields to handle this gracefully.

**Risk 5: Screen run triggered from UI while another is in progress**

If the user clicks "Run Screen Now" twice, two screen runs start concurrently and both try to write to SQLite. The `Mutex` ensures serialised writes but the runs' progress streams will interleave confusingly. Mitigation: use an `AtomicBool` in `AppState` as a `screen_running` flag. The action handler checks this flag and returns a `409 Conflict` with a fragment saying "A screen run is already in progress" if it is set.

---

## 9. Testing Strategy

The existing tests in `src/web/routes.rs` use `tower::ServiceExt::oneshot()` to send requests to the axum router and assert on status codes and body content. This pattern is the right one and should be extended, not replaced.

New tests to add:

- Every partial endpoint returns HTTP 200 with content that does not contain `<!DOCTYPE html>` — this enforces the partial/full-page contract.
- The static asset handler returns 200 for `htmx.min.js`, 200 for `pico.min.css`, and 404 for a nonexistent path.
- The watchlist add/remove partial handlers return a `<tbody>` fragment (not full HTML) — assert `!body.contains("<html")`.
- The export endpoints: seeded DB → GET `/api/export/csv` → response `Content-Type: text/csv` with NVDA in the body.
- The settings page renders the masked API key, not the real one — assert that the response does not contain the full key string, only the masked version.
- The stock detail page for an unknown ticker returns a 404 with a human-readable error page, not a 500.

---

## 10. Sequencing (Recommended Build Order)

Given the constraint of not breaking existing tests while making incremental progress:

1. **Add rust-embed and the static asset route** — zero risk, purely additive. Removes the CDN dependency immediately.

2. **Add `ghost-alpha.css` with CSS variables** — extract all inline badge styles to the external file. Existing tests still pass.

3. **Add missing DB trait methods** — `get_regime_history`, `get_screen_dates`, `get_ticker_score_history`, `get_alerts_filtered`. Add tests for each. No existing code changes.

4. **Add `src/web/charts.rs`** — pure functions, zero side effects. Test with `assert!(svg.starts_with("<svg"))`.

5. **Migrate to askama templates** — convert one page at a time (start with regime page, the simplest). Keep the existing `format!()` handler and the template handler behind the same route. Replace once the template output matches the format! output in tests.

6. **Fix the htmx partial targets** — change `watchlist_add`/`watchlist_remove` to return only the `<tbody>` fragment and update the form's `hx-target` to `#watchlist-tbody`. This is a breaking change to the UI behaviour but the existing tests only check status code and that the ticker appears in the body — both still pass.

7. **Add missing pages** — `/stock/:ticker`, `/flagged`, `/backtest`, `/strategies`, `/settings`. Each is self-contained.

8. **Add the screen run action and progress feedback** — the highest-risk piece, leave for last.
