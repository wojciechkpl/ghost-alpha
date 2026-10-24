# Ghost Alpha Web Dashboard — Comprehensive Test Plan

**Status:** Research / Planning
**Covers:** `src/web/` redesign as specified in `docs/planning/11-web-dashboard-design.md`
**Baseline:** Existing tests in `src/web/routes.rs` (11 inline tests, all passing)

---

## 0. Guiding Principles

The existing web tests in `src/web/routes.rs` establish the correct pattern for this codebase:

- Use `tower::ServiceExt::oneshot()` against the real Axum router.
- Use `SqliteRepository::open_in_memory()` for database isolation — never touch a real file.
- Seed data directly through the `Repository` trait before dispatching the request.
- Assert on HTTP status code, specific body substrings, and CSS class names rather than full HTML comparison.
- Async tests use `#[tokio::test]`, synchronous pure-logic tests use plain `#[test]`.
- Test names follow `snake_case` and describe the expected behaviour: `home_with_seeded_screen_results`, not `test_home`.

The design doc (section 2.3) reorganises the web module into five source files (`routes.rs`, `partials.rs`, `actions.rs`, `assets.rs`, `charts.rs`) and a `filters.rs` module. Each file gets its own `#[cfg(test)]` block covering the logic it owns. Cross-module integration tests live in `tests/integration/web_test.rs` and are gated behind `--features integration`.

---

## 1. Test Infrastructure

### 1.1 Shared Helpers

**File:** `src/web/mod.rs` (already contains `make_state()`)

The existing `make_state()` helper is sufficient for empty-DB tests. Add two more helpers alongside it in the `#[cfg(test)]` block:

```
make_state_with_seed(seed_fn: impl FnOnce(&SqliteRepository))
  — opens in-memory DB, calls seed_fn, returns AppState

make_post_request(path, body_str)
  — constructs an application/x-www-form-urlencoded POST request
```

`make_state_with_seed` eliminates the boilerplate of cloning `Arc` and calling `repo.*` that currently appears in every seeded test. All seeded tests in `routes.rs` should be refactored to use it.

### 1.2 Test Fixture Additions

**File:** `tests/common/fixtures.rs`

Add web-specific fixtures alongside the existing candle builders:

```
seed_screen_result(repo, ticker, date, score) -> ScreenResult
  — inserts a minimal ScreenResult row; all fields except ticker/date/score use
    sensible defaults (Grade::B, MarketRegime::Bull, etc.)

seed_alert(repo, ticker, severity) -> ()
  — inserts one OutboundAlert row via repo.record_alert()

seed_watchlist(repo, tickers: &[&str]) -> ()
  — inserts multiple watchlist entries in one call

seed_regime(repo, regime: MarketRegime) -> ()
  — inserts a RegimeState for today with plausible SPY/SMA200 values

seed_flagged_stock(repo, ticker) -> FlaggedStock
  — inserts a FlaggedStock with FlaggedStatus::Active and plausible prices
```

These are used by both unit tests (via `make_state_with_seed`) and integration tests.

### 1.3 Request Helpers

**File:** `src/web/mod.rs` `#[cfg(test)]` block

```rust
// Already exists:
async fn get_body(path: &str) -> (StatusCode, String)

// Add:
async fn get_body_seeded(
    seed_fn: impl FnOnce(&SqliteRepository),
    path: &str,
) -> (StatusCode, String)

async fn post_form_seeded(
    seed_fn: impl FnOnce(&SqliteRepository),
    path: &str,
    body: &str,
) -> (StatusCode, String)

// Returns the raw Response (needed for header inspection):
async fn get_response(path: &str) -> axum::response::Response
async fn get_response_seeded(
    seed_fn: impl FnOnce(&SqliteRepository),
    path: &str,
) -> axum::response::Response
```

---

## 2. Unit Tests — `src/web/routes.rs`

These tests exercise full-page GET handlers with the router's `oneshot()` pattern.

### 2.1 Home Page (`GET /`)

| Test name | Setup | Assertions | Priority |
|---|---|---|---|
| `home_returns_200_with_html` | empty DB | status 200, body contains `<!DOCTYPE html>`, `Ghost Alpha`, `Screening Results` | P0 (exists) |
| `home_empty_db_shows_empty_state` | empty DB | body contains `empty-state` class, does not contain `<td><strong>` (no data rows) | P0 |
| `home_with_seeded_screen_results` | one ScreenResult for today | body contains seeded ticker, composite score formatted to 1dp | P0 (exists) |
| `home_shows_regime_badge_bull` | RegimeState(Bull) seeded | body contains `badge-bull` | P1 |
| `home_shows_regime_badge_bear` | RegimeState(Bear) seeded | body contains `badge-bear` | P1 |
| `home_shows_regime_badge_unknown_when_no_regime` | empty DB | body contains `badge-sideways` and `Unknown` | P1 |
| `home_shows_all_12_table_columns` | one ScreenResult | body contains `<th>Grade</th>`, `<th>ADX</th>`, `<th>Price</th>`, `<th>% from High</th>` | P1 |
| `home_shows_vcp_badge_when_vcp_detected` | ScreenResult with `vcp: Some(...)` | body contains `VCP` text | P2 |
| `home_with_grade_a_plus_shows_correct_badge` | ScreenResult with Grade::APlus | body contains `badge-grade-aplus` | P1 |
| `home_ticker_with_html_special_chars_is_escaped` | ScreenResult with ticker `"A&B"` | body contains `A&amp;B`, does not contain raw `A&B` | P0 |

**Notes:**
- `home_shows_all_12_table_columns` is a new test that will fail against the current 7-column implementation. It should be added as a known-failing test (with `#[ignore]`) until the columns are expanded, documenting the gap.
- The `home_ticker_with_html_special_chars_is_escaped` test is important: the current code does not escape ticker symbols in the table rows. This is a bug the test will catch.

### 2.2 Alerts Page (`GET /alerts`)

| Test name | Setup | Assertions | Priority |
|---|---|---|---|
| `alerts_page_returns_200` | empty DB | status 200, body contains `Alert History` | P0 (exists) |
| `alerts_empty_db_shows_empty_state` | empty DB | body contains `empty-state` | P0 |
| `alerts_page_with_seeded_alerts` | one Critical alert for MSFT | body contains `badge-critical`, `MSFT` | P0 (exists) |
| `alerts_page_critical_badge_class` | Critical severity | body contains `badge-critical`, not `badge-warning` | P0 |
| `alerts_page_warning_badge_class` | Warning severity | body contains `badge-warning` | P0 |
| `alerts_page_info_badge_class` | Info severity | body contains `badge-info` | P0 |
| `alerts_page_message_html_escaped` | alert message `<script>alert(1)</script>` | body contains `&lt;script&gt;`, does not contain `<script>` raw | P0 |
| `alerts_page_shows_50_alerts_max` | 60 alerts seeded | body does not show more than 50 rows (count `<tr>` in tbody <= 51 including empty-state check) | P1 |

**Notes:**
- The XSS escape test for alert messages is critical. The current code calls `html_escape(&a.message)` correctly, but the test documents and enforces this.

### 2.3 Watchlist Page (`GET /watchlist`)

| Test name | Setup | Assertions | Priority |
|---|---|---|---|
| `watchlist_page_returns_200` | empty DB | status 200, body contains `Watchlist` | P0 (exists) |
| `watchlist_empty_db_shows_empty_state` | empty DB | body contains `empty-state` | P0 |
| `watchlist_page_shows_ticker_in_table` | one entry for NVDA | body contains `NVDA` | P0 |
| `watchlist_shows_score_when_available` | entry with `last_score: Some(87.3)` | body contains `87.3` | P1 |
| `watchlist_shows_dash_when_no_score` | entry with `last_score: None` | body contains `—` | P1 |
| `watchlist_add_form_is_present` | empty DB | body contains `hx-post="/watchlist/add"` or `hx-post="/api/watchlist/add"` (whichever route is used) | P0 |
| `watchlist_notes_html_escaped` | entry with notes `"<b>important</b>"` | body contains `&lt;b&gt;`, not `<b>` raw | P0 |

### 2.4 Regime Page (`GET /regime`)

| Test name | Setup | Assertions | Priority |
|---|---|---|---|
| `regime_page_returns_200` | empty DB | status 200, body contains `Market Regime` | P0 (exists) |
| `regime_page_empty_shows_no_data_message` | empty DB | body contains `empty-state` | P0 |
| `regime_page_with_bull_regime` | RegimeState(Bull) | body contains `badge-bull`, `Bull` | P0 (exists) |
| `regime_page_with_bear_regime` | RegimeState(Bear) | body contains `badge-bear`, `Bear` | P0 |
| `regime_page_with_sideways_regime` | RegimeState(Sideways) | body contains `badge-sideways`, `Sideways` | P0 |
| `regime_page_shows_spy_price` | RegimeState(spy_close=520.0) | body contains `520.00` | P1 |
| `regime_page_with_macro_risk_shows_vix` | MacroRisk(vix=18.5) | body contains `18.5` | P1 |
| `regime_page_no_macro_risk_shows_placeholder` | no macro risk | body contains `No macro risk data` | P1 |

### 2.5 Stock Detail Page (`GET /stock/:ticker`) — NEW

| Test name | Setup | Assertions | Priority |
|---|---|---|---|
| `stock_detail_returns_200_for_known_ticker` | candles + screen result for AAPL | status 200, body contains `AAPL` | P0 |
| `stock_detail_returns_404_for_unknown_ticker` | empty DB | status 404, body contains human-readable error (not a panic trace) | P0 |
| `stock_detail_shows_score_breakdown` | ScreenResult seeded | body contains `Composite`, `Technical`, `Volume` | P1 |
| `stock_detail_shows_minervini_checklist` | ScreenResult seeded | body contains all 8 condition descriptions | P1 |
| `stock_detail_back_link_is_present` | any seeded state | body contains `href="/"` back-to-results link | P2 |
| `stock_detail_watchlist_indicator_when_on_watchlist` | watchlist entry for ticker | body contains watchlist indicator element | P2 |
| `stock_detail_watchlist_indicator_when_not_on_watchlist` | no watchlist entry | body contains add-to-watchlist link | P2 |

### 2.6 Flagged Stocks Page (`GET /flagged`) — NEW

| Test name | Setup | Assertions | Priority |
|---|---|---|---|
| `flagged_page_returns_200` | empty DB | status 200, body contains `Flagged Stocks` | P0 |
| `flagged_page_empty_shows_message` | empty DB | body contains `empty-state` or `no active` | P0 |
| `flagged_page_shows_active_stocks` | one Active FlaggedStock | body contains ticker, `ACTIVE` label | P0 |
| `flagged_page_conviction_decay_positive` | FlaggedStock where latest > entry by 5% | body contains conviction-improving badge class | P1 |
| `flagged_page_conviction_decay_negative` | FlaggedStock where latest < entry by 18% | body contains conviction-declining badge class | P1 |
| `flagged_page_shows_exited_section` | one Exited FlaggedStock | body contains `Exited` section heading | P1 |
| `flagged_page_exit_reason_escaped` | exit_reason contains `<script>` | body does not contain raw `<script>` | P0 |

### 2.7 Backtest List Page (`GET /backtest`) — NEW

| Test name | Setup | Assertions | Priority |
|---|---|---|---|
| `backtest_page_returns_200` | empty DB | status 200, body contains `Backtest Results` | P0 |
| `backtest_page_empty_shows_message` | empty DB | body contains `empty-state` | P0 |
| `backtest_page_shows_run_list` | one BacktestRunEntry seeded | body contains run date, signal count | P1 |
| `backtest_detail_returns_200` | BacktestRunEntry with valid report_json | status 200, content-type `text/html` | P0 |
| `backtest_detail_returns_404_for_missing_id` | empty DB | status 404 | P0 |

### 2.8 Strategies Page (`GET /strategies`) — NEW

| Test name | Setup | Assertions | Priority |
|---|---|---|---|
| `strategies_page_returns_200` | no DB needed (static data) | status 200, body contains `Minervini`, `Breakout` | P0 |
| `strategies_page_shows_all_four_strategies` | no DB needed | body contains `Mean Reversion`, `Trend Strength` | P0 |
| `strategies_page_shows_weight_percentages` | no DB needed | body contains `55%`, `30%` (Minervini weights) | P1 |

### 2.9 Settings Page (`GET /settings`) — NEW

| Test name | Setup | Assertions | Priority |
|---|---|---|---|
| `settings_page_returns_200` | default AppConfig | status 200, body contains `Settings` | P0 |
| `settings_page_masks_api_key` | AppConfig with Finnhub key `"ghst_abc123xyz"` | body does NOT contain `ghst_abc123xyz`, DOES contain `****` | P0 |
| `settings_page_shows_alert_thresholds` | AppConfig with alerts.min_score_critical=90.0 | body contains `90` | P1 |
| `settings_page_shows_universe_count` | config.universe with 5 tickers | body contains `5 tickers` | P1 |
| `settings_page_shows_telegram_configured` | config.telegram = Some(...) | body contains `configured` | P2 |
| `settings_page_shows_discord_not_configured` | config.discord = None | body contains `not configured` | P2 |

---

## 3. Unit Tests — `src/web/charts.rs`

The SVG chart functions are pure: they take data slices and return `String`. No DB, no HTTP. These are the fastest tests in the web suite.

**Conventions:** Plain `#[test]`, Arrange-Act-Assert. Assert on the returned `String` using `starts_with`, `contains`, and XML parse validity.

### 3.1 Price Chart SVG (`price_chart_svg`)

| Test name | Input | Assertions | Priority |
|---|---|---|---|
| `price_chart_svg_starts_with_svg_tag` | 252 trending candles | return value starts with `<svg` | P0 |
| `price_chart_svg_contains_viewbox` | 252 candles | return value contains `viewBox="0 0 800 300"` (or design-specified dimensions) | P0 |
| `price_chart_svg_contains_close_polyline` | 252 candles | return value contains `<polyline` | P0 |
| `price_chart_svg_contains_volume_rects` | 252 candles | return value contains `<rect` | P0 |
| `price_chart_svg_valid_xml` | 252 candles | parseable by a minimal XML check: no unclosed tags; every `<` has a matching `>` | P0 |
| `price_chart_svg_single_candle_does_not_panic` | 1 candle | does not panic, returns non-empty string | P0 |
| `price_chart_svg_empty_candles_returns_placeholder` | empty slice | returns a `<svg` string (placeholder), does not panic | P0 |
| `price_chart_svg_contains_sma50_line` | 252 candles | return value contains at least 2 `<polyline` elements (price + MA lines) | P1 |
| `price_chart_svg_contains_52w_high_line` | 252 candles | return value contains `<line` (the 52-week high marker) | P1 |
| `price_chart_svg_no_nan_coordinates` | 252 uniform-price candles (zero variance) | return value does not contain `NaN` | P0 |
| `price_chart_svg_no_inf_coordinates` | candles with extreme price values (1e15) | return value does not contain `inf` or `Inf` | P0 |
| `price_chart_svg_zero_volume_does_not_panic` | candles with volume=0 | does not panic | P0 |

### 3.2 Sparkline SVG (`sparkline_svg`)

| Test name | Input | Assertions | Priority |
|---|---|---|---|
| `sparkline_svg_starts_with_svg_tag` | 10 score values | return value starts with `<svg` | P0 |
| `sparkline_svg_single_value_does_not_panic` | vec![75.0] | does not panic | P0 |
| `sparkline_svg_empty_values_returns_placeholder` | empty vec | does not panic, returns non-empty string | P0 |
| `sparkline_svg_improving_series_uses_green_color` | [40.0, 50.0, 60.0, 70.0] | return value contains `#51cf66` (or CSS var for improving) | P1 |
| `sparkline_svg_declining_series_uses_red_color` | [70.0, 60.0, 50.0, 40.0] | return value contains `#ff6b6b` (or CSS var for declining) | P1 |
| `sparkline_svg_constant_series_does_not_produce_nan` | [60.0; 10] | return value does not contain `NaN` | P0 |

### 3.3 Regime Timeline SVG (`regime_timeline_svg`)

| Test name | Input | Assertions | Priority |
|---|---|---|---|
| `regime_timeline_svg_starts_with_svg_tag` | vec of 252 RegimeState entries | starts with `<svg` | P0 |
| `regime_timeline_svg_contains_three_segment_colors` | history with Bull + Bear + Sideways runs | contains `#51cf66`, `#ff6b6b`, `#ffd43b` | P1 |
| `regime_timeline_svg_single_regime_does_not_panic` | single RegimeState | does not panic | P0 |
| `regime_timeline_svg_empty_history_returns_placeholder` | empty vec | does not panic | P0 |
| `regime_timeline_svg_valid_xml` | 12 regime rows | no unclosed tags | P0 |

### 3.4 Score Histogram SVG (`score_histogram_svg`)

| Test name | Input | Assertions | Priority |
|---|---|---|---|
| `score_histogram_svg_starts_with_svg_tag` | 50 scores | starts with `<svg` | P0 |
| `score_histogram_svg_empty_input_does_not_panic` | empty vec | does not panic | P0 |
| `score_histogram_svg_contains_rect_elements` | 10 scores | contains `<rect` | P0 |
| `score_histogram_svg_all_identical_scores_does_not_panic` | [75.0; 50] | does not panic, valid SVG | P0 |

---

## 4. Unit Tests — `src/web/partials.rs`

Partials are tested with the same `oneshot()` pattern as full-page routes. The defining invariant is that partial responses must never contain `<!DOCTYPE html>` and must never contain `<html`.

**Shared assertion helper to add:**

```rust
fn assert_is_fragment(body: &str) {
    assert!(!body.contains("<!DOCTYPE"), "partial must not emit DOCTYPE");
    assert!(!body.contains("<html"),    "partial must not emit <html>");
    assert!(!body.contains("<head"),    "partial must not emit <head>");
}
```

### 4.1 Screen Table Partial (`GET /partials/screen-table`)

| Test name | Setup | Assertions | Priority |
|---|---|---|---|
| `screen_table_partial_returns_200` | empty DB | status 200 | P0 |
| `screen_table_partial_is_fragment` | empty DB | `assert_is_fragment(&body)` | P0 |
| `screen_table_partial_empty_db_shows_empty_row` | empty DB | body contains `empty-state` | P0 |
| `screen_table_partial_shows_seeded_ticker` | one ScreenResult for today | body contains ticker | P0 |
| `screen_table_partial_date_filter_returns_correct_date` | two ScreenResults on different dates | GET with `?date=2026-01-15` returns only date-matched row | P0 |
| `screen_table_partial_min_score_filter` | results with scores 40, 60, 80 | GET with `?min_score=65` returns only score≥65 rows | P0 |
| `screen_table_partial_minervini_only_filter` | one pass, one fail | GET with `?minervini_only=true` returns only passing row | P0 |
| `screen_table_partial_strategy_filter` | results for different strategies | GET with `?strategy=breakout` returns breakout results only | P1 |
| `screen_table_partial_pagination_page_0` | 25 ScreenResults | GET with `?page=0&per_page=20` returns exactly 20 rows | P0 |
| `screen_table_partial_pagination_page_1` | 25 ScreenResults | GET with `?page=1&per_page=20` returns exactly 5 rows | P0 |
| `screen_table_partial_pagination_beyond_last_page` | 5 ScreenResults | GET with `?page=5&per_page=20` returns empty body with pagination info | P1 |
| `screen_table_partial_pagination_total_count_shown` | 25 ScreenResults | body contains `25` (total count) in pagination footer | P1 |
| `screen_table_partial_invalid_page_param_defaults_to_0` | 5 ScreenResults | GET with `?page=abc` returns 200, same as `?page=0` | P1 |
| `screen_table_partial_invalid_min_score_param_ignored` | 5 ScreenResults | GET with `?min_score=not_a_number` returns 200 with all rows | P1 |

### 4.2 Alert List Partial (`GET /partials/alert-list`)

| Test name | Setup | Assertions | Priority |
|---|---|---|---|
| `alert_list_partial_returns_200` | empty DB | status 200 | P0 |
| `alert_list_partial_is_fragment` | empty DB | `assert_is_fragment(&body)` | P0 |
| `alert_list_partial_severity_filter_critical` | Critical + Warning alerts | GET `?severity=CRITICAL` returns only Critical badge | P0 |
| `alert_list_partial_severity_filter_all` | Critical + Warning alerts | GET `?severity=ALL` returns both | P0 |
| `alert_list_partial_ticker_filter` | alerts for NVDA and AAPL | GET `?ticker=NVDA` returns only NVDA row | P0 |
| `alert_list_partial_ticker_filter_partial_match` | ticker "NVDA" | GET `?ticker=NV` returns NVDA row (LIKE match) | P1 |
| `alert_list_partial_pagination_offsets` | 30 alerts | GET `?page=1&per_page=20` returns rows 21–30 | P0 |

### 4.3 Watchlist Rows Partial (`GET /partials/watchlist-rows`)

| Test name | Setup | Assertions | Priority |
|---|---|---|---|
| `watchlist_rows_partial_returns_200` | empty DB | status 200 | P0 |
| `watchlist_rows_partial_is_fragment` | empty DB | `assert_is_fragment(&body)` | P0 |
| `watchlist_rows_partial_shows_entries` | two watchlist entries | body contains both tickers | P0 |
| `watchlist_rows_partial_contains_remove_button` | one entry | body contains `hx-post` remove trigger | P0 |

### 4.4 Flagged Rows Partial (`GET /partials/flagged-rows`)

| Test name | Setup | Assertions | Priority |
|---|---|---|---|
| `flagged_rows_partial_returns_200` | empty DB | status 200 | P0 |
| `flagged_rows_partial_is_fragment` | empty DB | `assert_is_fragment(&body)` | P0 |
| `flagged_rows_partial_shows_active_stocks` | one Active FlaggedStock | body contains ticker | P0 |

### 4.5 Stock Candles Partial (`GET /partials/stock-candles/:ticker`)

| Test name | Setup | Assertions | Priority |
|---|---|---|---|
| `stock_candles_partial_returns_200` | 252 candles seeded for NVDA | status 200 | P0 |
| `stock_candles_partial_is_svg_fragment` | 252 candles seeded | `assert_is_fragment(&body)`, body contains `<svg` | P0 |
| `stock_candles_partial_returns_404_for_unknown_ticker` | empty DB | status 404 | P0 |
| `stock_candles_partial_returns_placeholder_for_insufficient_data` | fewer than 50 candles | status 200, body contains placeholder text | P1 |

### 4.6 Regime Chart Partial (`GET /partials/regime-chart`)

| Test name | Setup | Assertions | Priority |
|---|---|---|---|
| `regime_chart_partial_returns_200` | regime history seeded | status 200 | P0 |
| `regime_chart_partial_is_fragment` | regime history seeded | `assert_is_fragment(&body)` | P0 |
| `regime_chart_partial_empty_db_returns_placeholder` | empty DB | status 200, body does not panic | P0 |

### 4.7 Nav Badge Partial (`GET /partials/nav-alerts-count`)

| Test name | Setup | Assertions | Priority |
|---|---|---|---|
| `nav_alerts_count_partial_returns_200` | empty DB | status 200 | P0 |
| `nav_alerts_count_partial_is_fragment` | empty DB | `assert_is_fragment(&body)` | P0 |
| `nav_alerts_count_shows_zero` | empty DB | body contains `0` | P1 |
| `nav_alerts_count_shows_correct_count` | 7 alerts seeded | body contains `7` | P0 |

---

## 5. Unit Tests — `src/web/actions.rs`

Action handlers perform mutations and return either an updated fragment (watchlist) or a status response.

### 5.1 Watchlist Mutations

| Test name | Setup | Request | Assertions | Priority |
|---|---|---|---|---|
| `watchlist_add_post_persists_and_renders` | empty DB | POST `/api/watchlist/add` body=`ticker=NVDA&notes=` | status 200, body contains `NVDA` | P0 (exists, new path) |
| `watchlist_add_post_response_is_fragment` | empty DB | POST add | `assert_is_fragment(&body)` | P0 |
| `watchlist_add_empty_ticker_is_noop` | empty DB | POST body=`ticker=&notes=` | watchlist_list() returns empty | P0 (exists) |
| `watchlist_add_trims_whitespace` | empty DB | POST body=`ticker=+NVDA+` | DB entry has ticker `NVDA` (no spaces) | P0 |
| `watchlist_add_uppercases_ticker` | empty DB | POST body=`ticker=nvda` | body contains `NVDA`, DB entry has `NVDA` | P0 |
| `watchlist_add_with_notes_persists_notes` | empty DB | POST body=`ticker=NVDA&notes=Strong+setup` | DB entry has notes `"Strong setup"` | P1 |
| `watchlist_add_duplicate_ticker_is_idempotent` | NVDA already on watchlist | POST body=`ticker=NVDA` | watchlist_list() returns exactly one NVDA entry | P1 |
| `watchlist_remove_post_removes_ticker` | TSLA seeded | POST `/api/watchlist/remove` body=`ticker=TSLA` | status 200, TSLA absent from watchlist_list() | P0 (exists) |
| `watchlist_remove_post_response_is_fragment` | any state | POST remove | `assert_is_fragment(&body)` | P0 |
| `watchlist_remove_nonexistent_ticker_is_noop` | empty DB | POST body=`ticker=AAPL` | status 200, no panic | P0 |
| `watchlist_remove_response_updates_table` | two entries: NVDA, TSLA; remove TSLA | body contains `NVDA`, does not contain `TSLA` | P0 |

### 5.2 Export Endpoints

| Test name | Setup | Request | Assertions | Priority |
|---|---|---|---|---|
| `export_csv_returns_200` | one ScreenResult seeded for today | GET `/api/export/csv` | status 200 | P0 |
| `export_csv_content_type_is_text_csv` | one ScreenResult | GET `/api/export/csv` | `Content-Type` header contains `text/csv` | P0 |
| `export_csv_contains_header_row` | one ScreenResult | GET `/api/export/csv` | body starts with `ticker,` (CSV header) | P0 |
| `export_csv_contains_seeded_ticker` | ScreenResult for NVDA | GET `/api/export/csv` | body contains `NVDA` | P0 |
| `export_csv_empty_db_returns_header_only` | empty DB | GET `/api/export/csv` | body starts with `ticker,`, has exactly 1 line | P1 |
| `export_csv_content_disposition_attachment` | any state | GET `/api/export/csv` | `Content-Disposition` header contains `attachment` | P1 |
| `export_json_returns_200` | one ScreenResult | GET `/api/export/json` | status 200 | P0 |
| `export_json_content_type_is_application_json` | one ScreenResult | GET `/api/export/json` | `Content-Type` contains `application/json` | P0 |
| `export_json_body_is_valid_json_array` | one ScreenResult for NVDA | GET `/api/export/json` | body starts with `[`, parses as JSON array | P0 |
| `export_json_contains_ticker_field` | ScreenResult for NVDA | GET `/api/export/json` | body contains `"ticker":"NVDA"` | P0 |
| `export_json_date_filter_param` | results on two different dates | GET `/api/export/json?date=2026-01-15` | body contains only the matching date's results | P1 |

### 5.3 Screen Run Trigger

| Test name | Setup | Request | Assertions | Priority |
|---|---|---|---|---|
| `screen_run_post_returns_accepted` | default AppState | POST `/api/screen/run` | status 202 Accepted | P1 |
| `screen_run_post_response_contains_job_id` | default AppState | POST `/api/screen/run` | body contains a job ID fragment (for SSE polling) | P1 |
| `screen_run_post_when_already_running_returns_409` | `screen_running` flag = true | POST `/api/screen/run` | status 409 Conflict, body explains run in progress | P1 |

---

## 6. Unit Tests — `src/web/assets.rs`

Static asset serving requires `rust-embed`. Tests check the embedded file handler directly.

| Test name | Assertions | Priority |
|---|---|---|
| `static_asset_htmx_js_returns_200` | GET `/static/htmx.min.js` → status 200 | P0 |
| `static_asset_pico_css_returns_200` | GET `/static/pico.min.css` → status 200 | P0 |
| `static_asset_ghost_alpha_css_returns_200` | GET `/static/ghost-alpha.css` → status 200 | P0 |
| `static_asset_htmx_js_content_type` | `Content-Type` header contains `application/javascript` or `text/javascript` | P0 |
| `static_asset_pico_css_content_type` | `Content-Type` header contains `text/css` | P0 |
| `static_asset_nonexistent_file_returns_404` | GET `/static/does-not-exist.xyz` → status 404 | P0 |
| `static_asset_cache_control_header_set` | GET any valid asset → `Cache-Control` header contains `max-age=` | P1 |
| `static_asset_svg_favicon_content_type` | GET `/static/favicon.svg` (if present) → `Content-Type` contains `image/svg+xml` | P2 |
| `static_asset_htmx_ext_sse_js_returns_200` | GET `/static/htmx-ext-sse.js` → status 200 | P1 |

**Note:** These tests require the `web/static/` directory and embedded files to exist. They will fail until step 1 of the build sequence (asset embedding) is implemented. Add them as `#[ignore]` tests immediately after writing, then remove `#[ignore]` when the feature lands.

---

## 7. Unit Tests — `src/web/filters.rs` (Askama Custom Filters)

These are pure functions — plain `#[test]`, no HTTP, no DB.

| Test name | Input | Expected output | Priority |
|---|---|---|---|
| `format_grade_badge_a_plus` | `Grade::APlus` | string contains `badge-grade-aplus` and `A+` | P0 |
| `format_grade_badge_a` | `Grade::A` | contains `badge-grade-a` and `A` | P0 |
| `format_grade_badge_b` | `Grade::B` | contains `badge-grade-b` and `B` | P0 |
| `format_grade_badge_c` | `Grade::C` | contains `badge-grade-c` and `C` | P0 |
| `format_grade_badge_d` | `Grade::D` | contains `badge-grade-d` and `D` | P0 |
| `format_grade_badge_f` | `Grade::F` | contains `badge-grade-f` and `F` | P0 |
| `format_severity_badge_critical` | `"CRITICAL"` | contains `badge-critical` and `CRITICAL` | P0 |
| `format_severity_badge_warning` | `"WARNING"` | contains `badge-warning` | P0 |
| `format_severity_badge_info` | `"INFO"` | contains `badge-info` | P0 |
| `format_severity_badge_unknown` | `"UNKNOWN"` | falls back to info badge, no panic | P1 |
| `format_f64_1dp_basic` | `87.349` | `"87.3"` | P0 |
| `format_f64_1dp_integer_value` | `80.0` | `"80.0"` | P0 |
| `format_f64_1dp_negative` | `-5.0` | `"-5.0"` | P0 |
| `format_f64_1dp_nan` | `f64::NAN` | `"—"` or `"N/A"` (graceful fallback) | P0 |
| `format_f64_1dp_infinity` | `f64::INFINITY` | `"—"` or `"N/A"` (graceful fallback) | P0 |
| `format_conviction_decay_badge_improving` | decay = +8.0 | contains improving badge class | P1 |
| `format_conviction_decay_badge_stable` | decay = +2.0 | contains stable badge class | P1 |
| `format_conviction_decay_badge_declining` | decay = -10.0 | contains declining badge class | P1 |
| `format_conviction_decay_badge_critical` | decay = -20.0 | contains critical-decay badge class | P1 |
| `html_escape_ampersand` | `"AT&T"` | `"AT&amp;T"` | P0 |
| `html_escape_less_than` | `"a<b"` | `"a&lt;b"` | P0 |
| `html_escape_greater_than` | `"a>b"` | `"a&gt;b"` | P0 |
| `html_escape_double_quote` | `r#"say "hi""#` | `"say &quot;hi&quot;"` | P0 |
| `html_escape_single_quote` | `"it's"` | `"it&#39;s"` | P0 |
| `html_escape_no_special_chars` | `"NVDA"` | `"NVDA"` (unchanged) | P0 |
| `html_escape_empty_string` | `""` | `""` | P0 |

---

## 8. Integration Tests — `tests/integration/web_test.rs`

These tests require `--features integration` and use a fully wired router. They are the authoritative end-to-end smoke tests for the web layer.

**Entry point addition to `tests/integration.rs`:**

```rust
#[path = "integration/web_test.rs"]
mod web_test;
```

### 8.1 Router Smoke Tests

| Test name | Setup | Assertions | Priority |
|---|---|---|---|
| `all_full_page_routes_return_200` | empty DB | GET every full-page route: `/`, `/alerts`, `/watchlist`, `/regime`, `/flagged`, `/backtest`, `/strategies`, `/settings`; all return 200 | P0 |
| `all_partial_routes_return_200` | minimal seeded state | GET every partial route; all return 200 | P0 |
| `all_partial_routes_are_fragments` | minimal seeded state | none of the partial response bodies contain `<!DOCTYPE html>` | P0 |
| `unknown_route_returns_404` | empty DB | GET `/does-not-exist` → status 404 | P0 |

**Note:** `all_full_page_routes_return_200` is the single most important test in the integration suite. If it passes, the router is wired correctly. If any route panics, returns 500, or is missing from the router, it will catch it.

### 8.2 Watchlist Round-Trip

| Test name | Setup | Sequence | Assertions | Priority |
|---|---|---|---|---|
| `watchlist_add_then_list_shows_ticker` | empty DB | POST add NVDA → GET `/watchlist` | GET response contains NVDA | P0 |
| `watchlist_add_then_remove_clears_ticker` | empty DB | POST add NVDA → POST remove NVDA → GET `/watchlist` | GET response does not contain NVDA in table data | P0 |
| `watchlist_add_multiple_tickers` | empty DB | POST add NVDA, AAPL, MSFT → GET `/partials/watchlist-rows` | body contains all three tickers | P0 |
| `watchlist_remove_response_is_updated_fragment` | NVDA + TSLA seeded | POST remove TSLA | response body contains NVDA, does not contain TSLA | P0 |

### 8.3 Filtered Data Flows

| Test name | Setup | Assertions | Priority |
|---|---|---|---|
| `screen_table_date_filter_round_trip` | results on 2026-01-10 and 2026-01-15 | GET `/partials/screen-table?date=2026-01-10` contains only Jan-10 tickers | P0 |
| `alert_list_severity_filter_round_trip` | Critical alert for NVDA, Warning for AAPL | GET `/partials/alert-list?severity=CRITICAL` contains NVDA, not AAPL | P0 |
| `alert_list_ticker_filter_round_trip` | alerts for NVDA, AAPL, MSFT | GET `/partials/alert-list?ticker=AAPL` contains only AAPL | P0 |

### 8.4 Export Round-Trips

| Test name | Setup | Assertions | Priority |
|---|---|---|---|
| `csv_export_contains_all_seeded_tickers` | NVDA + AAPL + MSFT screen results | GET `/api/export/csv` body contains all three tickers | P0 |
| `json_export_parses_as_valid_json` | 5 ScreenResults seeded | GET `/api/export/json` body is valid JSON array of length 5 | P0 |

### 8.5 Error Handling

| Test name | Setup | Assertions | Priority |
|---|---|---|---|
| `stock_detail_404_for_unknown_ticker` | empty DB | GET `/stock/ZZZZ` → status 404, body is human-readable HTML, not a Rust panic message | P0 |
| `backtest_detail_404_for_missing_id` | empty DB | GET `/backtest/999` → status 404 | P0 |
| `stock_candles_404_for_unknown_ticker` | empty DB | GET `/partials/stock-candles/ZZZZ` → status 404 | P0 |

---

## 9. Property-Based Tests

Property tests verify invariants that must hold across all possible inputs. Use the `proptest` crate. These live in the `#[cfg(test)]` blocks of the modules they test, with the `proptest!` macro.

**Cargo.toml addition required:**

```toml
[dev-dependencies]
proptest = "1"
```

### 9.1 SVG Output Validity

**File:** `src/web/charts.rs`

```
proptest! {
    #[test]
    fn price_chart_svg_never_contains_nan_or_inf(
        prices in prop::collection::vec(0.1f64..10000.0f64, 252..=300)
    ) {
        let candles = prices_to_candles(prices);
        let svg = price_chart_svg(&candles);
        assert!(!svg.contains("NaN"), "SVG must not contain NaN");
        assert!(!svg.contains("inf"), "SVG must not contain inf");
        assert!(svg.starts_with("<svg"), "SVG must start with <svg tag");
    }
```

| Property test name | Generator | Invariants checked | Priority |
|---|---|---|---|
| `price_chart_svg_never_contains_nan_or_inf` | vec of 252–300 random positive f64 prices | no `NaN`, no `inf`, starts with `<svg` | P0 |
| `price_chart_svg_always_starts_with_svg` | vec of 1–500 prices, including extreme values | starts with `<svg` | P0 |
| `sparkline_svg_never_panics` | vec of 0–100 random f64 in 0.0..=100.0 | does not panic, returns non-empty string | P0 |
| `sparkline_svg_no_nan_or_inf` | vec of 1–50 scores including constant values | no `NaN`, no `inf` | P0 |
| `regime_timeline_svg_never_panics` | vec of 0–365 regime values (3 variants) | does not panic | P0 |
| `html_escape_is_idempotent` | any ASCII string | `html_escape(html_escape(s)) == html_escape(s)` | P1 |
| `html_escape_output_has_no_bare_less_than` | any ASCII string | output does not contain `<` except as part of `&lt;` | P1 |

### 9.2 Pagination Invariants

**File:** `src/web/partials.rs`

| Property test name | Generator | Invariants checked | Priority |
|---|---|---|---|
| `pagination_row_count_matches_per_page` | total_rows in 0..=200, per_page in 1..=50, page in 0..=20 | rows on page = min(per_page, max(0, total - page*per_page)) | P1 |
| `pagination_no_duplicate_rows_across_pages` | same generator | union of all pages contains no duplicate row IDs | P1 |

---

## 10. Performance Tests

Performance tests use Criterion and live in `benches/`. They are excluded from `cargo test` and run with `cargo bench`.

**File:** `benches/web_bench.rs`

### 10.1 Response Time Benchmarks

| Benchmark name | Setup | Target | Priority |
|---|---|---|---|
| `bench_home_page_empty_db` | empty DB | < 5ms per render (Criterion measures iterations) | P1 |
| `bench_home_page_100_results` | 100 ScreenResults seeded | < 20ms per render | P1 |
| `bench_home_page_1000_results` | 1,000 ScreenResults seeded | < 50ms per render | P1 |
| `bench_home_page_10000_results` | 10,000 ScreenResults seeded | < 200ms per render (large dataset) | P2 |
| `bench_alerts_page_50_alerts` | 50 alerts seeded | < 5ms per render | P1 |
| `bench_screen_table_partial_page_0` | 1,000 ScreenResults, per_page=20 | < 10ms per request | P1 |
| `bench_screen_table_partial_with_filter` | 1,000 results, min_score=70 filter | < 15ms per request | P2 |

### 10.2 SVG Generation Benchmarks

| Benchmark name | Input | Target | Priority |
|---|---|---|---|
| `bench_price_chart_svg_252_candles` | 252 candles | < 1ms | P1 |
| `bench_sparkline_svg_30_points` | 30 score values | < 100µs | P1 |
| `bench_regime_timeline_svg_252_days` | 252 RegimeState entries | < 500µs | P1 |

### 10.3 Concurrent Request Handling

These require a real Axum server binding a port (use `axum::Server::bind` with `0.0.0.0:0` for ephemeral port selection) and are gated behind `--features integration`.

| Test name | Setup | Method | Assertions | Priority |
|---|---|---|---|---|
| `concurrent_50_home_requests` | 50 ScreenResults seeded | spawn 50 tokio tasks each sending GET `/` | all 50 complete with status 200, no deadlock within 5 seconds | P1 |
| `concurrent_home_and_watchlist_add` | empty DB | 25 GET `/` tasks + 25 POST `/api/watchlist/add` tasks | all complete, no DB corruption | P1 |
| `concurrent_home_and_export` | 100 ScreenResults | 10 GET `/` + 10 GET `/api/export/csv` + 10 GET `/api/export/json` | all 30 return 200 | P2 |

**Note:** Concurrency tests exercise the `Mutex<Connection>` bottleneck described in the design doc's Risk 1. They are not regression tests for a bug that exists now — they are forward-looking tests that confirm the single-user concurrency assumption holds and will detect deadlocks if a future change introduces one.

---

## 11. Accessibility Tests

These are semi-automated checks. The HTML is parsed from the response body using a minimal string-search approach. Full WCAG compliance requires a headless browser; these tests cover the most impactful structural requirements that can be verified with string matching.

**File:** `tests/integration/web_accessibility_test.rs`

### 11.1 Semantic HTML Structure

| Test name | Route | Assertions | Priority |
|---|---|---|---|
| `home_table_has_thead_with_th_elements` | GET `/` with seeded data | body contains `<thead>`, body contains `<th>` | P0 |
| `alerts_table_has_thead_with_th_elements` | GET `/alerts` | body contains `<thead>`, `<th>Ticker</th>` | P0 |
| `watchlist_table_has_thead` | GET `/watchlist` | body contains `<thead>` | P0 |
| `all_pages_have_h1` | GET each full-page route | each body contains exactly one `<h1` | P1 |
| `all_pages_have_lang_attribute` | GET each full-page route | each body contains `<html lang="en"` | P0 |
| `all_pages_have_charset_meta` | GET each full-page route | each body contains `charset="UTF-8"` | P0 |
| `all_pages_have_viewport_meta` | GET each full-page route | each body contains `name="viewport"` | P0 |
| `all_interactive_buttons_have_type_attribute` | GET `/watchlist` | every `<button` in body has `type="submit"` or `type="button"` (not implicit) | P1 |
| `remove_buttons_have_hx_confirm_attribute` | GET `/watchlist` with entries | every remove button has `hx-confirm=` to prevent accidental removal | P1 |
| `settings_masked_api_key` | GET `/settings` with configured API key | body does not contain the full raw key, only the masked version | P0 |

### 11.2 Color Contrast (Static Verification)

Full color contrast testing requires a headless browser. These tests verify that the CSS class names being emitted are the ones from `ghost-alpha.css` where the contrast ratios have been designed to meet WCAG 2.1 AA. They do not measure contrast ratios directly.

| Test name | Assertions | Priority |
|---|---|---|
| `severity_badges_use_correct_css_classes` | Critical→`badge-critical`, Warning→`badge-warning`, Info→`badge-info`; none of the classes are ad-hoc inline styles | P1 |
| `regime_badges_use_correct_css_classes` | Bull→`badge-bull`, Bear→`badge-bear`, Sideways→`badge-sideways` | P1 |
| `grade_badges_use_correct_css_classes` | A+→`badge-grade-aplus`, A→`badge-grade-a`, etc. | P1 |

**WCAG 2.1 AA compliance note:** The colors defined in `ghost-alpha.css` (section 5.1 of the design doc) must be verified manually or via a contrast tool:

- `#ff6b6b` text on white: ratio 3.4:1 — fails AA (4.5:1 minimum for normal text). The `badge-critical` class uses `color: #fff` on `background: #ff6b6b`. White on `#ff6b6b` = 3.4:1 — **fails AA for normal text, passes for large text (3:1)**. This should be noted and either the background darkened to `#e03131` or the badges styled as large text.
- `#ffd43b` text `#1a1a1a` on `#ffd43b` background: ratio approximately 11:1 — passes AA.
- `#74c0fc` info badge: `#1a1a1a` on `#74c0fc` = approximately 4.6:1 — passes AA.
- Recommendation: add a `contrast_ratio_note` comment in `ghost-alpha.css` for each badge class documenting the measured ratio.

### 11.3 Form Accessibility

| Test name | Route | Assertions | Priority |
|---|---|---|---|
| `watchlist_add_form_inputs_have_labels` | GET `/watchlist` | every `<input` in the add form is associated with a `<label` (either wrapping or via `for`/`id`) | P1 |
| `watchlist_add_form_ticker_input_has_required` | GET `/watchlist` | ticker input has `required` attribute | P0 |
| `theme_toggle_button_has_aria_label` | GET any page | body contains `aria-label="Toggle dark mode"` on theme toggle button | P1 |

---

## 12. Edge Case Tests

These are distributed into the unit and integration test files by subject. Collected here for completeness.

### 12.1 Empty Database (First Run)

All empty-DB tests have been listed per-route in sections 2 and 4. The key pattern: every page and every partial must render without panicking and must show a meaningful `empty-state` message, never a database error traceback or a blank page.

**Consolidated check test (P0, integration suite):**

```
all_routes_return_200_with_empty_db
  — loops over every registered route, dispatches GET with empty DB, asserts 200
```

### 12.2 Single Result

| Test name | File | Setup | Assertions | Priority |
|---|---|---|---|---|
| `home_with_exactly_one_screen_result` | routes.rs | 1 ScreenResult | table has exactly 1 data row, pagination shows `1–1 of 1` | P1 |
| `screen_table_partial_page_1_when_only_1_result` | partials.rs | 1 ScreenResult, GET `?page=1` | body shows empty tbody, pagination controls indicate end of results | P1 |

### 12.3 Extremely Long Ticker Names or Messages

| Test name | File | Setup | Assertions | Priority |
|---|---|---|---|---|
| `home_very_long_ticker_renders_without_breaking_layout` | routes.rs | ticker = `"AAAAAAAAAAAAAAAAAAA"` (19 chars) | status 200, no panic | P1 |
| `alerts_very_long_message_is_truncated_or_wrapped` | routes.rs | message = 1000-char string | status 200, no panic, body does not contain the full 1000-char string verbatim in a single `<td>` without wrapping attribute | P1 |
| `watchlist_very_long_notes_renders` | routes.rs | notes = 500-char string | status 200, no panic | P1 |

### 12.4 Unicode in Alert Messages

| Test name | File | Setup | Assertions | Priority |
|---|---|---|---|---|
| `alerts_page_unicode_message_renders` | routes.rs | message = `"NVDA: 🚀 breakout confirmed — 涨停 signal"` | status 200, body contains the emoji and CJK characters correctly | P0 |
| `html_escape_unicode_passthrough` | filters.rs | `"café résumé"` | identical string returned (no ASCII-only assumption) | P0 |
| `html_escape_unicode_with_special_chars` | filters.rs | `"<café>"` | `"&lt;café&gt;"` | P0 |

### 12.5 Date Boundaries

| Test name | File | Setup | Assertions | Priority |
|---|---|---|---|---|
| `screen_results_date_is_today_utc` | routes.rs | ScreenResult seeded with `chrono::Utc::now().date_naive()` | home page shows the result | P0 |
| `screen_results_yesterday_not_shown_on_home` | routes.rs | ScreenResult seeded for yesterday | home page shows empty-state (home is today-only) | P0 |
| `screen_table_partial_date_param_2023_01_01` | partials.rs | results on 2023-01-01 and 2023-01-02 | `?date=2023-01-01` returns only Jan-1 result | P0 |
| `screen_table_partial_invalid_date_param_defaults_gracefully` | partials.rs | 5 ScreenResults | GET `?date=not-a-date` returns 200, shows today or all results (no panic) | P1 |

### 12.6 Numeric Edge Cases in SVG Charts

| Test name | File | Input | Assertions | Priority |
|---|---|---|---|---|
| `price_chart_svg_all_same_price` | charts.rs | 252 candles all at $100.00 | no `NaN`, no div-by-zero panic | P0 |
| `price_chart_svg_monotonically_decreasing` | charts.rs | 252 declining candles | no negative coordinates in SVG output | P0 |
| `price_chart_svg_price_jumps_100x` | charts.rs | candles jumping from $1 to $100 in one bar | no coordinates outside viewBox bounds | P1 |
| `sparkline_svg_all_zeros` | charts.rs | [0.0; 10] | no `NaN`, valid SVG | P0 |

---

## 13. CI/CD Pipeline Integration

### 13.1 Current Pipeline Context

From `MEMORY.md`:
- `cargo test --lib --bins` runs on every push (unit tests only, fast)
- `cargo test --features integration` runs on PR only
- `cargo llvm-cov` for coverage reporting

### 13.2 Additions Required

**Unit tests (every push):**
- Web unit tests in `src/web/routes.rs`, `partials.rs`, `actions.rs`, `assets.rs`, `charts.rs`, `filters.rs` are already included in `--lib --bins` by default (they are `#[cfg(test)]` blocks in lib source files). No pipeline change needed.

**Integration tests (PR):**
- `tests/integration/web_test.rs` is added to `tests/integration.rs` with the `#[path]` pattern. It runs automatically when `cargo test --features integration --test integration` is invoked. No pipeline change needed beyond adding the file.

**Accessibility tests (PR):**
- `tests/integration/web_accessibility_test.rs` follows the same pattern.

**Benchmarks (scheduled / manual):**
- `cargo bench --bench web_bench` should be added as a manual workflow trigger, not a per-PR step. Benchmarks are slow and their output is directional, not a pass/fail gate.

### 13.3 Coverage Targets

Extending the existing coverage targets from `MEMORY.md`:

| Module | Target | Rationale |
|---|---|---|
| `src/web/routes.rs` | ≥ 90% | Full-page handlers are well-exercised by smoke + seeded tests |
| `src/web/partials.rs` | ≥ 90% | Every filter/pagination branch needs explicit coverage |
| `src/web/actions.rs` | ≥ 85% | Export and mutation handlers; SSE paths are harder to cover |
| `src/web/charts.rs` | ≥ 95% | Pure functions with no external dependencies; no excuse for low coverage |
| `src/web/filters.rs` | ≥ 95% | Pure formatting functions; exhaustive input space is small |
| `src/web/assets.rs` | ≥ 80% | Mostly a thin wrapper; main risk is MIME type map |

### 13.4 Quality Gate Additions

Add to the existing `cargo llvm-cov` step:

```yaml
- name: Coverage check (web modules)
  run: cargo llvm-cov --lib --bins --features integration
       --include-pattern "src/web/"
       --fail-under-lines 85
```

The `--fail-under-lines 85` threshold is conservative to account for the SSE and screen-run trigger code paths that are harder to test in CI.

---

## 14. Test Execution Guide

### 14.1 Running Web Tests Only

```bash
# Unit tests for the web module only (fast, no features needed):
cargo test --lib web

# All integration tests including web:
cargo test --features integration --test integration

# Web integration tests specifically:
cargo test --features integration --test integration web_test

# Benchmarks:
cargo bench --bench web_bench
```

### 14.2 Running a Single Test

```bash
# Run one specific test by name:
cargo test --lib web::routes::tests::home_with_seeded_screen_results

# Run all tests matching a pattern:
cargo test --lib web -- partial
```

### 14.3 Running with Coverage

```bash
cargo llvm-cov --lib --bins --features integration \
  --include-pattern "src/web/" \
  --html --open
```

### 14.4 Checking the Partial Invariant Locally

```bash
# Quick smoke-check that no partial returns a full HTML document:
cargo test --lib web -- partial_is_fragment
```

---

## 15. Test File Layout

The complete file layout after all tests are written:

```
src/web/
  mod.rs           — existing tests + make_state_with_seed helper
  routes.rs        — §2 tests (full-page handlers)
  partials.rs      — §4 tests (htmx fragments)
  actions.rs       — §5 tests (mutations + exports)
  assets.rs        — §6 tests (static file serving)
  charts.rs        — §3 tests (SVG generation)
  filters.rs       — §7 tests (formatting, escaping)

tests/
  integration.rs   — add: mod web_test; mod web_accessibility_test;
  integration/
    web_test.rs    — §8 (router smoke, round-trips, error handling)
    web_accessibility_test.rs  — §11 (semantic HTML, form labels)
  common/
    fixtures.rs    — add web-specific seed helpers (§1.2)

benches/
  web_bench.rs    — §10 (Criterion benchmarks)
```

---

## 16. Priority Summary

### P0 — Must Have (blocking for any PR touching web/)

- All full-page routes return HTTP 200 with `<!DOCTYPE html>` (one test per route)
- All partial routes return HTTP 200 without `<!DOCTYPE html>` (one test per partial)
- XSS escaping enforced: ticker, message, notes, exit reason (one test each)
- Static assets return 200 with correct Content-Type
- 404 for unknown stock ticker and unknown backtest ID
- Watchlist add/remove persists correctly and returns a fragment
- CSV and JSON export return correct Content-Type
- SVG functions never produce `NaN` or `inf` in output
- Settings page masks API key

### P1 — Should Have (required before stable release)

- All table columns from the design doc are present
- Filter/pagination invariants for screen-table and alert-list partials
- Regime badge variants (Bull/Bear/Sideways) all covered
- Score/grade badge variants all covered
- Conviction decay badge thresholds
- `all_full_page_routes_return_200` consolidated integration test
- Concurrent request handling (no deadlock with 50 simultaneous requests)
- All form inputs have associated labels
- Criterion benchmarks with 100-result and 1,000-result datasets

### P2 — Nice to Have (tracked but not blocking)

- Property-based tests for SVG validity (proptest)
- Pagination invariant properties
- VCP badge display
- Dark mode toggle accessibility
- Score history sparkline for watchlist entries
- Backtest report download Content-Disposition header
- 10,000-result benchmark
- Concurrent mixed read/write test

---

## 17. Known Gaps in Current Implementation

The following tests will fail against the current `src/web/routes.rs` implementation and should be marked `#[ignore]` with a comment until the corresponding feature is built. This documents technical debt without blocking CI.

| Test | Gap | Fixes when |
|---|---|---|
| `home_shows_all_12_table_columns` | Current table has 7 columns, not 12 | Home page expanded per §3.1 of design doc |
| `home_ticker_with_html_special_chars_is_escaped` | Current table rows do not call `html_escape()` on `r.ticker` or `r.composite_score` | Askama migration or manual fix |
| `watchlist_add_post_response_is_fragment` | Current handler re-renders full page | htmx partial target fix (design doc §3.4) |
| `watchlist_remove_post_response_is_fragment` | Same as above | Same |
| `static_asset_htmx_js_returns_200` | CDN-loaded, not embedded | rust-embed step (design doc §10 step 1) |
| `screen_table_partial_*` | `/partials/screen-table` route does not exist | Partials module implementation |
| `alert_list_partial_*` | `/partials/alert-list` route does not exist | Partials module implementation |
| `export_csv_*` | `/api/export/csv` route does not exist | Actions module implementation |
| `stock_detail_*` | `/stock/:ticker` route does not exist | Stock detail page (design doc §3.2) |
| `flagged_page_*` | `/flagged` route does not exist | Flagged page (design doc §3.6) |
| `backtest_page_*` | `/backtest` route does not exist | Backtest page (design doc §3.7) |
| `strategies_page_*` | `/strategies` route does not exist | Strategies page (design doc §3.8) |
| `settings_page_*` | `/settings` route does not exist | Settings page (design doc §3.9) |
