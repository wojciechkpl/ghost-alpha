//! Per-asset-class long-run return / vol assumptions. Loaded from
//! `config/asset_classes.toml` via `AssetClassLibrary::from_toml_file()`.

use crate::projection::monte_carlo::AssetClass;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// Long-run nominal CAGR (μ_arith) and annual volatility (σ_arith) per class.
#[derive(Debug)]
pub struct AssetClassLibrary {
    means: HashMap<AssetClass, f64>,
    vols: HashMap<AssetClass, f64>,
}

impl AssetClassLibrary {
    pub fn mean(&self, c: AssetClass) -> f64 {
        *self
            .means
            .get(&c)
            .unwrap_or_else(|| panic!("AssetClassLibrary missing mean for {:?}", c))
    }

    pub fn vol(&self, c: AssetClass) -> f64 {
        *self
            .vols
            .get(&c)
            .unwrap_or_else(|| panic!("AssetClassLibrary missing vol for {:?}", c))
    }

    /// Built-in long-run return and volatility assumptions per asset class.
    /// These match the committed `config/asset_classes.toml`.
    pub fn builtin_defaults() -> Self {
        let means = HashMap::from([
            (AssetClass::UsLargeCapIndex, 0.085),
            (AssetClass::UsTotalMarket, 0.080),
            (AssetClass::UsQualityFactor, 0.085),
            (AssetClass::UsBlueChipGrowth, 0.090),
            (AssetClass::TechMegaCap, 0.100),
            (AssetClass::ConsumerMegaCap, 0.090),
            (AssetClass::InternationalDeveloped, 0.070),
            (AssetClass::EmergingAsia, 0.080),
            (AssetClass::EnergySector, 0.060),
            (AssetClass::AerospaceDefense, 0.090),
            (AssetClass::Industrial, 0.080),
            (AssetClass::CashMmf, 0.040),
            (AssetClass::UsTreasuryIntermediate, 0.045),
            (AssetClass::UsTreasuryLong, 0.050),
            (AssetClass::UsTips, 0.040),
            (AssetClass::Gold, 0.050),
            (AssetClass::ManagedFutures, 0.060),
            (AssetClass::IntlDevelopedEtf, 0.070),
            (AssetClass::EmergingMarketsEtf, 0.080),
        ]);

        let vols = HashMap::from([
            (AssetClass::UsLargeCapIndex, 0.16),
            (AssetClass::UsTotalMarket, 0.16),
            (AssetClass::UsQualityFactor, 0.15),
            (AssetClass::UsBlueChipGrowth, 0.20),
            (AssetClass::TechMegaCap, 0.32),
            (AssetClass::ConsumerMegaCap, 0.30),
            (AssetClass::InternationalDeveloped, 0.18),
            (AssetClass::EmergingAsia, 0.24),
            (AssetClass::EnergySector, 0.28),
            (AssetClass::AerospaceDefense, 0.20),
            (AssetClass::Industrial, 0.22),
            (AssetClass::CashMmf, 0.005),
            (AssetClass::UsTreasuryIntermediate, 0.08),
            (AssetClass::UsTreasuryLong, 0.18),
            (AssetClass::UsTips, 0.06),
            (AssetClass::Gold, 0.16),
            (AssetClass::ManagedFutures, 0.14),
            (AssetClass::IntlDevelopedEtf, 0.18),
            (AssetClass::EmergingMarketsEtf, 0.22),
        ]);

        Self { means, vols }
    }

    /// Parse a TOML string and construct an `AssetClassLibrary`.
    ///
    /// Each top-level TOML table key must be the snake_case name of an
    /// `AssetClass` variant (e.g. `us_large_cap_index`). All 19 variants must
    /// be present. Unknown keys produce `AssetClassLibraryError::UnknownClass`;
    /// missing variants produce `AssetClassLibraryError::MissingClass`.
    pub fn from_toml_str(s: &str) -> Result<Self, AssetClassLibraryError> {
        // Typed intermediate: mean/vol as Option<f64> so we can distinguish
        // "field absent" from "field present but wrong type".
        #[derive(serde::Deserialize)]
        struct RawAssetClassEntry {
            mean: Option<f64>,
            vol: Option<f64>,
            #[serde(default)]
            #[allow(dead_code)]
            source: String,
        }

        let raw: BTreeMap<String, RawAssetClassEntry> = toml::from_str(s)?;

        // Build a lookup table: snake_case name → AssetClass variant.
        let snake_to_variant: HashMap<String, AssetClass> = AssetClass::iter_all()
            .map(|c| (pascal_to_snake(&format!("{:?}", c)), c))
            .collect();

        let mut means = HashMap::with_capacity(19);
        let mut vols = HashMap::with_capacity(19);

        // Validate every key in the TOML and extract mean/vol.
        for (key, entry) in &raw {
            let &variant = snake_to_variant
                .get(key.as_str())
                .ok_or_else(|| AssetClassLibraryError::UnknownClass(key.clone()))?;

            let mean = entry
                .mean
                .ok_or_else(|| AssetClassLibraryError::MissingField {
                    class: key.clone(),
                    field: "mean",
                })?;
            let vol = entry
                .vol
                .ok_or_else(|| AssetClassLibraryError::MissingField {
                    class: key.clone(),
                    field: "vol",
                })?;

            means.insert(variant, mean);
            vols.insert(variant, vol);
        }

        // Verify every AssetClass variant is present.
        for variant in AssetClass::iter_all() {
            if !means.contains_key(&variant) {
                return Err(AssetClassLibraryError::MissingClass(variant));
            }
        }

        Ok(Self { means, vols })
    }

    /// Read a TOML file from `path` and parse it via `from_toml_str`.
    pub fn from_toml_file(path: &Path) -> Result<Self, AssetClassLibraryError> {
        let s = std::fs::read_to_string(path)
            .map_err(|e| AssetClassLibraryError::Io(path.display().to_string(), e))?;
        Self::from_toml_str(&s)
    }
}

/// Errors that can arise when loading an `AssetClassLibrary` from TOML.
#[derive(Debug, thiserror::Error)]
pub enum AssetClassLibraryError {
    /// I/O failure while reading the TOML file (only from `from_toml_file`).
    #[error("I/O error reading asset class library {0}: {1}")]
    Io(String, #[source] std::io::Error),

    /// The TOML text is syntactically invalid.
    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml::de::Error),

    /// A top-level table key in the TOML does not map to any known `AssetClass`.
    #[error("Unknown asset class key in TOML: {0}")]
    UnknownClass(String),

    /// A required `AssetClass` variant was absent from the TOML.
    #[error("AssetClass missing from TOML: {0:?}")]
    MissingClass(AssetClass),

    /// A required field (`mean` or `vol`) was absent for the named class.
    #[error("AssetClass {class} missing required field: {field}")]
    MissingField {
        class: String,
        /// Always a compile-time constant: either `"mean"` or `"vol"`.
        field: &'static str,
    },
}

/// Convert a PascalCase string (as produced by `{:?}` on an enum variant) to
/// snake_case. E.g. `"UsTreasuryLong"` → `"us_treasury_long"`.
pub(crate) fn pascal_to_snake(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for (i, ch) in s.chars().enumerate() {
        if ch.is_ascii_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(ch.to_ascii_lowercase());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Canonical TOML string covering all 19 asset classes.  Values must match
    // `builtin_defaults()` exactly (no tolerance — they are
    // literal f64 constants in both places).
    // -----------------------------------------------------------------------
    const CANONICAL_TOML: &str = r#"
[us_large_cap_index]
mean = 0.085
vol  = 0.16
source = "Ibbotson 1926-2024 ~10%, haircut for elevated 2026 CAPE"

[us_total_market]
mean = 0.080
vol  = 0.16
source = "Slightly below large-cap due to small/mid-cap drag in 2024-2026 regime"

[us_quality_factor]
mean = 0.085
vol  = 0.15
source = "Quality premium over market, lower realised vol than pure large-cap"

[us_blue_chip_growth]
mean = 0.090
vol  = 0.20
source = "Growth tilt adds return and vol vs blend"

[tech_mega_cap]
mean = 0.100
vol  = 0.32
source = "FAAMNG proxy; higher return but idiosyncratic vol"

[consumer_mega_cap]
mean = 0.090
vol  = 0.30
source = "AMZN/TSLA-type basket; elevated vol"

[international_developed]
mean = 0.070
vol  = 0.18
source = "MSCI EAFE long-run with USD hedging haircut"

[emerging_asia]
mean = 0.080
vol  = 0.24
source = "MSCI EM Asia; higher growth, higher vol"

[energy_sector]
mean = 0.060
vol  = 0.28
source = "XLE long-run; commodity-driven vol"

[aerospace_defense]
mean = 0.090
vol  = 0.20
source = "ITA long-run; elevated by defense cycle"

[industrial]
mean = 0.080
vol  = 0.22
source = "XLI long-run average"

[cash_mmf]
mean = 0.040
vol  = 0.005
source = "Fed Funds forward curve 2026-2031 average"

[us_treasury_intermediate]
mean = 0.045
vol  = 0.08
source = "IEF long-run; duration ~7y"

[us_treasury_long]
mean = 0.050
vol  = 0.18
source = "TLT long-run; duration ~17y, higher vol"

[us_tips]
mean = 0.040
vol  = 0.06
source = "SCHP long-run real return + expected inflation"

[gold]
mean = 0.050
vol  = 0.16
source = "GLD long-run; inflation hedge, low correlation"

[managed_futures]
mean = 0.060
vol  = 0.14
source = "SG CTA index long-run; crisis alpha"

[intl_developed_etf]
mean = 0.070
vol  = 0.18
source = "EFA long-run; same regime as international_developed"

[emerging_markets_etf]
mean = 0.080
vol  = 0.22
source = "EEM long-run; slightly tighter vol than emerging_asia basket"
"#;

    // -----------------------------------------------------------------------
    // Round-trip tests (3)
    // -----------------------------------------------------------------------

    #[test]
    fn from_toml_str_matches_builtin_defaults_for_canonical_input_mean() {
        let toml_lib = AssetClassLibrary::from_toml_str(CANONICAL_TOML)
            .expect("canonical TOML must parse without error");
        let defaults = AssetClassLibrary::builtin_defaults();

        for c in AssetClass::iter_all() {
            assert_eq!(
                toml_lib.mean(c),
                defaults.mean(c),
                "mean mismatch for {:?}",
                c
            );
        }
    }

    #[test]
    fn from_toml_str_matches_builtin_defaults_for_canonical_input_vol() {
        let toml_lib = AssetClassLibrary::from_toml_str(CANONICAL_TOML)
            .expect("canonical TOML must parse without error");
        let defaults = AssetClassLibrary::builtin_defaults();

        for c in AssetClass::iter_all() {
            assert_eq!(toml_lib.vol(c), defaults.vol(c), "vol mismatch for {:?}", c);
        }
    }

    #[test]
    fn from_toml_file_loads_committed_config() {
        // Tests run from the repo root; Path::new("config/asset_classes.toml")
        // resolves against that working directory.
        let file_lib = AssetClassLibrary::from_toml_file(Path::new("config/asset_classes.toml"))
            .expect("committed config/asset_classes.toml must load without error");
        let defaults = AssetClassLibrary::builtin_defaults();

        for c in AssetClass::iter_all() {
            assert_eq!(
                file_lib.mean(c),
                defaults.mean(c),
                "mean mismatch for {:?} when loading committed file",
                c
            );
            assert_eq!(
                file_lib.vol(c),
                defaults.vol(c),
                "vol mismatch for {:?} when loading committed file",
                c
            );
        }
    }

    // -----------------------------------------------------------------------
    // Error-path tests (4)
    // -----------------------------------------------------------------------

    #[test]
    fn from_toml_str_fails_on_unknown_class_key() {
        // A TOML entry whose key does not correspond to any AssetClass variant.
        let bad_toml = format!(
            "{}\n[bogus_class]\nmean = 0.05\nvol = 0.10\nsource = \"invented\"\n",
            CANONICAL_TOML
        );
        let err = AssetClassLibrary::from_toml_str(&bad_toml)
            .expect_err("TOML with unknown key must fail");
        assert!(
            matches!(err, AssetClassLibraryError::UnknownClass(ref k) if k == "bogus_class"),
            "expected UnknownClass(\"bogus_class\"), got {:?}",
            err
        );
    }

    #[test]
    fn from_toml_str_fails_on_missing_class() {
        // Remove the [gold] entry from the canonical TOML.
        let toml_without_gold: String = CANONICAL_TOML
            .lines()
            .scan(false, |in_gold_block, line| {
                if line.trim() == "[gold]" {
                    *in_gold_block = true;
                } else if line.starts_with('[') && line.trim() != "[gold]" {
                    *in_gold_block = false;
                }
                if *in_gold_block {
                    Some(None) // drop this line
                } else {
                    Some(Some(line))
                }
            })
            .flatten()
            .collect::<Vec<_>>()
            .join("\n");

        let err = AssetClassLibrary::from_toml_str(&toml_without_gold)
            .expect_err("TOML missing [gold] must fail");
        assert!(
            matches!(err, AssetClassLibraryError::MissingClass(AssetClass::Gold)),
            "expected MissingClass(Gold), got {:?}",
            err
        );
    }

    #[test]
    fn from_toml_str_fails_on_missing_field() {
        // [gold] present but vol is absent.
        let toml_gold_no_vol = r#"
[us_large_cap_index]
mean = 0.085
vol  = 0.16
source = "test"

[us_total_market]
mean = 0.080
vol  = 0.16
source = "test"

[us_quality_factor]
mean = 0.085
vol  = 0.15
source = "test"

[us_blue_chip_growth]
mean = 0.090
vol  = 0.20
source = "test"

[tech_mega_cap]
mean = 0.100
vol  = 0.32
source = "test"

[consumer_mega_cap]
mean = 0.090
vol  = 0.30
source = "test"

[international_developed]
mean = 0.070
vol  = 0.18
source = "test"

[emerging_asia]
mean = 0.080
vol  = 0.24
source = "test"

[energy_sector]
mean = 0.060
vol  = 0.28
source = "test"

[aerospace_defense]
mean = 0.090
vol  = 0.20
source = "test"

[industrial]
mean = 0.080
vol  = 0.22
source = "test"

[cash_mmf]
mean = 0.040
vol  = 0.005
source = "test"

[us_treasury_intermediate]
mean = 0.045
vol  = 0.08
source = "test"

[us_treasury_long]
mean = 0.050
vol  = 0.18
source = "test"

[us_tips]
mean = 0.040
vol  = 0.06
source = "test"

[gold]
mean = 0.050
source = "test"

[managed_futures]
mean = 0.060
vol  = 0.14
source = "test"

[intl_developed_etf]
mean = 0.070
vol  = 0.18
source = "test"

[emerging_markets_etf]
mean = 0.080
vol  = 0.22
source = "test"
"#;
        let err = AssetClassLibrary::from_toml_str(toml_gold_no_vol)
            .expect_err("TOML with missing vol for gold must fail");
        assert!(
            matches!(
                err,
                AssetClassLibraryError::MissingField {
                    ref class,
                    field: "vol"
                } if class.contains("gold")
            ),
            "expected MissingField {{ class: \"gold\", field: \"vol\" }}, got {:?}",
            err
        );
    }

    #[test]
    fn from_toml_str_fails_on_malformed_toml() {
        let err = AssetClassLibrary::from_toml_str("not valid toml ====")
            .expect_err("malformed TOML must fail");
        assert!(
            matches!(err, AssetClassLibraryError::TomlParse(_)),
            "expected TomlParse(_), got {:?}",
            err
        );
    }

    // -----------------------------------------------------------------------
    // Snake_case → PascalCase mapping (test 8)
    //
    // Strategy: for a representative sample of variants across all category
    // groups, build a single-entry TOML (just that one class with mean=0.0,
    // vol=0.0).  Parsing must return Err(MissingClass(_)) — proving the parser
    // *did* recognise the present class before complaining about the 18 absent
    // ones.  If the snake_case key were unrecognised we would get
    // Err(UnknownClass(_)) instead.
    // -----------------------------------------------------------------------

    fn single_class_toml(snake_key: &str) -> String {
        format!(
            "[{snake_key}]\nmean = 0.0\nvol = 0.0\nsource = \"test\"\n",
            snake_key = snake_key
        )
    }

    #[test]
    fn snake_to_pascal_handles_representative_classes() {
        // One variant per logical group:
        //   us_core       → UsLargeCapIndex
        //   us_quality    → UsQualityFactor
        //   us_mega       → TechMegaCap
        //   intl          → InternationalDeveloped
        //   intl_etf      → IntlDevelopedEtf
        //   sector        → AerospaceDefense
        //   cash          → CashMmf
        //   treasury      → UsTreasuryIntermediate
        //   tips          → UsTips
        //   gold          → Gold
        //   mf            → ManagedFutures
        //   emerging      → EmergingMarketsEtf
        let cases = [
            "us_large_cap_index",
            "us_quality_factor",
            "tech_mega_cap",
            "international_developed",
            "intl_developed_etf",
            "aerospace_defense",
            "cash_mmf",
            "us_treasury_intermediate",
            "us_tips",
            "gold",
            "managed_futures",
            "emerging_markets_etf",
        ];

        for snake in cases {
            let toml = single_class_toml(snake);
            let err = AssetClassLibrary::from_toml_str(&toml).expect_err(&format!(
                "single-class TOML for '{snake}' must fail (18 classes missing)"
            ));
            assert!(
                matches!(err, AssetClassLibraryError::MissingClass(_)),
                "expected MissingClass(_) for snake key '{snake}', got {:?}",
                err
            );
        }
    }

    // -----------------------------------------------------------------------
    // Sanity (test 9)
    // -----------------------------------------------------------------------

    #[test]
    fn committed_config_file_exists_and_parses() {
        let path = Path::new("config/asset_classes.toml");
        assert!(
            path.exists(),
            "config/asset_classes.toml must exist; commit it as part of Gap 7"
        );
        AssetClassLibrary::from_toml_file(path)
            .expect("config/asset_classes.toml must parse without error");
    }

    #[test]
    fn h4_from_toml_file_missing_path_error_includes_the_path() {
        let p = std::path::Path::new("/nonexistent_dir_for_ghost_alpha/xyz_classes.toml");
        let err = AssetClassLibrary::from_toml_file(p).expect_err("missing file");
        assert!(matches!(err, AssetClassLibraryError::Io(..)), "{err:?}");
        assert!(err.to_string().contains("xyz_classes.toml"), "{err}");
    }
}
