# Plan 14 — Fundamental Data Integration: Progress Log

**Plan:** `docs/planning/14-fundamental-data-integration.md`
**Started:** 2026-03-08
**Last updated:** 2026-03-09

---

## Phase A: Data Foundation ✅

**Status:** Complete

### Deliverables
- Fundamental data can be fetched from Finnhub and stored in SQLite
- All new tests pass
- Running `ghost-alpha screen` produces identical output to v1.2

### Files Created
| File | Purpose |
|------|---------|
| `src/data/fundamentals.rs` | `FundamentalProvider` trait, `FundamentalSnapshot`, `EarningsRecord`, `EarningsQuality`, `SectorPeBenchmark`, `FinnhubClient` impl, `MockFundamentalProvider` |
| `src/analysis/fundamental_scorer.rs` | 4-pillar scoring: earnings quality (0-40), growth (0-30), valuation (0-20), balance sheet (0-10) |

### Files Modified
| File | Changes |
|------|---------|
| `src/config.rs` | Added `FundamentalsConfig` struct (enabled, metrics_ttl_hours, earnings_ttl_hours, force_refresh, history_quarters, score_weight) |
| `src/storage/db.rs` | Added Repository trait methods for fundamentals persistence (upsert_fundamental_metrics, upsert_earnings_history, get_latest_fundamental, get_upcoming_earnings, get_sector_pe_benchmarks) |
| `src/storage/migrations.rs` | Added migrations 009-011 (fundamental_metrics, earnings_history, earnings_calendar tables) |
| `src/lib.rs` | Re-exported new modules |

---

## Phase B: Scoring Integration ✅

**Status:** Complete

### Deliverables
- `RegimeWeights` extended to 4 components (v1.3 weights)
- `composite_score_v2()` with graceful weight redistribution when fundamentals unavailable
- All 4 screeners use the new scoring path
- Backward-compatible: old `composite_score()` and `composite_score_weighted()` preserved

### Key Design Decisions

**Weight redistribution:** When `fundamental_score: None`, the 20-point fundamental weight is redistributed proportionally: 60% to technical, 30% to volume, 10% to sentiment. This produces scores nearly identical to the old 3-component system.

**v1.3 Regime Weights:**
| Regime | Tech | Vol | Sent | Fund |
|--------|------|-----|------|------|
| Bull | 45 | 20 | 15 | 20 |
| Sideways | 45 | 20 | 15 | 20 |
| Bear | 55 | 15 | 15 | 15 |

### Files Modified
| File | Changes |
|------|---------|
| `src/analysis/scorer.rs` | Added `fundamental: f64` to `CompositeScore` and `RegimeWeights`; `for_regime()` v1.3 weights; `legacy()` method; `FundamentalScoreInput`; `composite_score_v2()`; 7 new tests |
| `src/analysis/screener.rs` | Added `fundamental_score: Option<f64>` to `ScreenResult`; switched to `composite_score_v2` |
| `src/analysis/breakout.rs` | Switched to `composite_score_v2`; added `fundamental_score: None` |
| `src/analysis/mean_reversion.rs` | Switched to `composite_score_v2`; added `fundamental_score: None` |
| `src/analysis/trend_strength.rs` | Switched to `composite_score_v2`; added `fundamental_score: None` |
| `src/main.rs` | Updated CompositeScore construction sites; updated sentiment recomputation |
| `src/output.rs` | Added "Fund" column to `ScreenResult` Renderable impl |
| `src/storage/db.rs` | Added `fundamental_score: None` to deserialization |
| `src/alerts/engine.rs` | Added `fundamental_score: None` to test helpers |
| `src/alerts/narrative.rs` | Added `fundamental_score: None` to test helpers |
| `src/web/routes.rs` | Added `fundamental_score: None` to 5+ test ScreenResult literals |
| `tests/demo_sentiment_wiring.rs` | Updated to use `composite_score_v2` + `FundamentalScoreInput` |
| `tests/integration/sentiment_test.rs` | Updated to use `composite_score_v2` + `FundamentalScoreInput` |
| `tests/integration/alert_test.rs` | Added `fundamental_score: None` to ScreenResult literals |
| `tests/e2e/full_pipeline_test.rs` | Added `fundamental_score: None` to ScreenResult literals |

### Blast Radius
Phase B touched **16 files** — systematic grep for `CompositeScore {`, `RegimeWeights::`, and `ScreenResult {` ensured no construction sites were missed.

---

## Phase C: CLI Integration ✅

**Status:** Complete

### Deliverables
- `ghost-alpha fundamentals fetch <TICKER>` — fetches and caches fundamental metrics
- `ghost-alpha fundamentals show <TICKER>` — displays cached metrics
- `ghost-alpha fundamentals calendar [--days N]` — upcoming earnings dates
- `ghost-alpha fundamentals scores <TICKER>` — shows 4-pillar breakdown
- `--skip-fundamentals` flag on `ghost-alpha screen`
- Fundamental scoring wired into screen pipeline (section 9.7)

### Files Modified
| File | Changes |
|------|---------|
| `src/main.rs` | Added `FundAction` enum (Fetch/Show/Calendar/Scores); `--skip-fundamentals` flag; `run_fundamentals()` handler (~250 lines); fundamental scoring step between sentiment and grade assignment; `print_metric()` helper |

---

## Phase D: Dashboard Integration ✅

**Status:** Complete

### Deliverables
- `/fundamentals` page with full layout and htmx partials
- 5 SVG chart functions with 20+ tests
- Home page screening table updated with Sent and Fund columns
- "Fundamentals" nav link in shared layout

### New SVG Charts (src/web/charts.rs)
| Function | Description |
|----------|-------------|
| `earnings_calendar_svg()` | Horizontal timeline with colored date badges |
| `pe_comparison_svg()` | Horizontal bar chart with sector median reference line |
| `growth_bars_svg()` | Grouped vertical bars (revenue + EPS growth) |
| `eps_surprise_sparkline_svg()` | Sparkline with zero baseline, colored circles |
| `fundamental_color()` | Blue-palette color function for heatmap cells |

### New Routes (src/web/mod.rs, src/web/routes.rs)
| Route | Handler | Type |
|-------|---------|------|
| `GET /fundamentals` | `fundamentals_page` | Full page |
| `GET /partials/fundamental-heatmap` | `fundamental_heatmap_partial` | htmx partial |
| `GET /partials/earnings-calendar` | `earnings_calendar_partial` | htmx partial |
| `GET /partials/pe-chart` | `pe_chart_partial` | htmx partial |
| `GET /partials/growth-bars` | `growth_bars_partial` | htmx partial |

### Files Modified
| File | Changes |
|------|---------|
| `src/web/charts.rs` | 5 new chart functions, `EarningsCalendarEntry`/`PeComparisonEntry`/`GrowthBarEntry` structs, 20 tests |
| `src/web/routes.rs` | `fundamentals_page`, 4 partial handlers, "Fundamentals" nav link in `layout()`, updated `render_screen_rows` with Sent/Fund columns, 6 route tests |
| `src/web/mod.rs` | 5 new route registrations |

---

## Test Summary

| Phase | New Tests | Running Total |
|-------|-----------|---------------|
| A | ~30 | — |
| B | 7 scorer + updates across 16 files | — |
| C | — (CLI; covered by integration tests) | — |
| D | 20 chart + 6 route | — |
| E | 8 TTL + 3 pipeline + 1 stress | — |
| **Final** | — | **784 unit + 33 integration = 817** |

All tests passing as of 2026-03-10.

---

## Phase E: Polish & Testing ✅

**Status:** Complete

### Task Results

| Task | Status | Notes |
|------|--------|-------|
| E1: Audit `Option<f64>` fields | ✅ | All access sites handle `None` gracefully — no fixes needed |
| E2: Earnings season detection | ✅ | `is_ticker_near_earnings()` + `effective_metrics_ttl()` reduce cache TTL to 6h when within 7 days of earnings. 8 new tests |
| E3: `MockFundamentalProvider` | ✅ | Moved from `#[cfg(test)]` to `#[cfg(any(test, feature = "test-support"))]`; re-exported from `lib.rs` |
| E4: Integration test | ✅ | `tests/integration/fundamental_test.rs` — 3 tests: composite inclusion, grade differentiation, legacy weight compat |
| E5: Planning README | ✅ | Done in Phase D (Plan 14 entry added) |
| E6: PRD v1.3 | ✅ | Updated version, status, test count; added section 15.3 with all v1.3 features |
| E7: Coverage check | ⏭️ Skipped | `cargo-llvm-cov` not installed on this machine |
| E8: Binary size | ✅ | **6.6MB** release — well under 15MB limit |
| E9: Stress test | ✅ | `tests/integration/stress_test.rs` — 500 deterministic stocks, full pipeline, sub-1ms completion |
| E10: Config documentation | ✅ | `[fundamentals]` section added to README.md and config.example.toml |

### Files Created
| File | Purpose |
|------|---------|
| `tests/integration/fundamental_test.rs` | 3 integration tests for fundamental scoring pipeline |
| `tests/integration/stress_test.rs` | 500-stock stress test with deterministic data |

### Files Modified
| File | Changes |
|------|---------|
| `src/config.rs` | Added `earnings_season_ttl_hours` (default 6) and `earnings_season_window_days` (default 7) |
| `src/storage/db.rs` | Added `is_ticker_near_earnings()` and `effective_metrics_ttl()` helpers + 8 tests |
| `src/main.rs` | Wired per-ticker TTL into fundamental scoring loop |
| `src/data/fundamentals.rs` | Changed mock module gate to `#[cfg(any(test, feature = "test-support"))]` |
| `src/lib.rs` | Feature-gated re-export of `MockFundamentalProvider` |
| `Cargo.toml` | Added `test-support` feature flag |
| `Ghost_Alpha_PRD.md` | Updated to v1.3 with fundamental integration documentation |
| `README.md` | Added `[fundamentals]` config documentation |
| `config.example.toml` | Added `[fundamentals]` section with all fields |
| `tests/integration.rs` | Added `fundamental_test` and `stress_test` module entries |
