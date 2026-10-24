//! FRED (Federal Reserve Economic Data) API helpers.
//!
//! FRED is a free, open-source data provider for macro-economic indicators.
//! Series IDs used: VIXCLS (VIX), BAMLH0A0HYM2 (HY OAS), T10Y2Y (2s10s spread).

use crate::error::DataError;

/// Build the FRED API URL for fetching the most recent observation of a series.
pub fn fred_series_url(series_id: &str, api_key: &str) -> String {
    format!(
        "https://api.stlouisfed.org/fred/series/observations\
         ?series_id={series_id}\
         &api_key={api_key}\
         &sort_order=desc\
         &limit=1\
         &file_type=json"
    )
}

/// Parse the latest value from a FRED API JSON response.
///
/// FRED returns observations as:
/// ```json
/// { "observations": [{ "date": "2024-01-15", "value": "14.23" }] }
/// ```
///
/// Missing data is represented as `"."` — returns an error in that case.
pub fn parse_fred_latest(json: &str) -> Result<f64, DataError> {
    let parsed: serde_json::Value =
        serde_json::from_str(json).map_err(|e| DataError::Parse(e.to_string()))?;

    let observations = parsed
        .get("observations")
        .and_then(|v| v.as_array())
        .ok_or_else(|| DataError::Parse("Missing 'observations' array".into()))?;

    if observations.is_empty() {
        return Err(DataError::Parse("Empty observations array".into()));
    }

    let value_str = observations[0]
        .get("value")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DataError::Parse("Missing 'value' field".into()))?;

    if value_str == "." {
        return Err(DataError::Parse("Missing data (value = '.')".into()));
    }

    value_str
        .parse::<f64>()
        .map_err(|e| DataError::Parse(format!("Cannot parse '{}' as f64: {}", value_str, e)))
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_format_correct() {
        let url = fred_series_url("VIXCLS", "abc123");
        assert!(url.contains("series_id=VIXCLS"));
        assert!(url.contains("api_key=abc123"));
        assert!(url.contains("file_type=json"));
        assert!(url.contains("limit=1"));
    }

    #[test]
    fn parse_latest_value() {
        let json = r#"{"observations": [{"date": "2024-01-15", "value": "14.23"}]}"#;
        let val = parse_fred_latest(json).unwrap();
        assert!((val - 14.23).abs() < 0.001);
    }

    #[test]
    fn parse_missing_data_dot() {
        let json = r#"{"observations": [{"date": "2024-01-15", "value": "."}]}"#;
        let err = parse_fred_latest(json).unwrap_err();
        assert!(matches!(err, DataError::Parse(_)));
    }

    #[test]
    fn parse_empty_observations() {
        let json = r#"{"observations": []}"#;
        let err = parse_fred_latest(json).unwrap_err();
        assert!(matches!(err, DataError::Parse(_)));
    }
}
