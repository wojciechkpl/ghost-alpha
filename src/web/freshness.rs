//! Data-freshness indicator ("Screen: Fresh, 2026-10-23, 1 trading day") shown
//! in the page header.
//!
//! "Trading days" are plain weekdays (Mon-Fri); exchange holidays are not
//! modelled, so the age can overstate by one around a holiday.

use chrono::{Datelike, Duration, NaiveDate, Weekday};

use super::shell::html_escape;

/// How current the newest screen run is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    Fresh,
    Warn,
    Stale,
    None,
}

impl Freshness {
    /// Value of the `data-freshness` attribute.
    pub fn as_str(self) -> &'static str {
        match self {
            Freshness::Fresh => "fresh",
            Freshness::Warn => "warn",
            Freshness::Stale => "stale",
            Freshness::None => "none",
        }
    }

    /// Visible state word (colour is never the only signal).
    pub fn word(self) -> &'static str {
        match self {
            Freshness::Fresh => "Fresh",
            Freshness::Warn => "Aging",
            Freshness::Stale => "Stale",
            Freshness::None => "No data",
        }
    }
}

/// Weekdays in the half-open interval `(as_of, today]`; 0 if `as_of >= today`.
pub fn trading_days_between(as_of: NaiveDate, today: NaiveDate) -> u32 {
    if as_of >= today {
        return 0;
    }
    let days = (today - as_of).num_days();
    let full_weeks = days / 7;
    let mut n = full_weeks * 5;
    let mut d = as_of + Duration::days(full_weeks * 7);
    while d < today {
        d += Duration::days(1);
        if !matches!(d.weekday(), Weekday::Sat | Weekday::Sun) {
            n += 1;
        }
    }
    n as u32
}

/// Age <= 1 trading day is fresh, 2..=3 aging, 4+ stale; no data is `None`.
pub fn classify(as_of: Option<NaiveDate>, today: NaiveDate) -> Freshness {
    match as_of {
        None => Freshness::None,
        Some(d) => match trading_days_between(d, today) {
            0..=1 => Freshness::Fresh,
            2..=3 => Freshness::Warn,
            _ => Freshness::Stale,
        },
    }
}

/// Render the header chip for the newest screen run date (`as_of`).
pub fn chip_html(as_of: Option<NaiveDate>, today: NaiveDate) -> String {
    let state = classify(as_of, today);
    let detail = match as_of {
        Some(d) => {
            let n = trading_days_between(d, today);
            let age = match n {
                0 => "today".to_string(),
                1 => "1 trading day".to_string(),
                n => format!("{n} trading days"),
            };
            format!(" &middot; {} &middot; {age}", html_escape(&d.to_string()))
        }
        None => String::new(),
    };
    format!(
        r#"<span class="freshness-chip" data-freshness="{}">Screen: {}{detail}</span>"#,
        state.as_str(),
        state.word()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_path_matches_naive_count() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        for a in 0..40 {
            for b in 0..40 {
                let (x, y) = (start + Duration::days(a), start + Duration::days(b));
                let mut naive = 0;
                let mut d = x;
                while d < y {
                    d += Duration::days(1);
                    if !matches!(d.weekday(), Weekday::Sat | Weekday::Sun) {
                        naive += 1;
                    }
                }
                assert_eq!(trading_days_between(x, y), naive, "{x} {y}");
            }
        }
    }
}
