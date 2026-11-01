//! RED-phase contract for `web::charts::tax_by_year_svg` (Track B, tax by calendar year).
//! This file fails to compile until the function and struct exist.
//!
//! Public API pinned here:
//!
//! ```ignore
//! // ghost_alpha::web::charts
//! #[derive(Debug, Clone, PartialEq)]
//! pub struct TaxYearBar {
//!     pub label: String,   // x-axis label, usually the calendar year ("2027")
//!     pub federal: f64,    // >= 0
//!     pub state: f64,      // >= 0
//!     pub niit: f64,       // >= 0
//! }
//! pub fn tax_by_year_svg(bars: &[TaxYearBar], width: u32, height: u32) -> String;
//! ```
//!
//! Rendering rules:
//! * One vertical bar per entry, in input order, stacked federal (bottom), state, NIIT (top).
//! * Segments are `<rect>` elements filled with `var(--ga-series-1)` (federal), `var(--ga-series-2)`
//!   (state) and `var(--ga-series-3)` (NIIT). No hex or rgb colours anywhere.
//! * Segment heights are proportional to the amount on a shared scale.
//! * Each bar's `label` appears as `<text>` on the x axis, XML-escaped.
//! * A legend with the TEXT labels `Federal`, `State`, `NIIT` (colour is never the only cue).
//! * Output is a complete `<svg ...>...</svg>` string with a viewBox of `0 0 {width} {height}`.
//!   All-zero or empty input still gives a valid svg (no NaN/inf); empty input need not draw bars.
//! * The caller wraps it with `accessible_with_id`; the function itself does not add role/title.

mod common;

use common::web::{attr, open_tags};
use ghost_alpha::web::charts::{tax_by_year_svg, TaxYearBar};

fn bar(label: &str, federal: f64, state: f64, niit: f64) -> TaxYearBar {
    TaxYearBar {
        label: label.to_string(),
        federal,
        state,
        niit,
    }
}

fn three_years() -> Vec<TaxYearBar> {
    vec![
        bar("2027", 10_000.0, 2_000.0, 500.0),
        bar("2028", 20_000.0, 4_000.0, 1_000.0),
        bar("2029", 30_000.0, 6_000.0, 1_500.0),
    ]
}

fn assert_valid_svg(svg: &str) {
    assert!(svg.starts_with("<svg"), "{svg}");
    assert!(svg.trim_end().ends_with("</svg>"), "{svg}");
    assert!(svg.contains("viewBox=\"0 0 640 320\""), "{svg}");
    for bad in ["NaN", "inf", "undefined"] {
        assert!(!svg.contains(bad), "{bad} in {svg}");
    }
}

/// Heights of the rects filled with `fill`, largest first.
fn heights(svg: &str, fill: &str) -> Vec<f64> {
    let mut v: Vec<f64> = open_tags(svg, "rect")
        .iter()
        .filter(|t| attr(t, "fill").as_deref() == Some(fill))
        .filter_map(|t| attr(t, "height")?.parse().ok())
        .collect();
    v.sort_by(|a, b| b.partial_cmp(a).unwrap());
    v
}

#[test]
fn test_tax_by_year_svg_should_be_a_complete_svg_with_viewbox() {
    assert_valid_svg(&tax_by_year_svg(&three_years(), 640, 320));
}

#[test]
fn test_tax_by_year_svg_should_fill_segments_with_the_three_series_tokens() {
    let svg = tax_by_year_svg(&three_years(), 640, 320);
    for n in 1..=3 {
        let fill = format!("var(--ga-series-{n})");
        assert!(
            heights(&svg, &fill).len() >= 3,
            "expected one {fill} rect per year in {svg}"
        );
    }
}

#[test]
fn test_tax_by_year_svg_should_use_no_hex_or_rgb_colours() {
    let svg = tax_by_year_svg(&three_years(), 640, 320);
    assert!(!svg.contains("=\"#"), "hex attribute in {svg}");
    assert!(!svg.contains(":#"), "hex style in {svg}");
    assert!(!svg.contains("rgb("), "rgb in {svg}");
}

#[test]
fn test_tax_by_year_svg_should_scale_segment_heights_with_the_amounts() {
    let svg = tax_by_year_svg(&three_years(), 640, 320);
    // Federal amounts are 10k / 20k / 30k, so the three tallest series-1 rects are in ratio 3:2:1
    // (the legend swatch, if any, is far smaller than the smallest bar).
    let h = heights(&svg, "var(--ga-series-1)");
    assert!(h.len() >= 3);
    assert!(h[2] > 5.0, "smallest bar should be visible: {h:?}");
    assert!((h[0] / h[2] - 3.0).abs() < 0.05, "{h:?}");
    assert!((h[1] / h[2] - 2.0).abs() < 0.05, "{h:?}");
    // State is 1/5 of federal in every year on the same scale.
    let s = heights(&svg, "var(--ga-series-2)");
    assert!((s[0] / h[0] - 0.2).abs() < 0.02, "{s:?} vs {h:?}");
}

#[test]
fn test_tax_by_year_svg_should_label_each_year_and_the_legend_with_text() {
    let svg = tax_by_year_svg(&three_years(), 640, 320);
    for t in ["2027", "2028", "2029", "Federal", "State", "NIIT"] {
        assert!(svg.contains(&format!(">{t}<")), "no text {t} in {svg}");
    }
}

#[test]
fn test_tax_by_year_svg_when_all_zero_should_be_valid() {
    let svg = tax_by_year_svg(
        &[bar("2027", 0.0, 0.0, 0.0), bar("2028", 0.0, 0.0, 0.0)],
        640,
        320,
    );
    assert_valid_svg(&svg);
    assert!(svg.contains(">2027<") && svg.contains(">2028<"));
    // Nothing owed: no visible segment.
    for n in 1..=3 {
        let h = heights(&svg, &format!("var(--ga-series-{n})"));
        assert!(
            h.iter().all(|x| *x <= 12.0),
            "only legend swatches allowed: {h:?}"
        );
    }
}

#[test]
fn test_tax_by_year_svg_when_empty_should_be_valid() {
    assert_valid_svg(&tax_by_year_svg(&[], 640, 320));
}

#[test]
fn test_tax_by_year_svg_when_single_year_should_be_valid() {
    let svg = tax_by_year_svg(&[bar("2027", 1_234.5, 0.0, 0.0)], 640, 320);
    assert_valid_svg(&svg);
    assert!(svg.contains(">2027<"));
}

#[test]
fn test_tax_by_year_svg_should_escape_labels() {
    let svg = tax_by_year_svg(&[bar("<b>\"x\"&", 100.0, 10.0, 1.0)], 640, 320);
    assert!(!svg.contains("<b>"), "{svg}");
    assert!(svg.contains("&lt;b&gt;"), "{svg}");
    assert!(svg.contains("&amp;"), "{svg}");
    assert_valid_svg(&svg);
}

#[test]
fn test_tax_by_year_svg_should_be_deterministic() {
    assert_eq!(
        tax_by_year_svg(&three_years(), 640, 320),
        tax_by_year_svg(&three_years(), 640, 320)
    );
}

// ───────────────────────────── round 2: geometry ─────────────────────────────

/// (y, height) of the rects filled with `fill`, in document order.
fn rects(svg: &str, fill: &str) -> Vec<(f64, f64)> {
    open_tags(svg, "rect")
        .iter()
        .filter(|t| attr(t, "fill").as_deref() == Some(fill))
        .filter_map(|t| {
            Some((
                attr(t, "y")?.parse().ok()?,
                attr(t, "height")?.parse().ok()?,
            ))
        })
        .collect()
}

/// Bar segments only: drop the (small) legend swatches by keeping the `n` tallest.
fn bar_segments(svg: &str, fill: &str, n: usize) -> Vec<(f64, f64)> {
    let mut all = rects(svg, fill);
    all.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    all.truncate(n);
    all
}

#[test]
fn test_tax_by_year_svg_segments_should_stack_flush_from_a_common_baseline() {
    let svg = tax_by_year_svg(&three_years(), 640, 320);
    let f = bar_segments(&svg, "var(--ga-series-1)", 3);
    let s = bar_segments(&svg, "var(--ga-series-2)", 3);
    let n = bar_segments(&svg, "var(--ga-series-3)", 3);
    assert_eq!((f.len(), s.len(), n.len()), (3, 3, 3));
    // Match segments of the same bar by size order (amounts grow with the year in this fixture).
    let by_h = |v: &[(f64, f64)]| {
        let mut v = v.to_vec();
        v.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        v
    };
    let (f, s, n) = (by_h(&f), by_h(&s), by_h(&n));
    let baseline = f[0].0 + f[0].1;
    for i in 0..3 {
        assert!(
            (f[i].0 + f[i].1 - baseline).abs() < 0.01,
            "federal must sit on the baseline"
        );
        assert!(
            (s[i].0 + s[i].1 - f[i].0).abs() < 0.01,
            "state must start where federal ends"
        );
        assert!(
            (n[i].0 + n[i].1 - s[i].0).abs() < 0.01,
            "NIIT must start where state ends"
        );
    }
}

#[test]
fn test_tax_by_year_svg_segments_should_carry_a_background_separator_stroke() {
    // Comparable component sizes: the smallest segment is ~22px tall, well above any legend swatch.
    let svg = tax_by_year_svg(
        &[
            bar("2027", 10_000.0, 8_000.0, 6_000.0),
            bar("2028", 20_000.0, 16_000.0, 12_000.0),
            bar("2029", 30_000.0, 24_000.0, 18_000.0),
        ],
        640,
        320,
    );
    for n in 1..=3 {
        let fill = format!("var(--ga-series-{n})");
        let seg: Vec<String> = open_tags(&svg, "rect")
            .into_iter()
            .filter(|t| attr(t, "fill").as_deref() == Some(fill.as_str()))
            .filter(|t| {
                attr(t, "height")
                    .and_then(|h| h.parse::<f64>().ok())
                    .is_some_and(|h| h > 12.0)
            })
            .collect();
        assert_eq!(seg.len(), 3, "{fill}");
        for t in seg {
            assert_eq!(attr(&t, "stroke").as_deref(), Some("var(--ga-bg)"), "{t}");
        }
    }
}

#[test]
fn test_tax_by_year_svg_when_all_values_are_under_a_dollar_should_not_fill_the_plot() {
    let full = tax_by_year_svg(&[bar("2027", 5.0, 0.0, 0.0)], 640, 320);
    let tiny = tax_by_year_svg(&[bar("2027", 0.3, 0.0, 0.0)], 640, 320);
    let hf = bar_segments(&full, "var(--ga-series-1)", 1)[0].1;
    let ht = bar_segments(&tiny, "var(--ga-series-1)", 1)[0].1;
    // The scale never drops below $1, so $0.30 is 30% of the plot height.
    assert!(ht < hf * 0.5, "tiny bar {ht} vs full {hf}");
    assert!((ht / hf - 0.3).abs() < 0.03, "tiny bar {ht} vs full {hf}");
}
