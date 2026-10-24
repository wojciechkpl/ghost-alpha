# Cross-language reference fixtures

These files pin the Rust projection engine to an independent Python reference
implementation of the same model (lognormal Monte Carlo with block
correlations, three vest treatments, concentration-floor unwind).

All portfolio-dependent fixtures were generated from the **synthetic** example
portfolio in `tests/fixtures/fidelity/example_main.csv` +
`example_small.csv` and the synthetic schedule in
`tests/fixtures/vesting/sample.csv` (concentrated symbol `MSFT`, as of
2026-05-01). The reference parameters were:

| parameter | value |
|---|---|
| withhold rate | 0.22 |
| LTCG rate (unwind) | 0.238 |
| concentrated cost basis / share | 185.00 |
| concentration floor | 0.25 |
| unwind schedule (existing shares) | year 1: 400, year 2: 300 |
| MC seed / paths | 42 / 5000 |

| file | contents | portfolio-dependent |
|---|---|---|
| `correlation.json` | PSD-clipped block correlation matrix | no |
| `year1_moments.json` | per-class year-1 return moments (seed 42) | no |
| `audit.json` | MC run audit block | yes |
| `audit_deterministic.json` | bear/base/bull × HOLD/SellReinvest/OptionB year-5 NAV | yes |
| `nav_projection.csv` | percentile aggregation per treatment × year | yes |

The Python reference is not part of this repository. If you change the model,
update these goldens deliberately and note why in the commit message.
