//! Ticker -> asset-class mapping.
//!
//! A small built-in map of broad, common tickers is shipped. Users extend or
//! override it with a `symbol_map.toml` file (see [`SymbolMap::from_toml_str`]).

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

use crate::projection::asset_class_library::pascal_to_snake;
use crate::projection::monte_carlo::AssetClass;

const BUILTIN: &[(&str, AssetClass)] = &[
    ("AAPL", AssetClass::TechMegaCap),
    ("AGG", AssetClass::UsTreasuryIntermediate),
    ("AMZN", AssetClass::ConsumerMegaCap),
    ("BND", AssetClass::UsTreasuryIntermediate),
    ("DBMF", AssetClass::ManagedFutures),
    ("FCASH**", AssetClass::CashMmf),
    ("FDRXX", AssetClass::CashMmf),
    ("FNILX", AssetClass::UsLargeCapIndex),
    ("FSKAX", AssetClass::UsTotalMarket),
    ("FXAIX", AssetClass::UsLargeCapIndex),
    ("FZROX", AssetClass::UsTotalMarket),
    ("GLD", AssetClass::Gold),
    ("GOOG", AssetClass::TechMegaCap),
    ("GOOGL", AssetClass::TechMegaCap),
    ("IAU", AssetClass::Gold),
    ("IEF", AssetClass::UsTreasuryIntermediate),
    ("IEFA", AssetClass::InternationalDeveloped),
    ("IEMG", AssetClass::EmergingMarketsEtf),
    ("ITOT", AssetClass::UsTotalMarket),
    ("IVV", AssetClass::UsLargeCapIndex),
    ("META", AssetClass::TechMegaCap),
    ("MSFT", AssetClass::TechMegaCap),
    ("NVDA", AssetClass::TechMegaCap),
    ("QQQ", AssetClass::UsBlueChipGrowth),
    ("SCHP", AssetClass::UsTips),
    ("SGOV", AssetClass::CashMmf),
    ("SPAXX", AssetClass::CashMmf),
    ("SPY", AssetClass::UsLargeCapIndex),
    ("TIP", AssetClass::UsTips),
    ("TLT", AssetClass::UsTreasuryLong),
    ("VEA", AssetClass::InternationalDeveloped),
    ("VMFXX", AssetClass::CashMmf),
    ("VOO", AssetClass::UsLargeCapIndex),
    ("VTI", AssetClass::UsTotalMarket),
    ("VWO", AssetClass::EmergingMarketsEtf),
    ("VXUS", AssetClass::IntlDevelopedEtf),
];

#[derive(Debug, thiserror::Error)]
pub enum SymbolMapError {
    #[error("failed to read symbol map {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid symbol map TOML: {0}")]
    Toml(#[from] toml::de::Error),
    #[error(
        "`default_class` is under the [symbols] table, where it would be read as a ticker; \
         move it above [symbols], at the top of the file"
    )]
    MisplacedDefaultClass,
    #[error("unknown asset class '{0}' in symbol map (use the snake_case keys from config/asset_classes.toml)")]
    UnknownClass(String),
}

/// Parse a snake_case asset-class name (e.g. `us_total_market`).
pub fn parse_asset_class(name: &str) -> Option<AssetClass> {
    let name = name.trim();
    AssetClass::iter_all().find(|c| pascal_to_snake(&format!("{c:?}")) == name)
}

/// Ticker -> asset-class map: built-in entries, optionally merged with user
/// entries, plus an optional fallback class for unmapped tickers.
#[derive(Debug, Clone)]
pub struct SymbolMap {
    map: HashMap<String, AssetClass>,
    default_class: Option<AssetClass>,
}

#[derive(Deserialize, Default)]
struct RawSymbolMap {
    default_class: Option<String>,
    #[serde(default)]
    symbols: HashMap<String, String>,
}

impl SymbolMap {
    pub fn builtin() -> Self {
        Self {
            map: BUILTIN
                .iter()
                .map(|(t, c)| ((*t).to_string(), *c))
                .collect(),
            default_class: None,
        }
    }

    /// Parse a TOML document and merge it over the built-in map.
    ///
    /// ```toml
    /// default_class = "us_large_cap_index"   # optional
    /// [symbols]
    /// FSKAX = "us_total_market"
    /// "BRK.B" = "us_large_cap_index"         # dotted tickers must be quoted
    /// ```
    pub fn from_toml_str(s: &str) -> Result<Self, SymbolMapError> {
        let raw: RawSymbolMap = toml::from_str(s)?;
        let mut out = Self::builtin();
        if raw
            .symbols
            .keys()
            .any(|k| k.trim().eq_ignore_ascii_case("default_class"))
        {
            return Err(SymbolMapError::MisplacedDefaultClass);
        }
        for (ticker, class) in raw.symbols {
            let c = parse_asset_class(&class).ok_or(SymbolMapError::UnknownClass(class))?;
            out.map.insert(ticker.trim().to_ascii_uppercase(), c);
        }
        if let Some(d) = raw.default_class {
            out.default_class = Some(parse_asset_class(&d).ok_or(SymbolMapError::UnknownClass(d))?);
        }
        Ok(out)
    }

    pub fn from_toml_file(p: &Path) -> Result<Self, SymbolMapError> {
        let s = std::fs::read_to_string(p).map_err(|source| SymbolMapError::Io {
            path: p.display().to_string(),
            source,
        })?;
        Self::from_toml_str(&s)
    }

    pub fn with_default_class(mut self, c: Option<AssetClass>) -> Self {
        self.default_class = c;
        self
    }

    /// Exact match, then upper-cased match, then the default class.
    pub fn class_of(&self, symbol: &str) -> Option<AssetClass> {
        self.map
            .get(symbol)
            .or_else(|| self.map.get(symbol.trim().to_ascii_uppercase().as_str()))
            .copied()
            .or(self.default_class)
    }

    pub fn default_class(&self) -> Option<AssetClass> {
        self.default_class
    }
}

impl Default for SymbolMap {
    fn default() -> Self {
        Self::builtin()
    }
}

/// Built-in lookup; equivalent to `SymbolMap::builtin().class_of(symbol)`.
pub fn symbol_to_class(symbol: &str) -> Option<AssetClass> {
    SymbolMap::builtin().class_of(symbol)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::monte_carlo::AssetClass;
    use tempfile::TempDir;

    /// Tickers the built-in map MUST contain (spec §3), with their classes.
    fn must_include() -> Vec<(&'static str, AssetClass)> {
        use AssetClass::*;
        vec![
            ("VOO", UsLargeCapIndex),
            ("IVV", UsLargeCapIndex),
            ("SPY", UsLargeCapIndex),
            ("FXAIX", UsLargeCapIndex),
            ("FNILX", UsLargeCapIndex),
            ("VTI", UsTotalMarket),
            ("ITOT", UsTotalMarket),
            ("FSKAX", UsTotalMarket),
            ("FZROX", UsTotalMarket),
            ("QQQ", UsBlueChipGrowth),
            ("VXUS", IntlDevelopedEtf),
            ("VEA", InternationalDeveloped),
            ("IEFA", InternationalDeveloped),
            ("VWO", EmergingMarketsEtf),
            ("IEMG", EmergingMarketsEtf),
            ("BND", UsTreasuryIntermediate),
            ("AGG", UsTreasuryIntermediate),
            ("IEF", UsTreasuryIntermediate),
            ("TLT", UsTreasuryLong),
            ("SCHP", UsTips),
            ("TIP", UsTips),
            ("IAU", Gold),
            ("GLD", Gold),
            ("DBMF", ManagedFutures),
            ("SPAXX", CashMmf),
            ("FCASH**", CashMmf),
            ("FDRXX", CashMmf),
            ("VMFXX", CashMmf),
            ("SGOV", CashMmf),
            ("AAPL", TechMegaCap),
            ("MSFT", TechMegaCap),
            ("NVDA", TechMegaCap),
            ("GOOGL", TechMegaCap),
            ("GOOG", TechMegaCap),
            ("META", TechMegaCap),
            ("AMZN", ConsumerMegaCap),
        ]
    }

    // --- Built-in map: must-include ---

    #[test]
    fn builtin_contains_every_required_ticker_with_the_specified_class() {
        let map = SymbolMap::builtin();
        for (ticker, class) in must_include() {
            assert_eq!(
                map.class_of(ticker),
                Some(class),
                "builtin map must map {ticker} to {class:?}"
            );
        }
    }

    // --- Built-in map: must-exclude ---

    #[test]
    fn builtin_does_not_contain_fictional_tickers() {
        let map = SymbolMap::builtin();
        for ticker in ["ZZZQ", "QQQZ", "ZZXA", "ZZXB", "ZZXC"] {
            assert_eq!(
                map.class_of(ticker),
                None,
                "builtin map must NOT include fictional ticker {ticker}"
            );
        }
    }

    #[test]
    fn builtin_has_no_default_class() {
        assert_eq!(SymbolMap::builtin().default_class(), None);
    }

    #[test]
    fn default_impl_equals_builtin_behaviour() {
        let d = SymbolMap::default();
        for (ticker, class) in must_include() {
            assert_eq!(d.class_of(ticker), Some(class));
        }
        assert_eq!(d.class_of("ZZZQ"), None);
        assert_eq!(d.default_class(), None);
    }

    // --- symbol_to_class compat wrapper ---

    #[test]
    fn symbol_to_class_equals_builtin_class_of() {
        let builtin = SymbolMap::builtin();
        for (ticker, _) in must_include() {
            assert_eq!(
                symbol_to_class(ticker),
                builtin.class_of(ticker),
                "{ticker}"
            );
        }
        for ticker in ["ZZZQ", "ZZXA", "NOT_A_REAL_TICKER", ""] {
            assert_eq!(
                symbol_to_class(ticker),
                builtin.class_of(ticker),
                "{ticker}"
            );
        }
    }

    #[test]
    fn symbol_to_class_returns_none_for_unknown_ticker() {
        assert_eq!(symbol_to_class("NOT_A_REAL_TICKER"), None);
    }

    #[test]
    fn symbol_to_class_maps_cash_funds_to_cash_mmf() {
        assert_eq!(symbol_to_class("SPAXX"), Some(AssetClass::CashMmf));
        // FCASH** — the `**` suffix is intentional; Fidelity uses it for held-cash
        assert_eq!(symbol_to_class("FCASH**"), Some(AssetClass::CashMmf));
    }

    // --- class_of lookup order ---

    #[test]
    fn class_of_falls_back_to_upper_cased_symbol() {
        let map = SymbolMap::builtin();
        assert_eq!(map.class_of("msft"), Some(AssetClass::TechMegaCap));
        assert_eq!(map.class_of("Voo"), Some(AssetClass::UsLargeCapIndex));
    }

    /// M1: TOML keys are upper-cased at ingestion, so lookups work in any case.
    #[test]
    fn toml_keys_are_upper_cased_at_ingestion() {
        let map = SymbolMap::from_toml_str("[symbols]\nzzzq = \"gold\"\n\"Qqqz\" = \"cash_mmf\"\n")
            .unwrap();
        assert_eq!(map.class_of("ZZZQ"), Some(AssetClass::Gold));
        assert_eq!(map.class_of("zzzq"), Some(AssetClass::Gold));
        assert_eq!(map.class_of("QQQZ"), Some(AssetClass::CashMmf));
    }

    #[test]
    fn lower_case_toml_key_overrides_builtin_entry() {
        let map = SymbolMap::from_toml_str("[symbols]\nvoo = \"gold\"\n").unwrap();
        assert_eq!(map.class_of("VOO"), Some(AssetClass::Gold));
    }

    // --- TOML merge over builtin ---

    #[test]
    fn from_toml_str_adds_new_symbols() {
        let map = SymbolMap::from_toml_str(
            "[symbols]\nZZZQ = \"industrial\"\nQQQZ = \"aerospace_defense\"\n",
        )
        .unwrap();
        assert_eq!(map.class_of("ZZZQ"), Some(AssetClass::Industrial));
        assert_eq!(map.class_of("QQQZ"), Some(AssetClass::AerospaceDefense));
    }

    #[test]
    fn from_toml_str_overrides_builtin_entries() {
        let map = SymbolMap::from_toml_str("[symbols]\nVOO = \"us_total_market\"\n").unwrap();
        assert_eq!(map.class_of("VOO"), Some(AssetClass::UsTotalMarket));
    }

    #[test]
    fn from_toml_str_keeps_builtin_entries_not_mentioned() {
        let map = SymbolMap::from_toml_str("[symbols]\nZZZQ = \"industrial\"\n").unwrap();
        for (ticker, class) in must_include() {
            assert_eq!(
                map.class_of(ticker),
                Some(class),
                "{ticker} lost after merge"
            );
        }
    }

    #[test]
    fn from_toml_str_empty_document_is_just_builtin() {
        let map = SymbolMap::from_toml_str("").unwrap();
        assert_eq!(map.class_of("AAPL"), Some(AssetClass::TechMegaCap));
        assert_eq!(map.class_of("ZZZQ"), None);
        assert_eq!(map.default_class(), None);
    }

    /// Tickers containing a dot must be quoted in TOML (bare `BRK.B` is a dotted key).
    #[test]
    fn from_toml_str_accepts_quoted_dotted_ticker() {
        let map =
            SymbolMap::from_toml_str("[symbols]\n\"BRK.B\" = \"us_large_cap_index\"\n").unwrap();
        assert_eq!(map.class_of("BRK.B"), Some(AssetClass::UsLargeCapIndex));
    }

    // --- default_class ---

    #[test]
    fn toml_default_class_applies_to_unmapped_tickers_only() {
        let map = SymbolMap::from_toml_str("default_class = \"us_large_cap_index\"\n").unwrap();
        assert_eq!(map.default_class(), Some(AssetClass::UsLargeCapIndex));
        assert_eq!(
            map.class_of("TOTALLY_UNKNOWN"),
            Some(AssetClass::UsLargeCapIndex)
        );
        // mapped tickers keep their own class
        assert_eq!(map.class_of("AAPL"), Some(AssetClass::TechMegaCap));
        assert_eq!(map.class_of("GLD"), Some(AssetClass::Gold));
    }

    #[test]
    fn with_default_class_some_sets_fallback() {
        let map = SymbolMap::builtin().with_default_class(Some(AssetClass::Gold));
        assert_eq!(map.default_class(), Some(AssetClass::Gold));
        assert_eq!(map.class_of("ZZZZ"), Some(AssetClass::Gold));
        // even a deliberately-excluded ticker falls back to the default class
        assert_eq!(map.class_of("ZZZQ"), Some(AssetClass::Gold));
        assert_eq!(map.class_of("VOO"), Some(AssetClass::UsLargeCapIndex));
    }

    #[test]
    fn with_default_class_none_clears_fallback() {
        let map = SymbolMap::from_toml_str("default_class = \"gold\"\n")
            .unwrap()
            .with_default_class(None);
        assert_eq!(map.default_class(), None);
        assert_eq!(map.class_of("ZZZZ"), None);
    }

    #[test]
    fn with_default_class_overrides_toml_default_class() {
        let map = SymbolMap::from_toml_str("default_class = \"gold\"\n")
            .unwrap()
            .with_default_class(Some(AssetClass::CashMmf));
        assert_eq!(map.class_of("ZZZZ"), Some(AssetClass::CashMmf));
    }

    // --- Errors ---

    #[test]
    fn from_toml_str_unknown_class_name_errors_naming_the_bad_value() {
        let err = SymbolMap::from_toml_str("[symbols]\nFOO = \"not_a_real_class\"\n")
            .expect_err("unknown class name must be an error");
        assert!(
            err.to_string().contains("not_a_real_class"),
            "error should name the bad class value: {err}"
        );
    }

    #[test]
    fn from_toml_str_unknown_default_class_errors_naming_the_bad_value() {
        let err = SymbolMap::from_toml_str("default_class = \"bogus_class\"\n")
            .expect_err("unknown default_class must be an error");
        assert!(err.to_string().contains("bogus_class"), "{err}");
    }

    #[test]
    fn from_toml_str_malformed_toml_is_an_error() {
        assert!(SymbolMap::from_toml_str("[symbols\nFOO = ").is_err());
    }

    // --- Files ---

    #[test]
    fn from_toml_file_reads_and_merges() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("symbol_map.toml");
        std::fs::write(
            &path,
            "default_class = \"us_total_market\"\n[symbols]\nFSKAX = \"us_total_market\"\nZZZQ = \"industrial\"\n",
        )
        .unwrap();
        let map = SymbolMap::from_toml_file(&path).unwrap();
        assert_eq!(map.class_of("ZZZQ"), Some(AssetClass::Industrial));
        assert_eq!(map.class_of("AAPL"), Some(AssetClass::TechMegaCap));
        assert_eq!(map.default_class(), Some(AssetClass::UsTotalMarket));
    }

    #[test]
    fn from_toml_file_missing_file_is_an_error() {
        let dir = TempDir::new().unwrap();
        assert!(SymbolMap::from_toml_file(&dir.path().join("nope.toml")).is_err());
    }

    // ═══ Round 3 ═══

    #[test]
    fn r3_default_class_under_symbols_table_is_an_error_with_a_hint() {
        let err = SymbolMap::from_toml_str("[symbols]\nVOO = \"gold\"\ndefault_class = \"gold\"\n")
            .expect_err("default_class under [symbols] is a misplaced key");
        let msg = err.to_string().to_lowercase();
        assert!(msg.contains("default_class"), "{msg}");
        assert!(
            msg.contains("above") && msg.contains("[symbols]"),
            "hint must say to move it above [symbols]: {msg}"
        );
    }

    #[test]
    fn r3_misplaced_default_class_in_a_file_is_also_an_error() {
        let dir = TempDir::new().unwrap();
        let p = dir.path().join("symbol_map.toml");
        std::fs::write(&p, "[symbols]\ndefault_class = \"us_total_market\"\n").unwrap();
        let msg = SymbolMap::from_toml_file(&p)
            .expect_err("misplaced")
            .to_string()
            .to_lowercase();
        assert!(
            msg.contains("default_class") && msg.contains("above"),
            "{msg}"
        );
    }

    #[test]
    fn r3_default_class_above_symbols_is_still_fine() {
        let map = SymbolMap::from_toml_str("default_class = \"gold\"\n[symbols]\nVOO = \"gold\"\n")
            .unwrap();
        assert_eq!(map.default_class(), Some(AssetClass::Gold));
    }
}
