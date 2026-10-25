//! RED-phase contract for dashboard Q4: the freshness chip.
//!
//! Public API (new module `ghost_alpha::web::freshness`):
//! ```ignore
//! #[derive(Debug, Clone, Copy, PartialEq, Eq)]
//! pub enum Freshness { Fresh, Warn, Stale, None }
//! impl Freshness { pub fn as_str(self) -> &'static str } // "fresh" | "warn" | "stale" | "none"
//! /// Weekdays (Mon-Fri) in the half-open interval (as_of, today]. 0 if as_of >= today.
//! pub fn trading_days_between(as_of: NaiveDate, today: NaiveDate) -> u32;
//! /// None => Freshness::None. age <= 1 => Fresh, 2..=3 => Warn, >= 4 => Stale
//! /// ("older than 1 trading day" is warn, "older than 3" is stale).
//! pub fn classify(as_of: Option<NaiveDate>, today: NaiveDate) -> Freshness;
//! ```
//! Rendered chip (every full page, inside `<header>`): one element carrying
//! `data-freshness="fresh|warn|stale|none"` and class `freshness-chip`. Its visible text names
//! the source ("Screen"), a state word (Fresh / Aging / Stale / No data), and, when data exists,
//! the as-of date `YYYY-MM-DD` and the age as "{n} trading day(s)" ("today" is accepted for
//! n = 0). The as-of date is the newest `run_date` in screen results; "today" is the UTC date.

mod common;

use chrono::NaiveDate;
use common::web::*;
use ghost_alpha::web::freshness::{classify, trading_days_between, Freshness};

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

#[test]
fn test_trading_days_when_weekend_in_between_should_count_weekdays_only() {
    // 2026-10-23 is a Friday.
    let fri = d(2026, 10, 23);
    assert_eq!(chrono::Datelike::weekday(&fri), chrono::Weekday::Fri);
    assert_eq!(trading_days_between(fri, d(2026, 10, 23)), 0);
    assert_eq!(trading_days_between(fri, d(2026, 10, 24)), 0); // Sat
    assert_eq!(trading_days_between(fri, d(2026, 10, 25)), 0); // Sun
    assert_eq!(trading_days_between(fri, d(2026, 10, 26)), 1); // Mon
    assert_eq!(trading_days_between(fri, d(2026, 10, 27)), 2); // Tue
    assert_eq!(trading_days_between(d(2026, 10, 19), d(2026, 10, 26)), 5);
}

#[test]
fn test_trading_days_when_as_of_is_in_the_future_should_be_zero() {
    assert_eq!(trading_days_between(d(2026, 10, 28), d(2026, 10, 25)), 0);
}

#[test]
fn test_classify_when_age_crosses_thresholds_should_follow_pinned_rule() {
    let today = d(2026, 10, 28); // Wednesday
    let cases = [
        (d(2026, 10, 28), Freshness::Fresh), // 0
        (d(2026, 10, 27), Freshness::Fresh), // 1
        (d(2026, 10, 26), Freshness::Warn),  // 2
        (d(2026, 10, 23), Freshness::Warn),  // 3 (Fri)
        (d(2026, 10, 22), Freshness::Stale), // 4
        (d(2026, 9, 1), Freshness::Stale),
    ];
    for (as_of, want) in cases {
        assert_eq!(classify(Some(as_of), today), want, "{as_of}");
    }
    assert_eq!(classify(None, today), Freshness::None);
}

#[test]
fn test_classify_when_friday_data_viewed_on_weekend_should_be_fresh() {
    assert_eq!(
        classify(Some(d(2026, 10, 23)), d(2026, 10, 25)),
        Freshness::Fresh
    );
}

#[test]
fn test_freshness_as_str_should_use_pinned_attribute_values() {
    assert_eq!(Freshness::Fresh.as_str(), "fresh");
    assert_eq!(Freshness::Warn.as_str(), "warn");
    assert_eq!(Freshness::Stale.as_str(), "stale");
    assert_eq!(Freshness::None.as_str(), "none");
}

/// Locate the chip and return (opening tag, visible text).
fn chip(html: &str) -> (String, String) {
    let header_s = html.find("<header").expect("header");
    let header_e = html.find("</header>").expect("</header>");
    let header = &html[header_s..header_e];
    let tag = all_tags(header)
        .into_iter()
        .find(|t| t.contains("data-freshness="))
        .expect("data-freshness element inside <header>");
    assert!(
        attr(&tag, "class")
            .unwrap_or_default()
            .contains("freshness-chip"),
        "{tag}"
    );
    let text = element_text(header, &tag);
    (tag, text)
}

#[tokio::test]
async fn test_chip_when_screen_data_has_each_age_should_show_matching_state_date_and_age() {
    let today = today();
    let cases = [
        (0, "fresh", "Fresh"),
        (1, "fresh", "Fresh"),
        (2, "warn", "Aging"),
        (3, "warn", "Aging"),
        (4, "stale", "Stale"),
        (9, "stale", "Stale"),
    ];
    for (age, state, word) in cases {
        let as_of = date_with_age(today, age);
        assert_eq!(weekdays_between(as_of, today), age); // fixture self-check
        let app = app_with(|r| seed_screen_run(r, as_of));
        for page in ["/", "/regime"] {
            let html = get(&app, page).await.text();
            let (tag, text) = chip(&html);
            assert_eq!(
                attr(&tag, "data-freshness").as_deref(),
                Some(state),
                "age {age} on {page}"
            );
            assert!(text.contains("Screen"), "source label: {text}");
            assert!(text.contains(word), "state word {word} in {text}");
            assert!(text.contains(&as_of.to_string()), "as-of in {text}");
            let age_ok = text.contains(&format!("{age} trading day"))
                || (age == 0 && text.contains("today"));
            assert!(age_ok, "age text in {text}");
        }
    }
}

#[tokio::test]
async fn test_chip_when_no_screen_data_should_say_no_data() {
    let app = app();
    let html = get(&app, "/").await.text();
    let (tag, text) = chip(&html);
    assert_eq!(attr(&tag, "data-freshness").as_deref(), Some("none"));
    assert!(text.contains("No data"), "{text}");
    assert!(text.contains("Screen"), "{text}");
}

#[tokio::test]
async fn test_chip_when_any_page_rendered_should_appear_exactly_once_in_header() {
    let app = seeded_app();
    for p in page_routes() {
        let html = get(&app, &p).await.text();
        assert_eq!(count(&html, "data-freshness="), 1, "{p}");
        chip(&html);
    }
}

#[tokio::test]
async fn test_chip_when_newest_run_is_used_should_ignore_older_runs() {
    let today = today();
    let old = date_with_age(today, 9);
    let new = date_with_age(today, 1);
    let app = app_with(|r| {
        seed_screen_run(r, old);
        seed_screen_run(r, new);
    });
    let html = get(&app, "/").await.text();
    let (tag, text) = chip(&html);
    assert_eq!(attr(&tag, "data-freshness").as_deref(), Some("fresh"));
    assert!(text.contains(&new.to_string()) && !text.contains(&old.to_string()));
}

#[test]
fn test_css_when_loaded_should_style_every_freshness_state_with_tokens() {
    let css = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/web/static/ghost-alpha.css"
    ))
    .unwrap();
    for s in ["fresh", "warn", "stale", "none"] {
        assert!(
            css.contains(&format!("data-freshness=\"{s}\""))
                || css.contains(&format!("data-freshness={s}")),
            "no CSS rule for data-freshness={s}"
        );
    }
}
