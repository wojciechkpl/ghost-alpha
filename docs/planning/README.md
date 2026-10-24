# Ghost Alpha — Adoption & Improvement Plans

Detailed implementation plans for making Ghost Alpha useful beyond a single-user tool.

> **All 10 plans have been implemented as of March 2026.**

## Plans (in priority order)

| # | Plan | Effort | Impact | Status | File |
|---|------|--------|--------|--------|------|
| 1 | Demo mode — zero-config first experience | Small | Very High | ✅ Implemented | [01-demo-mode.md](01-demo-mode.md) |
| 2 | Docker one-liner deployment | Small | High | ✅ Implemented | [02-docker.md](02-docker.md) |
| 3 | Interactive setup wizard | Medium | High | ✅ Implemented | [03-setup-wizard.md](03-setup-wizard.md) |
| 4 | Watchlist-first workflow | Small | High | ✅ Implemented | [04-watchlist.md](04-watchlist.md) |
| 5 | JSON/CSV export | Small | Medium | ✅ Implemented | [05-export-formats.md](05-export-formats.md) |
| 6 | Narrative alert mode | Small | Medium | ✅ Implemented | [06-narrative-alerts.md](06-narrative-alerts.md) |
| 7 | Published backtest report | Medium | High | ✅ Implemented | [07-backtest-report.md](07-backtest-report.md) |
| 8 | Yahoo Finance data provider | Medium | Very High | ✅ Implemented | [08-yahoo-provider.md](08-yahoo-provider.md) |
| 9 | Built-in strategy presets | Large | High | ✅ Implemented | [09-strategy-presets.md](09-strategy-presets.md) |
| 10 | Embedded web dashboard | Large | Very High | ✅ Implemented | [10-web-dashboard.md](10-web-dashboard.md) |

## Guiding Principles

- **Time-to-value:** A new user must get value within 10 minutes
- **Single binary:** Never break the zero-dependency deployment story
- **Additive only:** New features are opt-in; existing workflows stay untouched
- **Trust through transparency:** Show track records, not just claims
