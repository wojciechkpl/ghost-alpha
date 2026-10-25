use crate::data::models::MarketRegime;
use std::f64::consts::PI;
use std::sync::atomic::{AtomicU64, Ordering};

/// Escape text for safe embedding in SVG/XML elements (text, title).
pub fn svg_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

static SVG_ID: AtomicU64 = AtomicU64::new(1);

/// Mark an inline SVG as decorative (hidden from assistive technology).
///
/// Use for small in-cell graphics whose value is also printed as text.
pub fn decorative(svg: &str) -> String {
    svg.replacen("<svg ", "<svg aria-hidden=\"true\" focusable=\"false\" ", 1)
}

/// Give an inline SVG an accessible name and description.
///
/// Adds `role="img"`, `aria-labelledby` and a `<title>`/`<desc>` pair with a
/// process-unique id, so several charts can share one response.
pub fn accessible(svg: &str, title: &str, desc: &str) -> String {
    debug_assert!(
        svg.trim_start().starts_with("<svg"),
        "accessible() expects an <svg> element"
    );
    let id = SVG_ID.fetch_add(1, Ordering::Relaxed);
    let Some(open_end) = svg.find('>') else {
        return svg.to_string();
    };
    let (open, rest) = svg.split_at(open_end + 1);
    let open = open.replacen(
        "<svg ",
        &format!(
            "<svg role=\"img\" aria-labelledby=\"ga-svg-{id}\" aria-describedby=\"ga-desc-{id}\" "
        ),
        1,
    );
    format!(
        "{open}<title id=\"ga-svg-{id}\">{}</title><desc id=\"ga-desc-{id}\">{}</desc>{rest}",
        svg_escape(title),
        svg_escape(desc)
    )
}

/// Render a horizontal score bar as an inline SVG string.
///
/// `score` is clamped to `[0, 100]`. The filled portion scales linearly.
pub fn score_bar_svg(score: f64, width: u32, height: u32) -> String {
    let clamped = score.clamp(0.0, 100.0);
    let fill_width = (clamped / 100.0) * width as f64;

    let color = if clamped >= 80.0 {
        "var(--ga-pos)"
    } else if clamped >= 50.0 {
        "var(--ga-heat-3)"
    } else {
        "var(--ga-neg)"
    };

    let bg = "var(--ga-border)";
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}"><rect x="0" y="0" width="{width}" height="{height}" fill="{bg}" rx="3"/><rect x="0" y="0" width="{fw:.1}" height="{height}" fill="{color}" rx="3"/></svg>"#,
        fw = fill_width,
    )
}

/// A single entry in a regime timeline.
pub struct RegimeEntry {
    pub regime: MarketRegime,
    pub days: u32,
}

/// Render a horizontal stacked-bar regime timeline as an inline SVG string.
///
/// Each entry occupies a proportional width of the total. Colors:
/// - Bull = green (`--ga-pos`)
/// - Bear = red (`--ga-neg`)
/// - Sideways = yellow (`--ga-heat-3`)
pub fn regime_timeline_svg(entries: &[RegimeEntry], width: u32, height: u32) -> String {
    if entries.is_empty() {
        return format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}"></svg>"#
        );
    }

    let total_days: u32 = entries.iter().map(|e| e.days).sum();
    if total_days == 0 {
        return format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}"></svg>"#
        );
    }

    let mut rects = String::new();
    let mut x: f64 = 0.0;

    for entry in entries {
        let w = (entry.days as f64 / total_days as f64) * width as f64;
        let color = match entry.regime {
            MarketRegime::Bull => "var(--ga-pos)",
            MarketRegime::Bear => "var(--ga-neg)",
            MarketRegime::Sideways => "var(--ga-heat-3)",
        };
        rects.push_str(&format!(
            r#"<rect x="{x:.1}" y="0" width="{w:.1}" height="{height}" fill="{color}"/>"#,
        ));
        x += w;
    }

    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">{rects}</svg>"#
    )
}

/// Render a sparkline as an inline SVG with a `<polyline>`.
///
/// The values are scaled to fit the `[0, height]` range. Edge cases:
/// - Empty slice produces an empty SVG shell.
/// - Single point produces a horizontal line at mid-height.
/// - All-same values produce a flat line at mid-height (no NaN).
pub fn sparkline_svg(values: &[f64], width: u32, height: u32) -> String {
    if values.is_empty() {
        return format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}"></svg>"#
        );
    }

    let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let range = max - min;

    let padding = 2.0_f64;
    let usable_height = (height as f64 - 2.0 * padding).max(1.0);

    let n = values.len();
    let x_step = if n > 1 {
        width as f64 / (n - 1) as f64
    } else {
        0.0
    };

    let points: String = values
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let x = x_step * i as f64;
            let y = if range.abs() < f64::EPSILON {
                // All values are the same: draw at mid-height
                padding + usable_height / 2.0
            } else {
                // Invert Y so higher values go up
                padding + usable_height - ((v - min) / range) * usable_height
            };
            format!("{x:.1},{y:.1}")
        })
        .collect::<Vec<_>>()
        .join(" ");

    let stroke = "var(--ga-series-1)";
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}"><polyline points="{points}" fill="none" stroke="{stroke}" stroke-width="1.5"/></svg>"#
    )
}

// ── Heatmap ─────────────────────────────────────────────────────────────────

/// A single cell in the heatmap grid.
pub struct HeatmapCell {
    pub value: f64,
    pub label: String,
}

/// Map a composite score (0-100) to a fill color.
pub fn score_color(score: f64) -> &'static str {
    if score >= 90.0 {
        "var(--ga-heat-5)"
    } else if score >= 70.0 {
        "var(--ga-heat-4)"
    } else if score >= 50.0 {
        "var(--ga-heat-3)"
    } else if score >= 30.0 {
        "var(--ga-heat-2)"
    } else {
        "var(--ga-heat-1)"
    }
}

/// Map a sentiment score to a fill color.
pub fn sentiment_color(score: f64) -> &'static str {
    if score >= 12.0 {
        "var(--ga-heat-5)"
    } else if score >= 9.0 {
        "var(--ga-heat-4)"
    } else if score >= 6.0 {
        "var(--ga-heat-neutral)"
    } else if score >= 3.0 {
        "var(--ga-heat-2)"
    } else {
        "var(--ga-heat-1)"
    }
}

/// Map a normalized sentiment score (0-100) to a diverging color scale.
/// 0 = very negative (red), 50 = neutral (gray), 100 = very positive (green).
pub fn sentiment_color_normalized(score: f64) -> &'static str {
    if score >= 80.0 {
        "var(--ga-heat-5)" // dark green - very positive
    } else if score >= 60.0 {
        "var(--ga-heat-4)" // light green - positive
    } else if score >= 40.0 {
        "var(--ga-heat-neutral)" // gray - neutral
    } else if score >= 20.0 {
        "var(--ga-heat-2)" // orange - negative
    } else {
        "var(--ga-heat-1)" // red - very negative
    }
}

/// Render a grid heatmap. `data` is row-major: `data[ticker_idx][date_idx]`.
/// `tickers` and `dates` provide labels for Y and X axes.
pub fn heatmap_svg(
    data: &[Vec<Option<HeatmapCell>>],
    tickers: &[&str],
    dates: &[&str],
    width: u32,
    height: u32,
    color_fn: fn(f64) -> &'static str,
) -> String {
    let left_margin: f64 = 60.0;
    let top_margin: f64 = 66.0;
    let right_margin: f64 = 40.0;

    if data.is_empty() || tickers.is_empty() || dates.is_empty() {
        return format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}"></svg>"#
        );
    }

    let num_rows = tickers.len();
    let num_cols = dates.len();
    let gap: f64 = 1.0;

    let avail_w = (width as f64 - left_margin - right_margin).max(1.0);
    let avail_h = (height as f64 - top_margin).max(1.0);
    let cell_w = (avail_w / num_cols as f64 - gap).max(1.0);
    let cell_h = (avail_h / num_rows as f64 - gap).max(1.0);

    let mut body = String::new();

    // Date labels (rotated 45deg, thinned to avoid overlap)
    let font10 = "11";
    let label_step = 1.max(num_cols / 6);
    for (ci, date) in dates.iter().enumerate() {
        if ci % label_step != 0 {
            continue;
        }
        let x = left_margin + ci as f64 * (cell_w + gap) + cell_w / 2.0;
        let y = top_margin - 4.0;
        body.push_str(&format!(
            r#"<text x="{x:.1}" y="{y:.1}" font-size="{font10}" text-anchor="start" transform="rotate(-45 {x:.1} {y:.1})">{}</text>"#,
            svg_escape(date)
        ));
    }

    // Ticker labels
    for (ri, ticker) in tickers.iter().enumerate() {
        let x = left_margin - 4.0;
        let y = top_margin + ri as f64 * (cell_h + gap) + cell_h / 2.0 + 4.0;
        body.push_str(&format!(
            r#"<text x="{x:.1}" y="{y:.1}" font-size="{font10}" text-anchor="end">{}</text>"#,
            svg_escape(ticker)
        ));
    }

    // Axis titles
    let date_title_x = left_margin + avail_w / 2.0;
    body.push_str(&format!(
        r#"<text class="axis-title chart-muted" x="{date_title_x:.1}" y="11" font-size="11" text-anchor="middle">Date</text>"#
    ));
    let ticker_title_y = top_margin + avail_h / 2.0;
    body.push_str(&format!(
        r#"<text class="axis-title chart-muted" x="11" y="{ticker_title_y:.1}" font-size="11" text-anchor="middle" transform="rotate(-90 11 {ticker_title_y:.1})">Ticker</text>"#
    ));

    // Cells
    let missing_color = "var(--ga-heat-missing)";
    for (ri, row) in data.iter().enumerate() {
        for ci in 0..num_cols {
            let rx = left_margin + ci as f64 * (cell_w + gap);
            let ry = top_margin + ri as f64 * (cell_h + gap);

            let (fill, tooltip) = match row.get(ci).and_then(|c| c.as_ref()) {
                Some(cell) => {
                    let v = if cell.value.is_nan() { 0.0 } else { cell.value };
                    (color_fn(v), svg_escape(&cell.label))
                }
                None => (missing_color, String::from("no data")),
            };

            body.push_str(&format!(
                r#"<rect x="{rx:.1}" y="{ry:.1}" width="{cw:.1}" height="{ch:.1}" fill="{fill}"><title>{tooltip}</title></rect>"#,
                cw = cell_w,
                ch = cell_h,
            ));
        }
    }

    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">{body}</svg>"#
    )
}

// ── Stacked Area ────────────────────────────────────────────────────────────

/// A single series in a stacked area chart.
pub struct AreaSeries {
    pub label: String,
    pub color: String,
    pub values: Vec<f64>,
}

/// Render a stacked area chart as an SVG string.
pub fn stacked_area_svg(series: &[AreaSeries], dates: &[&str], width: u32, height: u32) -> String {
    let ml: f64 = 40.0;
    let mr: f64 = 20.0;
    let mt: f64 = 10.0;
    let mb: f64 = 44.0;

    if series.is_empty() || dates.is_empty() {
        return format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}"></svg>"#
        );
    }

    let n = dates.len();
    let plot_w = (width as f64 - ml - mr).max(1.0);
    let plot_h = (height as f64 - mt - mb).max(1.0);

    // Compute cumulative stacks
    let mut cumulative: Vec<Vec<f64>> = Vec::with_capacity(series.len() + 1);
    cumulative.push(vec![0.0; n]);
    for s in series {
        let prev = cumulative.last().unwrap();
        let next: Vec<f64> = prev
            .iter()
            .enumerate()
            .map(|(i, &base)| {
                let v = s.values.get(i).copied().unwrap_or(0.0);
                let v = if v.is_nan() { 0.0 } else { v };
                base + v
            })
            .collect();
        cumulative.push(next);
    }

    let max_val = cumulative
        .last()
        .unwrap()
        .iter()
        .cloned()
        .fold(0.0_f64, f64::max)
        .max(1.0);

    let x_of = |i: usize| -> f64 {
        if n <= 1 {
            ml
        } else {
            ml + (i as f64 / (n - 1) as f64) * plot_w
        }
    };
    let y_of = |v: f64| -> f64 { mt + plot_h - (v / max_val) * plot_h };

    let mut body = String::new();

    // Draw each area from bottom to top
    let stroke_color = "var(--ga-bg)";
    for (si, s) in series.iter().enumerate() {
        let bottom = &cumulative[si];
        let top = &cumulative[si + 1];

        let mut d = String::new();
        // Forward along top edge
        for (i, &top_val) in top.iter().enumerate().take(n) {
            let x = x_of(i);
            let y = y_of(top_val);
            if i == 0 {
                d.push_str(&format!("M{x:.1},{y:.1}"));
            } else {
                d.push_str(&format!(" L{x:.1},{y:.1}"));
            }
        }
        // Backward along bottom edge
        for i in (0..n).rev() {
            let x = x_of(i);
            let y = y_of(bottom[i]);
            d.push_str(&format!(" L{x:.1},{y:.1}"));
        }
        d.push('Z');

        let fill = &s.color;
        body.push_str(&format!(
            r#"<path d="{d}" fill="{fill}" opacity="0.85" stroke="{stroke_color}" stroke-width="1"/>"#
        ));
    }

    // X-axis date labels (every 5th)
    let font10 = "11";
    let label_y = height as f64 - 20.0;
    for (i, date) in dates.iter().enumerate() {
        if i % 5 == 0 {
            let x = x_of(i);
            body.push_str(&format!(
                r#"<text x="{x:.1}" y="{label_y:.1}" font-size="{font10}" text-anchor="middle">{date}</text>"#
            ));
        }
    }
    let title_x = ml + plot_w / 2.0;
    let title_y = height as f64 - 5.0;
    body.push_str(&format!(
        r#"<text class="axis-title chart-muted" x="{title_x:.1}" y="{title_y:.1}" font-size="11" text-anchor="middle">Date</text>"#
    ));
    let y_title_y = mt + plot_h / 2.0;
    body.push_str(&format!(
        r#"<text class="axis-title chart-muted" x="12" y="{y_title_y:.1}" font-size="11" text-anchor="middle" transform="rotate(-90 12 {y_title_y:.1})">Count (stocks)</text>"#
    ));

    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">{body}</svg>"#
    )
}

// ── Scatter Plot ────────────────────────────────────────────────────────────

/// A single point on a scatter plot.
pub struct ScatterPoint {
    pub x: f64,
    pub y: f64,
    pub label: String,
    pub color: String,
    pub size: f64,
}

/// Render a scatter plot as an SVG string.
pub fn scatter_svg(
    points: &[ScatterPoint],
    x_label: &str,
    y_label: &str,
    width: u32,
    height: u32,
) -> String {
    let ml: f64 = 50.0;
    let mr: f64 = 20.0;
    let mt: f64 = 20.0;
    let mb: f64 = 40.0;
    let plot_w = (width as f64 - ml - mr).max(1.0);
    let plot_h = (height as f64 - mt - mb).max(1.0);

    if points.is_empty() {
        return format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}"></svg>"#
        );
    }

    // X fixed 0..100 for RS percentile
    let x_min = 0.0_f64;
    let x_max = 100.0_f64;

    // Y auto-scale, symmetric around 0 if data spans both sides
    let raw_y_min = points.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
    let raw_y_max = points.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
    let (y_min, y_max) = if raw_y_min >= 0.0 {
        (0.0, raw_y_max.max(1.0))
    } else if raw_y_max <= 0.0 {
        (raw_y_min.min(-1.0), 0.0)
    } else {
        let abs = raw_y_min.abs().max(raw_y_max.abs());
        (-abs, abs)
    };
    let y_range = (y_max - y_min).max(1.0);

    let px = |v: f64| -> f64 { ml + ((v - x_min) / (x_max - x_min)) * plot_w };
    let py = |v: f64| -> f64 { mt + plot_h - ((v - y_min) / y_range) * plot_h };

    let mut body = String::new();

    // Quadrant dividers
    let dash = "4,4";
    let gray = "var(--ga-muted)";
    let mid_x = px(50.0);
    let zero_y = py(0.0);
    body.push_str(&format!(
        r#"<line x1="{mid_x:.1}" y1="{mt:.1}" x2="{mid_x:.1}" y2="{y2:.1}" stroke="{gray}" stroke-dasharray="{dash}" stroke-width="1"/>"#,
        y2 = mt + plot_h,
    ));
    body.push_str(&format!(
        r#"<line x1="{ml:.1}" y1="{zero_y:.1}" x2="{x2:.1}" y2="{zero_y:.1}" stroke="{gray}" stroke-dasharray="{dash}" stroke-width="1"/>"#,
        x2 = ml + plot_w,
    ));

    // Y-axis grid lines and tick labels
    let num_y_ticks: usize = 5;
    for i in 0..=num_y_ticks {
        let frac = i as f64 / num_y_ticks as f64;
        let val = y_min + frac * y_range;
        let tick_y = py(val);
        let grid_color = "var(--ga-border)";
        body.push_str(&format!(
            r#"<line x1="{ml:.1}" y1="{tick_y:.1}" x2="{x2:.1}" y2="{tick_y:.1}" stroke="{grid_color}" stroke-width="0.5"/>"#,
            x2 = ml + plot_w,
        ));
        body.push_str(&format!(
            r#"<text x="{tx:.1}" y="{ty:.1}" font-size="11" text-anchor="end">{val:.1}</text>"#,
            tx = ml - 4.0,
            ty = tick_y + 3.0,
        ));
    }

    // X-axis tick labels at 0, 25, 50, 75, 100
    for &tick_val in &[0.0_f64, 25.0, 50.0, 75.0, 100.0] {
        let tick_x = px(tick_val);
        let tick_label_y = mt + plot_h + 14.0;
        body.push_str(&format!(
            r#"<text x="{tick_x:.1}" y="{tick_label_y:.1}" font-size="11" text-anchor="middle">{tick_val:.0}</text>"#,
        ));
    }

    // Points
    for p in points {
        let cx = px(p.x.clamp(x_min, x_max));
        let cy = py(p.y.clamp(y_min, y_max));
        let r = if p.size.is_nan() || p.size <= 0.0 {
            6.0
        } else {
            p.size
        };
        let fill = &p.color;
        let lab = svg_escape(&p.label);
        body.push_str(&format!(
            r#"<circle cx="{cx:.1}" cy="{cy:.1}" r="{r:.1}" fill="{fill}" opacity="0.8"><title>{lab}</title></circle>"#
        ));
    }

    // Top 5 by size get text labels
    let mut indices: Vec<usize> = (0..points.len()).collect();
    indices.sort_by(|&a, &b| {
        points[b]
            .size
            .partial_cmp(&points[a].size)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let font9 = "11";
    for &idx in indices.iter().take(5) {
        let p = &points[idx];
        let cx = px(p.x.clamp(x_min, x_max));
        let cy = py(p.y.clamp(y_min, y_max));
        let r = if p.size.is_nan() || p.size <= 0.0 {
            6.0
        } else {
            p.size
        };
        let lab = svg_escape(&p.label);
        let tx = cx + r + 2.0;
        let ty = cy - r;
        body.push_str(&format!(
            r#"<text x="{tx:.1}" y="{ty:.1}" font-size="{font9}">{lab}</text>"#
        ));
    }

    // Axis labels
    let x_lab_x = ml + plot_w / 2.0;
    let x_lab_y = height as f64 - 5.0;
    let y_lab_x = 12.0;
    let y_lab_y = mt + plot_h / 2.0;
    body.push_str(&format!(
        r#"<text x="{x_lab_x:.1}" y="{x_lab_y:.1}" font-size="11" text-anchor="middle" class="axis-title chart-muted">{x_label}</text>"#
    ));
    body.push_str(&format!(
        r#"<text x="{y_lab_x:.1}" y="{y_lab_y:.1}" font-size="11" text-anchor="middle" class="axis-title chart-muted" transform="rotate(-90 {y_lab_x:.1} {y_lab_y:.1})">{y_label}</text>"#
    ));

    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">{body}</svg>"#
    )
}

// ── Calendar Heatmap ────────────────────────────────────────────────────────

/// A single day in the calendar heatmap.
pub struct CalendarDay {
    pub date: String,
    pub weekday: u8,
    pub count: u32,
    pub has_critical: bool,
}

/// Render a calendar heatmap of alert activity as an SVG string.
pub fn calendar_heatmap_svg(days: &[CalendarDay], width: u32, height: u32) -> String {
    if days.is_empty() {
        return format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}"></svg>"#
        );
    }

    let cell: f64 = 14.0;
    let gap: f64 = 2.0;
    let left_margin: f64 = 20.0;
    let top_margin: f64 = 4.0;

    let num_cols = days.len().div_ceil(5);

    let mut body = String::new();

    // Weekday labels
    let labels = ["M", "T", "W", "T", "F"];
    let font10 = "11";
    for (r, lab) in labels.iter().enumerate() {
        let y = top_margin + r as f64 * (cell + gap) + cell / 2.0 + 4.0;
        body.push_str(&format!(
            r#"<text x="2" y="{y:.1}" font-size="{font10}">{lab}</text>"#
        ));
    }

    // Cells
    let critical_color = "var(--ga-neg)";
    for (i, day) in days.iter().enumerate() {
        let row = day.weekday.min(4) as f64;
        let col = (i / 5) as f64;

        let rx = left_margin + col * (cell + gap);
        let ry = top_margin + row * (cell + gap);

        let fill = match day.count {
            0 => "var(--ga-cal-0)",
            1..=2 => "var(--ga-cal-1)",
            3..=5 => "var(--ga-cal-2)",
            _ => "var(--ga-cal-3)",
        };

        let tooltip = if day.has_critical {
            svg_escape(&format!("{}: {} alerts (critical)", day.date, day.count))
        } else {
            svg_escape(&format!("{}: {} alerts", day.date, day.count))
        };

        body.push_str(&format!(
            r#"<rect x="{rx:.1}" y="{ry:.1}" width="{cell}" height="{cell}" fill="{fill}" rx="2"><title>{tooltip}</title></rect>"#
        ));

        if day.has_critical {
            let cx = rx + cell - 3.0;
            let cy = ry + 3.0;
            body.push_str(&format!(
                r#"<circle cx="{cx:.1}" cy="{cy:.1}" r="3" fill="{critical_color}"/>"#
            ));
        }
    }

    // Use computed size for viewBox so the grid is not clipped
    let vb_w = (left_margin + num_cols as f64 * (cell + gap)).max(width as f64);
    let vb_h = (top_margin + 5.0 * (cell + gap)).max(height as f64);
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {vbw:.0} {vbh:.0}">{body}</svg>"#,
        vbw = vb_w,
        vbh = vb_h,
    )
}

// ── Gauge ───────────────────────────────────────────────────────────────────

/// Compute a point on a circle at the given angle (degrees).
fn arc_point(cx: f64, cy: f64, r: f64, angle_deg: f64) -> (f64, f64) {
    let rad = angle_deg * PI / 180.0;
    (cx + r * rad.cos(), cy + r * rad.sin())
}

/// Render a gauge arc as an SVG string (120x120).
pub fn gauge_svg(value: f64, min: f64, max: f64, label: &str, risk_color: &str) -> String {
    let w: f64 = 120.0;
    let h: f64 = 120.0;
    let cx = w / 2.0;
    let cy = h / 2.0;
    let r: f64 = 45.0;
    let stroke_w: f64 = 10.0;

    let start_angle = 150.0;
    let end_angle = 390.0;
    let arc_span = end_angle - start_angle; // 240 degrees

    // Clamp and normalize value
    let range = (max - min).abs();
    let fraction = if range < f64::EPSILON || value.is_nan() {
        0.0
    } else {
        ((value - min) / range).clamp(0.0, 1.0)
    };
    let value_angle = start_angle + fraction * arc_span;

    // Build SVG arc path helper
    let arc_path = |from_deg: f64, to_deg: f64| -> String {
        let (x1, y1) = arc_point(cx, cy, r, from_deg);
        let (x2, y2) = arc_point(cx, cy, r, to_deg);
        let sweep_deg = to_deg - from_deg;
        let large_arc = if sweep_deg.abs() > 180.0 { 1 } else { 0 };
        format!("M{x1:.1},{y1:.1} A{r},{r} 0 {large_arc} 1 {x2:.1},{y2:.1}")
    };

    let bg_color = "var(--ga-border)";
    let bg_path = arc_path(start_angle, end_angle);
    let mut body = format!(
        r#"<path d="{bg_path}" fill="none" stroke="{bg_color}" stroke-width="{stroke_w}" stroke-linecap="round"/>"#
    );

    if fraction > 0.001 {
        let val_path = arc_path(start_angle, value_angle);
        body.push_str(&format!(
            r#"<path d="{val_path}" fill="none" stroke="{risk_color}" stroke-width="{stroke_w}" stroke-linecap="round"/>"#
        ));
    }

    // Center text: formatted value
    let display_val = if value.is_nan() {
        String::from("--")
    } else {
        format!("{value:.1}")
    };
    let text_y = cy + 5.0;
    body.push_str(&format!(
        r#"<text x="{cx}" y="{text_y:.1}" font-size="16" font-weight="bold" text-anchor="middle">{display_val}</text>"#
    ));

    // Label
    let label_y = cy + 22.0;
    body.push_str(&format!(
        r#"<text class="chart-muted" x="{cx}" y="{label_y:.1}" font-size="11" text-anchor="middle">{label}</text>"#
    ));

    let wi = w as u32;
    let hi = h as u32;
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{wi}" height="{hi}" viewBox="0 0 {w} {h}">{body}</svg>"#
    )
}

// ── Donut Chart ──────────────────────────────────────────────────────────────

/// A single slice in a donut chart.
pub struct DonutSlice {
    pub label: String,
    pub value: f64,
    pub color: String,
}

/// Render a donut (ring) chart as an SVG string.
///
/// Each slice occupies a proportional arc. Labels with percentages are shown
/// outside the donut for slices > 5%. The center shows the total count.
/// Edge cases: empty slices produce an empty SVG, single slice draws a full
/// circle, all-zero values produce an empty SVG.
pub fn donut_svg(slices: &[DonutSlice], width: u32, height: u32) -> String {
    let valid: Vec<&DonutSlice> = slices
        .iter()
        .filter(|s| s.value.is_finite() && s.value > 0.0)
        .collect();

    let total: f64 = valid.iter().map(|s| s.value).sum();

    if valid.is_empty() || total <= 0.0 {
        return format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}"></svg>"#
        );
    }

    let cx = width as f64 / 2.0;
    let cy = height as f64 / 2.0;
    let outer_r = (width.min(height) as f64 / 2.0 - 30.0).max(20.0);
    let inner_r = outer_r * 0.55;
    let label_r = outer_r + 16.0;

    let mut body = String::new();

    // Single slice: draw a full circle ring
    if valid.len() == 1 {
        let s = valid[0];
        let fill = &s.color;
        body.push_str(&format!(
            r#"<circle cx="{cx:.1}" cy="{cy:.1}" r="{or:.1}" fill="{fill}"/>"#,
            or = outer_r,
        ));
        body.push_str(&format!(
            r#"<circle cx="{cx:.1}" cy="{cy:.1}" r="{ir:.1}" fill="var(--ga-bg)"/>"#,
            ir = inner_r,
        ));
        // Label
        let lab = &s.label;
        body.push_str(&format!(
            r#"<text x="{cx:.1}" y="{ly:.1}" font-size="11" text-anchor="middle">{lab} 100%</text>"#,
            ly = cy - outer_r - 6.0,
        ));
    } else {
        let mut start_angle: f64 = -PI / 2.0; // 12 o'clock

        for s in &valid {
            let fraction = s.value / total;
            let sweep = fraction * 2.0 * PI;
            let end_angle = start_angle + sweep;

            let large_arc = if sweep > PI { 1 } else { 0 };

            let (ox1, oy1) = (
                cx + outer_r * start_angle.cos(),
                cy + outer_r * start_angle.sin(),
            );
            let (ox2, oy2) = (
                cx + outer_r * end_angle.cos(),
                cy + outer_r * end_angle.sin(),
            );
            let (ix2, iy2) = (
                cx + inner_r * end_angle.cos(),
                cy + inner_r * end_angle.sin(),
            );
            let (ix1, iy1) = (
                cx + inner_r * start_angle.cos(),
                cy + inner_r * start_angle.sin(),
            );

            let fill = &s.color;
            body.push_str(&format!(
                r#"<path d="M{ox1:.1},{oy1:.1} A{or:.1},{or:.1} 0 {large_arc} 1 {ox2:.1},{oy2:.1} L{ix2:.1},{iy2:.1} A{ir:.1},{ir:.1} 0 {large_arc} 0 {ix1:.1},{iy1:.1} Z" fill="{fill}"/>"#,
                or = outer_r,
                ir = inner_r,
            ));

            // Label for slices > 5%
            let pct = fraction * 100.0;
            if pct > 5.0 {
                let mid_angle = start_angle + sweep / 2.0;
                let lx = cx + label_r * mid_angle.cos();
                let ly = cy + label_r * mid_angle.sin();
                let anchor = if lx > cx { "start" } else { "end" };
                let lab = &s.label;
                body.push_str(&format!(
                    r#"<text x="{lx:.1}" y="{ly:.1}" font-size="11" text-anchor="{anchor}">{lab} {pct:.0}%</text>"#,
                ));
            }

            start_angle = end_angle;
        }
    }

    // Center text: total count
    let total_display = if total >= 1_000_000.0 {
        format!("{:.1}M", total / 1_000_000.0)
    } else if total >= 1_000.0 {
        format!("{:.0}K", total / 1_000.0)
    } else {
        format!("{total:.0}")
    };
    body.push_str(&format!(
        r#"<text x="{cx:.1}" y="{cy1:.1}" font-size="16" font-weight="bold" text-anchor="middle">{total_display}</text>"#,
        cy1 = cy + 5.0,
    ));

    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">{body}</svg>"#
    )
}

// ── Correlation Matrix ───────────────────────────────────────────────────────

/// Mix percentage of the end colour for |r|.
///
/// Weak cells (|r| < 0.5 as displayed) use 0-50% so `--ga-text` stays readable;
/// strong cells jump to 75-100% so `--ga-on-strong` is readable. This keeps every
/// cell out of the mid-luminance band where neither text colour reaches 4.5:1
/// (verified for all four theme blocks in `tests/web_tokens.rs`).
pub fn correlation_mix_pct(abs_r: f64) -> f64 {
    let a = abs_r.clamp(0.0, 1.0);
    if (a * 100.0).round() >= 50.0 {
        (50.0 + a * 50.0).round()
    } else {
        (a * 100.0).round()
    }
}

/// Interpolate a hex color for a correlation value in [-1, +1].
///
/// `--ga-corr-neg` at -1 `--ga-corr-zero` at 0, `--ga-corr-pos` at +1; the
/// result is a CSS `color-mix()` expression so it follows the active theme.
/// Strength maps through [`correlation_mix_pct`].
pub fn correlation_color(value: f64) -> String {
    let v = if value.is_finite() {
        value.clamp(-1.0, 1.0)
    } else {
        0.0
    };
    let end = if v < 0.0 {
        "var(--ga-corr-neg)"
    } else {
        "var(--ga-corr-pos)"
    };
    let pct = correlation_mix_pct(v.abs());
    format!("color-mix(in srgb, {end} {pct:.0}%, var(--ga-corr-zero))")
}

/// Render a correlation matrix heatmap as an SVG string.
///
/// `tickers` provides the axis labels. `matrix` is row-major NxN with
/// values in [-1, +1]. Diagonal cells are highlighted with a stroke.
pub fn correlation_matrix_svg(
    tickers: &[&str],
    matrix: &[Vec<f64>],
    width: u32,
    height: u32,
) -> String {
    let n = tickers.len();
    if n == 0 || matrix.is_empty() {
        return format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}"></svg>"#
        );
    }

    let left_margin: f64 = 60.0;
    let top_margin: f64 = 60.0;
    let avail_w = (width as f64 - left_margin - 30.0).max(1.0);
    let avail_h = (height as f64 - top_margin - 10.0).max(1.0);
    let cell_w = avail_w / n as f64;
    let cell_h = avail_h / n as f64;

    let mut body = String::new();

    // Top ticker labels (rotated)
    for (ci, ticker) in tickers.iter().enumerate() {
        let x = left_margin + ci as f64 * cell_w + cell_w / 2.0;
        let y = top_margin - 6.0;
        body.push_str(&format!(
            r#"<text x="{x:.1}" y="{y:.1}" font-size="11" text-anchor="start" transform="rotate(-45 {x:.1} {y:.1})">{ticker}</text>"#
        ));
    }

    // Left ticker labels
    for (ri, ticker) in tickers.iter().enumerate() {
        let x = left_margin - 4.0;
        let y = top_margin + ri as f64 * cell_h + cell_h / 2.0 + 4.0;
        body.push_str(&format!(
            r#"<text x="{x:.1}" y="{y:.1}" font-size="11" text-anchor="end">{ticker}</text>"#
        ));
    }

    // Cells
    for (ri, row) in matrix.iter().enumerate() {
        for ci in 0..n {
            let val = row.get(ci).copied().unwrap_or(0.0);
            let val = if val.is_finite() {
                val.clamp(-1.0, 1.0)
            } else {
                0.0
            };
            let fill = correlation_color(val);
            let rx = left_margin + ci as f64 * cell_w;
            let ry = top_margin + ri as f64 * cell_h;

            let diag_stroke = r#"stroke="var(--ga-text)" stroke-width="2""#;
            let stroke_attr = if ri == ci {
                format!(" {diag_stroke}")
            } else {
                String::new()
            };

            body.push_str(&format!(
                r#"<rect x="{rx:.1}" y="{ry:.1}" width="{cw:.1}" height="{ch:.1}" fill="{fill}"{stroke_attr}/>"#,
                cw = cell_w,
                ch = cell_h,
            ));

            // Value text
            let tx = rx + cell_w / 2.0;
            let ty = ry + cell_h / 2.0 + 4.0;
            // Cells at |r| >= 0.5 (as displayed) are saturated: use the on-strong text colour.
            let class = if (val.abs() * 100.0).round() >= 50.0 {
                r#" class="on-strong""#
            } else {
                ""
            };
            body.push_str(&format!(
                r#"<text x="{tx:.1}" y="{ty:.1}" font-size="11" text-anchor="middle"{class}>{val:.2}</text>"#,
            ));
        }
    }

    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">{body}</svg>"#
    )
}

// ── Efficient Frontier ───────────────────────────────────────────────────────

/// A point on the efficient frontier chart.
pub struct FrontierPoint {
    pub risk: f64,
    pub ret: f64,
    pub label: String,
    pub color: String,
    pub highlighted: bool,
}

/// Render an efficient frontier chart as an SVG string.
///
/// Draws a line through the points (the frontier curve) with labeled dots for
/// key portfolios. Highlighted points get a larger radius and text labels.
pub fn frontier_svg(
    points: &[FrontierPoint],
    current: Option<FrontierPoint>,
    width: u32,
    height: u32,
) -> String {
    let ml: f64 = 60.0;
    let mr: f64 = 20.0;
    let mt: f64 = 20.0;
    let mb: f64 = 40.0;
    let plot_w = (width as f64 - ml - mr).max(1.0);
    let plot_h = (height as f64 - mt - mb).max(1.0);

    let all_points: Vec<&FrontierPoint> = points.iter().chain(current.as_ref()).collect();

    if all_points.is_empty() {
        return format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}"></svg>"#
        );
    }

    let sanitize = |v: f64| -> f64 {
        if v.is_finite() {
            v
        } else {
            0.0
        }
    };

    let x_vals: Vec<f64> = all_points.iter().map(|p| sanitize(p.risk)).collect();
    let y_vals: Vec<f64> = all_points.iter().map(|p| sanitize(p.ret)).collect();

    let x_min = x_vals.iter().cloned().fold(f64::INFINITY, f64::min);
    let x_max = x_vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let y_min = y_vals.iter().cloned().fold(f64::INFINITY, f64::min);
    let y_max = y_vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    let x_range = (x_max - x_min).max(1.0);
    let y_range = (y_max - y_min).max(1.0);

    // Add 10% padding
    let x_lo = x_min - x_range * 0.1;
    let x_hi = x_max + x_range * 0.1;
    let y_lo = y_min - y_range * 0.1;
    let y_hi = y_max + y_range * 0.1;
    let x_span = (x_hi - x_lo).max(1.0);
    let y_span = (y_hi - y_lo).max(1.0);

    let px = |v: f64| -> f64 { ml + ((v - x_lo) / x_span) * plot_w };
    let py = |v: f64| -> f64 { mt + plot_h - ((v - y_lo) / y_span) * plot_h };

    let mut body = String::new();

    // Grid lines (4 horizontal, 4 vertical)
    let grid_color = "var(--ga-border)";
    for i in 0..=4 {
        let frac = i as f64 / 4.0;
        let gx = ml + frac * plot_w;
        let gy = mt + frac * plot_h;
        body.push_str(&format!(
            r#"<line x1="{gx:.1}" y1="{mt:.1}" x2="{gx:.1}" y2="{y2:.1}" stroke="{grid_color}" stroke-width="0.5"/>"#,
            y2 = mt + plot_h,
        ));
        body.push_str(&format!(
            r#"<line x1="{ml:.1}" y1="{gy:.1}" x2="{x2:.1}" y2="{gy:.1}" stroke="{grid_color}" stroke-width="0.5"/>"#,
            x2 = ml + plot_w,
        ));
    }

    // Frontier curve (line through points in order)
    if points.len() > 1 {
        let line_points: String = points
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let x = px(sanitize(p.risk));
                let y = py(sanitize(p.ret));
                if i == 0 {
                    format!("M{x:.1},{y:.1}")
                } else {
                    format!(" L{x:.1},{y:.1}")
                }
            })
            .collect();
        let line_color = "var(--ga-series-1)";
        body.push_str(&format!(
            r#"<path d="{line_points}" fill="none" stroke="{line_color}" stroke-width="2"/>"#
        ));
    }

    // Points
    for p in &all_points {
        let x = px(sanitize(p.risk));
        let y = py(sanitize(p.ret));
        let r = if p.highlighted { 6.0 } else { 4.0 };
        let fill = &p.color;
        let lab = &p.label;
        body.push_str(&format!(
            r#"<circle cx="{x:.1}" cy="{y:.1}" r="{r}" fill="{fill}"><title>{lab}</title></circle>"#
        ));

        if p.highlighted {
            let tx = x + 8.0;
            let ty = y - 8.0;
            body.push_str(&format!(
                r#"<text x="{tx:.1}" y="{ty:.1}" font-size="11" font-weight="bold">{lab}</text>"#,
            ));
        }
    }

    // Axis labels
    let x_label = "Risk (%)";
    let y_label = "Return (%)";
    let x_lab_x = ml + plot_w / 2.0;
    let x_lab_y = height as f64 - 5.0;
    let y_lab_x = 12.0;
    let y_lab_y = mt + plot_h / 2.0;
    body.push_str(&format!(
        r#"<text x="{x_lab_x:.1}" y="{x_lab_y:.1}" font-size="11" text-anchor="middle" class="axis-title chart-muted">{x_label}</text>"#
    ));
    body.push_str(&format!(
        r#"<text x="{y_lab_x:.1}" y="{y_lab_y:.1}" font-size="11" text-anchor="middle" class="axis-title chart-muted" transform="rotate(-90 {y_lab_x:.1} {y_lab_y:.1})">{y_label}</text>"#
    ));

    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">{body}</svg>"#
    )
}

// ── Fan Chart ────────────────────────────────────────────────────────────────

/// A single year in a fan chart (wealth projection).
pub struct FanChartYear {
    pub year: u32,
    pub p10: f64,
    pub p25: f64,
    pub p50: f64,
    pub p75: f64,
    pub p90: f64,
}

/// Format a dollar amount with $K / $M suffixes.
fn format_dollars(v: f64) -> String {
    let abs = v.abs();
    if abs >= 1_000_000.0 {
        format!("${:.1}M", v / 1_000_000.0)
    } else if abs >= 1_000.0 {
        format!("${:.0}K", v / 1_000.0)
    } else {
        format!("${:.0}", v)
    }
}

/// Render a fan chart (wealth projection) as an SVG string.
///
/// Draws graduated area bands: p10-p90 (lightest), p25-p75 (medium), and
/// p50 as a solid dark line. Uses graduated opacity of `--ga-series-1`.
pub fn fan_chart_svg(percentiles: &[FanChartYear], width: u32, height: u32) -> String {
    let ml: f64 = 70.0;
    let mr: f64 = 20.0;
    let mt: f64 = 20.0;
    let mb: f64 = 30.0;

    if percentiles.is_empty() {
        return format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}"></svg>"#
        );
    }

    let plot_w = (width as f64 - ml - mr).max(1.0);
    let plot_h = (height as f64 - mt - mb).max(1.0);
    let n = percentiles.len();

    // Compute data ranges
    let safe = |v: f64| -> f64 {
        if v.is_finite() {
            v
        } else {
            0.0
        }
    };

    let y_min = percentiles
        .iter()
        .map(|p| {
            [p.p10, p.p25, p.p50, p.p75, p.p90]
                .iter()
                .cloned()
                .map(safe)
                .fold(f64::INFINITY, f64::min)
        })
        .fold(f64::INFINITY, f64::min);
    let y_max = percentiles
        .iter()
        .map(|p| {
            [p.p10, p.p25, p.p50, p.p75, p.p90]
                .iter()
                .cloned()
                .map(safe)
                .fold(f64::NEG_INFINITY, f64::max)
        })
        .fold(f64::NEG_INFINITY, f64::max);

    let y_range = (y_max - y_min).max(1.0);
    let y_lo = y_min - y_range * 0.05;
    let y_hi = y_max + y_range * 0.05;
    let y_span = (y_hi - y_lo).max(1.0);

    let x_of = |i: usize| -> f64 {
        if n <= 1 {
            ml
        } else {
            ml + (i as f64 / (n - 1) as f64) * plot_w
        }
    };
    let y_of = |v: f64| -> f64 {
        let v = safe(v);
        mt + plot_h - ((v - y_lo) / y_span) * plot_h
    };

    let mut body = String::new();
    let blue = "var(--ga-series-1)";

    // Helper to build an area band path between upper and lower series
    let band_path = |upper: &dyn Fn(usize) -> f64, lower: &dyn Fn(usize) -> f64| -> String {
        let mut d = String::new();
        for i in 0..n {
            let x = x_of(i);
            let y = y_of(upper(i));
            if i == 0 {
                d.push_str(&format!("M{x:.1},{y:.1}"));
            } else {
                d.push_str(&format!(" L{x:.1},{y:.1}"));
            }
        }
        for i in (0..n).rev() {
            let x = x_of(i);
            let y = y_of(lower(i));
            d.push_str(&format!(" L{x:.1},{y:.1}"));
        }
        d.push('Z');
        d
    };

    // P10-P90 band (lightest)
    let d_outer = band_path(&|i| percentiles[i].p90, &|i| percentiles[i].p10);
    body.push_str(&format!(
        r#"<path d="{d_outer}" fill="{blue}" opacity="0.15"/>"#
    ));

    // P25-P75 band (medium)
    let d_inner = band_path(&|i| percentiles[i].p75, &|i| percentiles[i].p25);
    body.push_str(&format!(
        r#"<path d="{d_inner}" fill="{blue}" opacity="0.3"/>"#
    ));

    // P50 median line
    let mut median_d = String::new();
    for (i, pct) in percentiles.iter().enumerate().take(n) {
        let x = x_of(i);
        let y = y_of(pct.p50);
        if i == 0 {
            median_d.push_str(&format!("M{x:.1},{y:.1}"));
        } else {
            median_d.push_str(&format!(" L{x:.1},{y:.1}"));
        }
    }
    body.push_str(&format!(
        r#"<path d="{median_d}" fill="none" stroke="{blue}" stroke-width="2"/>"#
    ));

    // Y-axis labels (5 ticks)
    for i in 0..=4 {
        let frac = i as f64 / 4.0;
        let val = y_lo + frac * y_span;
        let y = y_of(val);
        let label = format_dollars(val);
        body.push_str(&format!(
            r#"<text x="{x:.1}" y="{y:.1}" font-size="11" text-anchor="end">{label}</text>"#,
            x = ml - 4.0,
        ));
    }

    // X-axis year labels (thin to ~6)
    let label_step = 1.max(n / 6);
    for (i, p) in percentiles.iter().enumerate() {
        if i % label_step == 0 || i == n - 1 {
            let x = x_of(i);
            let y = height as f64 - 5.0;
            body.push_str(&format!(
                r#"<text x="{x:.1}" y="{y:.1}" font-size="11" text-anchor="middle">{yr}</text>"#,
                yr = p.year,
            ));
        }
    }

    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">{body}</svg>"#
    )
}

// ── Horizontal Bar Chart ─────────────────────────────────────────────────────

/// A single bar in a horizontal bar chart.
pub struct HBar {
    pub label: String,
    pub value: f64,
    pub color: String,
}

/// Render a horizontal bar chart as an SVG string.
///
/// Bars are sorted by value (largest on top) with value labels at the end.
pub fn horizontal_bar_svg(bars: &[HBar], width: u32, height: u32) -> String {
    let valid: Vec<&HBar> = bars.iter().filter(|b| b.value.is_finite()).collect();

    if valid.is_empty() {
        return format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}"></svg>"#
        );
    }

    let mut sorted: Vec<&HBar> = valid;
    sorted.sort_by(|a, b| {
        b.value
            .partial_cmp(&a.value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let left_margin: f64 = 80.0;
    let right_margin: f64 = 60.0;
    let top_margin: f64 = 10.0;
    let bottom_margin: f64 = 10.0;
    let n = sorted.len();
    let avail_w = (width as f64 - left_margin - right_margin).max(1.0);
    let avail_h = (height as f64 - top_margin - bottom_margin).max(1.0);
    let bar_h = (avail_h / n as f64 - 4.0).max(4.0);
    let gap = 4.0;

    let max_val = sorted
        .iter()
        .map(|b| b.value.abs())
        .fold(0.0_f64, f64::max)
        .max(1.0);

    let mut body = String::new();

    for (i, bar) in sorted.iter().enumerate() {
        let y = top_margin + i as f64 * (bar_h + gap);
        let bw = (bar.value.abs() / max_val) * avail_w;
        let fill = &bar.color;
        let lab = &bar.label;

        // Label on the left
        body.push_str(&format!(
            r#"<text x="{lx:.1}" y="{ty:.1}" font-size="11" text-anchor="end">{lab}</text>"#,
            lx = left_margin - 4.0,
            ty = y + bar_h / 2.0 + 4.0,
        ));

        // Bar rect
        body.push_str(&format!(
            r#"<rect x="{lm:.1}" y="{y:.1}" width="{bw:.1}" height="{bh:.1}" fill="{fill}" rx="2"/>"#,
            lm = left_margin,
            bh = bar_h,
        ));

        // Value label
        body.push_str(&format!(
            r#"<text x="{vx:.1}" y="{ty:.1}" font-size="11">{val:.1}</text>"#,
            vx = left_margin + bw + 4.0,
            ty = y + bar_h / 2.0 + 4.0,
            val = bar.value,
        ));
    }

    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">{body}</svg>"#
    )
}

// ── Timeline Bar Chart ───────────────────────────────────────────────────────

/// A single period in a timeline bar chart (target vs executed).
pub struct TimelinePeriod {
    pub label: String,
    pub target: f64,
    pub executed: f64,
    pub color: String,
}

/// Render a paired horizontal bar chart showing target vs executed per period.
///
/// Target bars are gray, executed bars are overlaid in green. A completion
/// percentage is shown at the end of each pair.
pub fn timeline_bar_svg(periods: &[TimelinePeriod], width: u32, height: u32) -> String {
    let valid: Vec<&TimelinePeriod> = periods
        .iter()
        .filter(|p| p.target.is_finite() && p.executed.is_finite())
        .collect();

    if valid.is_empty() {
        return format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}"></svg>"#
        );
    }

    let left_margin: f64 = 80.0;
    let right_margin: f64 = 60.0;
    let top_margin: f64 = 10.0;
    let n = valid.len();
    let avail_w = (width as f64 - left_margin - right_margin).max(1.0);
    let avail_h = (height as f64 - top_margin - 10.0).max(1.0);
    let bar_h = (avail_h / n as f64 - 6.0).max(4.0);
    let gap = 6.0;

    let max_val = valid
        .iter()
        .map(|p| p.target.abs().max(p.executed.abs()))
        .fold(0.0_f64, f64::max)
        .max(1.0);

    let mut body = String::new();
    let target_color = "var(--ga-border)";
    let executed_color = "var(--ga-pos)";

    for (i, period) in valid.iter().enumerate() {
        let y = top_margin + i as f64 * (bar_h + gap);
        let tw = (period.target.abs() / max_val) * avail_w;
        let ew = (period.executed.abs() / max_val) * avail_w;
        let lab = &period.label;

        // Period label
        body.push_str(&format!(
            r#"<text x="{lx:.1}" y="{ty:.1}" font-size="11" text-anchor="end">{lab}</text>"#,
            lx = left_margin - 4.0,
            ty = y + bar_h / 2.0 + 4.0,
        ));

        // Target bar (gray background)
        body.push_str(&format!(
            r#"<rect x="{lm:.1}" y="{y:.1}" width="{tw:.1}" height="{bh:.1}" fill="{target_color}" rx="2"/>"#,
            lm = left_margin,
            bh = bar_h,
        ));

        // Executed bar (green overlay)
        body.push_str(&format!(
            r#"<rect x="{lm:.1}" y="{y:.1}" width="{ew:.1}" height="{bh:.1}" fill="{executed_color}" rx="2"/>"#,
            lm = left_margin,
            bh = bar_h,
        ));

        // Completion percentage
        let pct = if period.target.abs() > f64::EPSILON {
            (period.executed / period.target * 100.0).clamp(0.0, 999.0)
        } else {
            0.0
        };
        let bar_end = left_margin + tw.max(ew);
        body.push_str(&format!(
            r#"<text x="{vx:.1}" y="{ty:.1}" font-size="11">{pct:.0}%</text>"#,
            vx = bar_end + 4.0,
            ty = y + bar_h / 2.0 + 4.0,
        ));
    }

    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">{body}</svg>"#
    )
}

// ─── Concentration Decay Chart (Item 8) ─────────────────────────────────────

use crate::portfolio::selling::SellingProgramme;

/// A single point on the concentration decay curve.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ConcentrationPoint {
    pub quarter_label: String,
    pub concentrated_weight: f64,
    pub hhi: f64,
    pub estimated_annual_vol: f64,
    pub shares_remaining: f64,
}

/// BNY Mellon empirical data: (effective_positions, annual_vol%).
const DIVERSIFICATION_CURVE: &[(f64, f64)] = &[
    (1.0, 34.0),
    (2.0, 27.2),
    (5.0, 21.4),
    (10.0, 18.6),
    (20.0, 15.8),
    (50.0, 14.5),
    (100.0, 14.0),
];

/// Estimate annual volatility given the effective number of positions.
fn interpolate_vol(effective_positions: f64) -> f64 {
    let ep = effective_positions.max(1.0);
    for i in 0..DIVERSIFICATION_CURVE.len() - 1 {
        let (x0, y0) = DIVERSIFICATION_CURVE[i];
        let (x1, y1) = DIVERSIFICATION_CURVE[i + 1];
        if ep <= x1 {
            let t = (ep - x0) / (x1 - x0);
            return y0 + t * (y1 - y0);
        }
    }
    DIVERSIFICATION_CURVE.last().unwrap().1
}

/// Compute the projected concentration decay over a selling programme.
pub fn concentration_decay_points(
    _starting_weight: f64,
    starting_shares: f64,
    programme: &SellingProgramme,
    current_price: f64,
    other_portfolio_value: f64,
    start_year: u32,
) -> Vec<ConcentrationPoint> {
    let mut points = Vec::new();
    let mut remaining_shares = starting_shares;

    // Add starting point.
    let conc_value = remaining_shares * current_price;
    let total = conc_value + other_portfolio_value;
    let weight = if total > 0.0 { conc_value / total } else { 0.0 };
    let hhi = weight * weight + (1.0 - weight).powi(2) * 0.01;
    let eff_n = if hhi > 0.0 { 1.0 / hhi } else { 100.0 };

    points.push(ConcentrationPoint {
        quarter_label: "Start".to_string(),
        concentrated_weight: weight,
        hhi,
        estimated_annual_vol: interpolate_vol(eff_n),
        shares_remaining: remaining_shares,
    });

    for period in &programme.schedule {
        remaining_shares = (remaining_shares - period.shares_target).max(0.0);
        let conc_value = remaining_shares * current_price;
        let total = conc_value + other_portfolio_value;
        let weight = if total > 0.0 { conc_value / total } else { 0.0 };
        let hhi = weight * weight + (1.0 - weight).powi(2) * 0.01;
        let eff_n = if hhi > 0.0 { 1.0 / hhi } else { 100.0 };

        points.push(ConcentrationPoint {
            quarter_label: format!("{} Q{}", start_year + (period.year - 1), period.quarter),
            concentrated_weight: weight,
            hhi,
            estimated_annual_vol: interpolate_vol(eff_n),
            shares_remaining: remaining_shares,
        });
    }

    points
}

/// Render a concentration decay chart as an inline SVG.
pub fn concentration_decay_svg(points: &[ConcentrationPoint], width: u32, height: u32) -> String {
    if points.is_empty() {
        return format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\">\
             <text class=\"chart-muted\" x=\"50%\" y=\"50%\" text-anchor=\"middle\" font-size=\"14\">No data</text></svg>"
        );
    }

    let pad_l = 60.0_f64;
    let pad_r = 20.0;
    let pad_t = 30.0;
    let pad_b = 56.0;
    let w = width as f64 - pad_l - pad_r;
    let h = height as f64 - pad_t - pad_b;

    let max_vol = points
        .iter()
        .map(|p| p.estimated_annual_vol)
        .fold(0.0_f64, f64::max)
        * 1.1;
    let n = points.len() as f64;

    let mut body = String::new();

    let cx = width as f64 / 2.0;
    body.push_str(&format!(
        "<text x=\"{cx:.1}\" y=\"18\" font-size=\"13\" text-anchor=\"middle\">Portfolio Volatility Decay</text>"
    ));

    let mut path = String::new();
    let mut area = String::new();
    for (i, p) in points.iter().enumerate() {
        let x = pad_l + (i as f64 / (n - 1.0).max(1.0)) * w;
        let y = pad_t + h - (p.estimated_annual_vol / max_vol) * h;
        if i == 0 {
            path.push_str(&format!("M{x:.1},{y:.1}"));
            let base_y = pad_t + h;
            area.push_str(&format!("M{pad_l:.1},{base_y:.1} L{x:.1},{y:.1}"));
        } else {
            path.push_str(&format!(" L{x:.1},{y:.1}"));
            area.push_str(&format!(" L{x:.1},{y:.1}"));
        }
    }
    let close_x = pad_l + w;
    let close_y = pad_t + h;
    area.push_str(&format!(" L{close_x:.1},{close_y:.1} Z"));

    body.push_str(&format!(
        "<path d=\"{area}\" fill=\"var(--ga-series-1)\" fill-opacity=\"0.2\"/>"
    ));
    body.push_str(&format!(
        "<path d=\"{path}\" fill=\"none\" stroke=\"var(--ga-series-1)\" stroke-width=\"2.5\"/>"
    ));

    for (i, p) in points.iter().enumerate() {
        let x = pad_l + (i as f64 / (n - 1.0).max(1.0)) * w;
        let y = pad_t + h - (p.estimated_annual_vol / max_vol) * h;
        body.push_str(&format!(
            "<circle cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"3.5\" fill=\"var(--ga-series-1)\"/>"
        ));
        if i % 2 == 0 || i == points.len() - 1 {
            let ly = y - 8.0;
            let vol = p.estimated_annual_vol;
            body.push_str(&format!(
                "<text x=\"{x:.1}\" y=\"{ly:.1}\" font-size=\"11\" text-anchor=\"middle\">{vol:.1}%</text>"
            ));
            let label_y = pad_t + h + 15.0;
            body.push_str(&format!(
                "<text class=\"chart-muted\" x=\"{x:.1}\" y=\"{label_y:.1}\" font-size=\"11\" text-anchor=\"middle\">{}</text>",
                svg_escape(&p.quarter_label)
            ));
        }
    }

    let mid_y = pad_t + h / 2.0;
    body.push_str(&format!(
        "<text class=\"axis-title chart-muted\" x=\"15\" y=\"{mid_y:.1}\" font-size=\"11\" \
         transform=\"rotate(-90,15,{mid_y:.1})\" text-anchor=\"middle\">Annual volatility (%)</text>"
    ));
    let x_title_y = height as f64 - 6.0;
    body.push_str(&format!(
        "<text class=\"axis-title chart-muted\" x=\"{cx:.1}\" y=\"{x_title_y:.1}\" font-size=\"11\" \
         text-anchor=\"middle\">Time (years)</text>"
    ));

    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" \
         viewBox=\"0 0 {width} {height}\">{body}</svg>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_valid_svg(svg: &str) {
        assert!(svg.starts_with("<svg"), "SVG should start with <svg: {svg}");
        assert!(svg.ends_with("</svg>"), "SVG should end with </svg>: {svg}");
        assert!(!svg.contains("NaN"), "SVG must not contain NaN: {svg}");
        assert!(
            !svg.contains("Infinity"),
            "SVG must not contain Infinity: {svg}"
        );
    }

    // ── score_bar_svg ────────────────────────────────────────────────────────

    #[test]
    fn score_bar_svg_zero() {
        let svg = score_bar_svg(0.0, 200, 20);
        assert_valid_svg(&svg);
        // The fill bar should have width 0
        assert!(
            svg.contains(r#"width="0.0""#),
            "zero score should produce zero-width fill bar: {svg}"
        );
    }

    #[test]
    fn score_bar_svg_hundred() {
        let svg = score_bar_svg(100.0, 200, 20);
        assert_valid_svg(&svg);
        // The fill bar should span the full width
        assert!(
            svg.contains(r#"width="200.0""#),
            "score=100 should produce full-width bar: {svg}"
        );
    }

    #[test]
    fn score_bar_svg_negative_clamped() {
        let svg = score_bar_svg(-50.0, 200, 20);
        assert_valid_svg(&svg);
        assert!(
            svg.contains(r#"width="0.0""#),
            "negative score should be clamped to 0: {svg}"
        );
    }

    // ── regime_timeline_svg ──────────────────────────────────────────────────

    #[test]
    fn regime_timeline_svg_empty() {
        let svg = regime_timeline_svg(&[], 300, 30);
        assert_valid_svg(&svg);
        assert!(!svg.contains("<rect"), "empty input should have no rects");
    }

    #[test]
    fn regime_timeline_svg_single_entry() {
        let entries = vec![RegimeEntry {
            regime: MarketRegime::Bull,
            days: 10,
        }];
        let svg = regime_timeline_svg(&entries, 300, 30);
        assert_valid_svg(&svg);
        assert!(svg.contains("<rect"), "single entry should produce a rect");
    }

    #[test]
    fn regime_timeline_svg_colors() {
        let entries = vec![
            RegimeEntry {
                regime: MarketRegime::Bull,
                days: 10,
            },
            RegimeEntry {
                regime: MarketRegime::Bear,
                days: 5,
            },
            RegimeEntry {
                regime: MarketRegime::Sideways,
                days: 5,
            },
        ];
        let svg = regime_timeline_svg(&entries, 300, 30);
        assert_valid_svg(&svg);
        assert!(svg.contains("var(--ga-pos)"), "Bull should be green");
        assert!(svg.contains("var(--ga-neg)"), "Bear should be red");
        assert!(
            svg.contains("var(--ga-heat-3)"),
            "Sideways should be yellow"
        );
    }

    // ── sparkline_svg ────────────────────────────────────────────────────────

    #[test]
    fn sparkline_svg_basic() {
        let values = vec![1.0, 3.0, 2.0, 5.0, 4.0];
        let svg = sparkline_svg(&values, 100, 30);
        assert_valid_svg(&svg);
        assert!(
            svg.contains("<polyline"),
            "should contain a polyline element"
        );
    }

    #[test]
    fn sparkline_svg_single_point() {
        let svg = sparkline_svg(&[42.0], 100, 30);
        assert_valid_svg(&svg);
        // Should not panic and should produce valid SVG
        assert!(
            svg.contains("<polyline"),
            "single point should still produce polyline"
        );
    }

    #[test]
    fn sparkline_svg_empty() {
        let svg = sparkline_svg(&[], 100, 30);
        assert_valid_svg(&svg);
        assert!(
            !svg.contains("<polyline"),
            "empty input should have no polyline"
        );
    }

    #[test]
    fn sparkline_svg_all_same_value() {
        let values = vec![5.0, 5.0, 5.0, 5.0];
        let svg = sparkline_svg(&values, 100, 30);
        assert_valid_svg(&svg);
        // Specifically verify no NaN from zero-range division
        assert!(!svg.contains("NaN"), "constant series must not produce NaN");
    }

    // ── heatmap_svg ────────────────────────────────────────────────────────

    #[test]
    fn heatmap_empty_data() {
        let svg = heatmap_svg(&[], &[], &[], 400, 200, score_color);
        assert_valid_svg(&svg);
        assert!(!svg.contains("<rect"), "empty heatmap should have no rects");
    }

    #[test]
    fn heatmap_single_cell() {
        let data = vec![vec![Some(HeatmapCell {
            value: 95.0,
            label: "NVDA — 03/05: 95.0".into(),
        })]];
        let svg = heatmap_svg(&data, &["NVDA"], &["03/05"], 200, 100, score_color);
        assert_valid_svg(&svg);
        assert!(svg.contains("<rect"), "single cell should produce a rect");
        assert!(
            svg.contains("var(--ga-heat-5)"),
            "score 95 should map to dark green"
        );
        assert!(svg.contains("<title>"), "cell should have a tooltip");
    }

    #[test]
    fn heatmap_missing_cell_uses_gray() {
        let data = vec![vec![None]];
        let svg = heatmap_svg(&data, &["AAPL"], &["03/06"], 200, 100, score_color);
        assert_valid_svg(&svg);
        assert!(
            svg.contains("var(--ga-heat-missing)"),
            "missing cell should use gray fill"
        );
    }

    #[test]
    fn heatmap_multiple_tickers_and_dates() {
        let data = vec![
            vec![
                Some(HeatmapCell {
                    value: 92.0,
                    label: "NVDA 03/05".into(),
                }),
                Some(HeatmapCell {
                    value: 45.0,
                    label: "NVDA 03/06".into(),
                }),
            ],
            vec![
                Some(HeatmapCell {
                    value: 25.0,
                    label: "AAPL 03/05".into(),
                }),
                Some(HeatmapCell {
                    value: 75.0,
                    label: "AAPL 03/06".into(),
                }),
            ],
        ];
        let svg = heatmap_svg(
            &data,
            &["NVDA", "AAPL"],
            &["03/05", "03/06"],
            400,
            200,
            score_color,
        );
        assert_valid_svg(&svg);
        assert!(svg.contains("NVDA"), "should have ticker labels");
        assert!(svg.contains("03/05"), "should have date labels");
        // score 92 -> dark green, 45 -> orange, 25 -> red, 75 -> green
        assert!(svg.contains("var(--ga-heat-5)"), "score 92 = dark green");
        assert!(svg.contains("var(--ga-heat-2)"), "score 45 = orange");
        assert!(svg.contains("var(--ga-heat-1)"), "score 25 = red");
        assert!(svg.contains("var(--ga-heat-4)"), "score 75 = green");
    }

    #[test]
    fn heatmap_nan_value_does_not_panic() {
        let data = vec![vec![Some(HeatmapCell {
            value: f64::NAN,
            label: "nan".into(),
        })]];
        let svg = heatmap_svg(&data, &["X"], &["d1"], 200, 100, score_color);
        assert_valid_svg(&svg);
    }

    #[test]
    fn heatmap_sentiment_color_fn() {
        let data = vec![vec![Some(HeatmapCell {
            value: 15.0,
            label: "high".into(),
        })]];
        let svg = heatmap_svg(&data, &["T"], &["d1"], 200, 100, sentiment_color);
        assert_valid_svg(&svg);
        assert!(
            svg.contains("var(--ga-heat-5)"),
            "sentiment 15 should be dark green"
        );
    }

    // ── score_color / sentiment_color ──────────────────────────────────────

    #[test]
    fn score_color_thresholds() {
        assert_eq!(score_color(95.0), "var(--ga-heat-5)");
        assert_eq!(score_color(90.0), "var(--ga-heat-5)");
        assert_eq!(score_color(80.0), "var(--ga-heat-4)");
        assert_eq!(score_color(50.0), "var(--ga-heat-3)");
        assert_eq!(score_color(30.0), "var(--ga-heat-2)");
        assert_eq!(score_color(10.0), "var(--ga-heat-1)");
    }

    #[test]
    fn sentiment_color_thresholds() {
        assert_eq!(sentiment_color(15.0), "var(--ga-heat-5)");
        assert_eq!(sentiment_color(10.0), "var(--ga-heat-4)");
        assert_eq!(sentiment_color(7.0), "var(--ga-heat-neutral)");
        assert_eq!(sentiment_color(4.0), "var(--ga-heat-2)");
        assert_eq!(sentiment_color(1.0), "var(--ga-heat-1)");
    }

    // ── stacked_area_svg ───────────────────────────────────────────────────

    #[test]
    fn stacked_area_empty() {
        let svg = stacked_area_svg(&[], &[], 400, 200);
        assert_valid_svg(&svg);
        assert!(!svg.contains("<path"), "empty area should have no paths");
    }

    #[test]
    fn stacked_area_single_series_single_date() {
        let series = vec![AreaSeries {
            label: "A+".into(),
            color: "#2b8a3e".into(),
            values: vec![10.0],
        }];
        let svg = stacked_area_svg(&series, &["03/05"], 400, 200);
        assert_valid_svg(&svg);
        assert!(svg.contains("<path"), "should produce a path");
        assert!(svg.contains("#2b8a3e"), "should use series color");
    }

    #[test]
    fn stacked_area_multiple_series() {
        let series = vec![
            AreaSeries {
                label: "A+".into(),
                color: "#2b8a3e".into(),
                values: vec![5.0, 8.0, 3.0],
            },
            AreaSeries {
                label: "B".into(),
                color: "#51cf66".into(),
                values: vec![3.0, 2.0, 7.0],
            },
        ];
        let dates: Vec<&str> = vec!["d1", "d2", "d3"];
        let svg = stacked_area_svg(&series, &dates, 500, 300);
        assert_valid_svg(&svg);
        assert!(svg.contains("#2b8a3e"), "should contain first series color");
        assert!(
            svg.contains("#51cf66"),
            "should contain second series color"
        );
        assert!(svg.contains("opacity"), "paths should have opacity");
    }

    #[test]
    fn stacked_area_date_labels() {
        let series = vec![AreaSeries {
            label: "X".into(),
            color: "#aaa".into(),
            values: vec![1.0; 12],
        }];
        let dates: Vec<&str> = (0..12)
            .map(|i| match i {
                0 => "d0",
                1 => "d1",
                2 => "d2",
                3 => "d3",
                4 => "d4",
                5 => "d5",
                6 => "d6",
                7 => "d7",
                8 => "d8",
                9 => "d9",
                10 => "d10",
                _ => "d11",
            })
            .collect();
        let svg = stacked_area_svg(&series, &dates, 600, 200);
        assert_valid_svg(&svg);
        // Every 5th date label: d0, d5, d10
        assert!(svg.contains("d0"), "should show d0 label");
        assert!(svg.contains("d5"), "should show d5 label");
        assert!(svg.contains("d10"), "should show d10 label");
    }

    #[test]
    fn stacked_area_nan_values() {
        let series = vec![AreaSeries {
            label: "S".into(),
            color: "#ccc".into(),
            values: vec![f64::NAN, 5.0],
        }];
        let svg = stacked_area_svg(&series, &["d1", "d2"], 300, 150);
        assert_valid_svg(&svg);
    }

    // ── scatter_svg ────────────────────────────────────────────────────────

    #[test]
    fn scatter_empty() {
        let svg = scatter_svg(&[], "RS", "Mom", 400, 300);
        assert_valid_svg(&svg);
        assert!(
            !svg.contains("<circle"),
            "empty scatter should have no circles"
        );
    }

    #[test]
    fn scatter_single_point() {
        let pts = vec![ScatterPoint {
            x: 80.0,
            y: 5.0,
            label: "NVDA".into(),
            color: "#51cf66".into(),
            size: 6.0,
        }];
        let svg = scatter_svg(&pts, "RS Percentile", "RS Momentum", 400, 300);
        assert_valid_svg(&svg);
        assert!(svg.contains("<circle"), "should render a circle");
        assert!(svg.contains("NVDA"), "tooltip should contain label");
    }

    #[test]
    fn scatter_quadrant_lines() {
        let pts = vec![ScatterPoint {
            x: 50.0,
            y: 0.0,
            label: "T".into(),
            color: "#ccc".into(),
            size: 4.0,
        }];
        let svg = scatter_svg(&pts, "X", "Y", 400, 300);
        assert_valid_svg(&svg);
        assert!(
            svg.contains("stroke-dasharray"),
            "should have dashed quadrant lines"
        );
    }

    #[test]
    fn scatter_top5_labels() {
        let pts: Vec<ScatterPoint> = (0..8)
            .map(|i| ScatterPoint {
                x: i as f64 * 10.0,
                y: i as f64,
                label: format!("T{i}"),
                color: "#aaa".into(),
                size: (i + 1) as f64,
            })
            .collect();
        let svg = scatter_svg(&pts, "X", "Y", 600, 400);
        assert_valid_svg(&svg);
        // Top 5 by size: T7, T6, T5, T4, T3
        assert!(svg.contains(">T7<"), "T7 (largest) should have text label");
        assert!(
            svg.contains(">T3<"),
            "T3 (5th largest) should have text label"
        );
    }

    #[test]
    fn scatter_negative_y_symmetric() {
        let pts = vec![
            ScatterPoint {
                x: 30.0,
                y: -5.0,
                label: "A".into(),
                color: "#ccc".into(),
                size: 4.0,
            },
            ScatterPoint {
                x: 70.0,
                y: 5.0,
                label: "B".into(),
                color: "#ccc".into(),
                size: 4.0,
            },
        ];
        let svg = scatter_svg(&pts, "X", "Y", 400, 300);
        assert_valid_svg(&svg);
    }

    #[test]
    fn scatter_nan_size_defaults() {
        let pts = vec![ScatterPoint {
            x: 50.0,
            y: 1.0,
            label: "Z".into(),
            color: "#ccc".into(),
            size: f64::NAN,
        }];
        let svg = scatter_svg(&pts, "X", "Y", 400, 300);
        assert_valid_svg(&svg);
        assert!(svg.contains(r#"r="6.0""#), "NaN size should default to 6.0");
    }

    // ── calendar_heatmap_svg ───────────────────────────────────────────────

    #[test]
    fn calendar_empty() {
        let svg = calendar_heatmap_svg(&[], 400, 120);
        assert_valid_svg(&svg);
        assert!(
            !svg.contains("<rect"),
            "empty calendar should have no rects"
        );
    }

    #[test]
    fn calendar_single_day() {
        let days = vec![CalendarDay {
            date: "2026-03-05".into(),
            weekday: 3,
            count: 2,
            has_critical: false,
        }];
        let svg = calendar_heatmap_svg(&days, 400, 120);
        assert_valid_svg(&svg);
        assert!(svg.contains("<rect"), "should render a cell rect");
        assert!(
            svg.contains("var(--ga-cal-1)"),
            "count 2 should be light green"
        );
        assert!(svg.contains("2026-03-05"), "tooltip should contain date");
    }

    #[test]
    fn calendar_critical_overlay() {
        let days = vec![CalendarDay {
            date: "2026-03-05".into(),
            weekday: 0,
            count: 4,
            has_critical: true,
        }];
        let svg = calendar_heatmap_svg(&days, 400, 120);
        assert_valid_svg(&svg);
        assert!(
            svg.contains("<circle"),
            "critical day should have circle overlay"
        );
        assert!(
            svg.contains("var(--ga-neg)"),
            "critical circle should be red"
        );
        assert!(svg.contains("critical"), "tooltip should mention critical");
    }

    #[test]
    fn calendar_color_buckets() {
        let days = vec![
            CalendarDay {
                date: "d1".into(),
                weekday: 0,
                count: 0,
                has_critical: false,
            },
            CalendarDay {
                date: "d2".into(),
                weekday: 1,
                count: 1,
                has_critical: false,
            },
            CalendarDay {
                date: "d3".into(),
                weekday: 2,
                count: 4,
                has_critical: false,
            },
            CalendarDay {
                date: "d4".into(),
                weekday: 3,
                count: 8,
                has_critical: false,
            },
        ];
        let svg = calendar_heatmap_svg(&days, 400, 120);
        assert_valid_svg(&svg);
        assert!(svg.contains("var(--ga-cal-0)"), "count 0 -> lightest");
        assert!(svg.contains("var(--ga-cal-1)"), "count 1 -> light green");
        assert!(svg.contains("var(--ga-cal-2)"), "count 4 -> medium green");
        assert!(svg.contains("var(--ga-cal-3)"), "count 8 -> dark green");
    }

    #[test]
    fn calendar_weekday_labels() {
        let days = vec![CalendarDay {
            date: "d".into(),
            weekday: 0,
            count: 0,
            has_critical: false,
        }];
        let svg = calendar_heatmap_svg(&days, 400, 120);
        assert_valid_svg(&svg);
        assert!(svg.contains(">M<"), "should have Monday label");
        assert!(svg.contains(">F<"), "should have Friday label");
    }

    // ── gauge_svg ──────────────────────────────────────────────────────────

    #[test]
    fn gauge_basic() {
        let green = "#51cf66";
        let svg = gauge_svg(75.0, 0.0, 100.0, "Score", green);
        assert_valid_svg(&svg);
        assert!(svg.contains("75.0"), "should display the value");
        assert!(svg.contains("Score"), "should display the label");
        assert!(svg.contains("#51cf66"), "should use risk_color for the arc");
    }

    #[test]
    fn gauge_min_value() {
        let svg = gauge_svg(0.0, 0.0, 100.0, "Min", "#aaa");
        assert_valid_svg(&svg);
        // Background arc should be present but value arc may be absent
        assert!(
            svg.contains("var(--ga-border)"),
            "should have background arc"
        );
    }

    #[test]
    fn gauge_max_value() {
        let svg = gauge_svg(100.0, 0.0, 100.0, "Max", "#ff6b6b");
        assert_valid_svg(&svg);
        assert!(
            svg.contains("#ff6b6b"),
            "should use risk_color for full arc"
        );
    }

    #[test]
    fn gauge_nan_value() {
        let svg = gauge_svg(f64::NAN, 0.0, 100.0, "Risk", "#ccc");
        // The label "Risk" is fine; we just need to verify the value displays as "--"
        assert!(svg.starts_with("<svg"), "must produce an SVG element");
        assert!(svg.ends_with("</svg>"), "must close the SVG element");
        assert!(svg.contains("--"), "NaN should display as --");
    }

    #[test]
    fn gauge_zero_range() {
        let svg = gauge_svg(5.0, 5.0, 5.0, "Flat", "#ccc");
        assert_valid_svg(&svg);
    }

    #[test]
    fn heatmap_thins_date_labels_for_many_dates() {
        let num_dates = 35;
        let dates: Vec<String> = (0..num_dates).map(|i| format!("d{i:02}")).collect();
        let date_refs: Vec<&str> = dates.iter().map(|s| s.as_str()).collect();
        let row: Vec<Option<HeatmapCell>> = (0..num_dates)
            .map(|_| {
                Some(HeatmapCell {
                    value: 50.0,
                    label: "x".into(),
                })
            })
            .collect();
        let data = vec![row];
        let svg = heatmap_svg(&data, &["T1"], &date_refs, 800, 200, score_color);
        assert_valid_svg(&svg);
        // Count how many date labels appear as <text> elements with date content
        let label_count = dates
            .iter()
            .filter(|d| svg.contains(&format!(">{d}<")))
            .count();
        // With thinning (N = max(1, 35/6) = 5), we expect ~7 labels, not all 35
        assert!(
            label_count < num_dates,
            "Expected label thinning to reduce {num_dates} labels, but got {label_count}"
        );
    }

    // ── sentiment_color_normalized ────────────────────────────────────────

    #[test]
    fn sentiment_color_normalized_thresholds() {
        assert_eq!(
            sentiment_color_normalized(90.0),
            "var(--ga-heat-5)",
            ">=80 -> dark green"
        );
        assert_eq!(
            sentiment_color_normalized(80.0),
            "var(--ga-heat-5)",
            "80 boundary -> dark green"
        );
        assert_eq!(
            sentiment_color_normalized(70.0),
            "var(--ga-heat-4)",
            ">=60 -> light green"
        );
        assert_eq!(
            sentiment_color_normalized(60.0),
            "var(--ga-heat-4)",
            "60 boundary -> light green"
        );
        assert_eq!(
            sentiment_color_normalized(50.0),
            "var(--ga-heat-neutral)",
            ">=40 -> gray"
        );
        assert_eq!(
            sentiment_color_normalized(40.0),
            "var(--ga-heat-neutral)",
            "40 boundary -> gray"
        );
        assert_eq!(
            sentiment_color_normalized(30.0),
            "var(--ga-heat-2)",
            ">=20 -> orange"
        );
        assert_eq!(
            sentiment_color_normalized(20.0),
            "var(--ga-heat-2)",
            "20 boundary -> orange"
        );
        assert_eq!(
            sentiment_color_normalized(10.0),
            "var(--ga-heat-1)",
            "<20 -> red"
        );
        assert_eq!(
            sentiment_color_normalized(0.0),
            "var(--ga-heat-1)",
            "0 -> red"
        );
    }

    #[test]
    fn sentiment_color_normalized_extreme_values() {
        assert_eq!(sentiment_color_normalized(100.0), "var(--ga-heat-5)");
        assert_eq!(sentiment_color_normalized(-5.0), "var(--ga-heat-1)");
        assert_eq!(sentiment_color_normalized(150.0), "var(--ga-heat-5)");
    }

    // ── scatter_svg grid lines ────────────────────────────────────────────

    #[test]
    fn scatter_clustered_data_has_grid_lines() {
        let pts: Vec<ScatterPoint> = (0..10)
            .map(|i| ScatterPoint {
                x: 45.0 + i as f64,
                y: -0.5 + i as f64 * 0.1,
                label: format!("S{i}"),
                color: "#aaa".into(),
                size: 5.0,
            })
            .collect();
        let svg = scatter_svg(&pts, "RS Percentile", "RS Momentum", 600, 400);
        assert_valid_svg(&svg);
        // Should have subtle grid lines (stroke-width="0.5")
        assert!(
            svg.contains(r#"stroke-width="0.5""#),
            "scatter should have Y-axis grid lines"
        );
        // Should have X-axis tick labels
        assert!(svg.contains(">25<"), "should have x-axis tick at 25");
        assert!(svg.contains(">75<"), "should have x-axis tick at 75");
    }

    #[test]
    fn gauge_arc_has_path_element() {
        let svg = gauge_svg(50.0, 0.0, 100.0, "Mid", "#ffd43b");
        assert_valid_svg(&svg);
        assert!(
            svg.contains("<path"),
            "gauge should contain arc path elements"
        );
        // Should have at least the background arc path
        let path_count = svg.matches("<path").count();
        assert!(
            path_count >= 2,
            "should have background + value arc paths, got {path_count}"
        );
    }

    // ── donut_svg ─────────────────────────────────────────────────────────

    #[test]
    fn donut_empty_slices() {
        let svg = donut_svg(&[], 300, 300);
        assert_valid_svg(&svg);
        assert!(!svg.contains("<path"), "empty donut should have no paths");
        assert!(
            !svg.contains("<circle"),
            "empty donut should have no circles"
        );
    }

    #[test]
    fn donut_all_zero_values() {
        let slices = vec![
            DonutSlice {
                label: "A".into(),
                value: 0.0,
                color: "#ff0000".into(),
            },
            DonutSlice {
                label: "B".into(),
                value: 0.0,
                color: "#00ff00".into(),
            },
        ];
        let svg = donut_svg(&slices, 300, 300);
        assert_valid_svg(&svg);
        assert!(
            !svg.contains("<path"),
            "all-zero donut should have no paths"
        );
    }

    #[test]
    fn donut_single_slice_full_circle() {
        let slices = vec![DonutSlice {
            label: "Only".into(),
            value: 42.0,
            color: "#228be6".into(),
        }];
        let svg = donut_svg(&slices, 300, 300);
        assert_valid_svg(&svg);
        // Single slice renders as two circles (outer filled, inner white)
        assert!(
            svg.contains("<circle"),
            "single slice should use circle elements"
        );
        assert!(svg.contains("100%"), "single slice should show 100%");
        assert!(svg.contains("Only"), "single slice should show label");
        assert!(svg.contains("42"), "center should show total");
    }

    #[test]
    fn donut_multiple_slices() {
        let slices = vec![
            DonutSlice {
                label: "Tech".into(),
                value: 60.0,
                color: "#228be6".into(),
            },
            DonutSlice {
                label: "Health".into(),
                value: 30.0,
                color: "#51cf66".into(),
            },
            DonutSlice {
                label: "Energy".into(),
                value: 10.0,
                color: "#ff6b6b".into(),
            },
        ];
        let svg = donut_svg(&slices, 400, 400);
        assert_valid_svg(&svg);
        assert!(
            svg.contains("<path"),
            "multi-slice donut should have path arcs"
        );
        assert!(svg.contains("#228be6"), "should contain Tech color");
        assert!(svg.contains("#51cf66"), "should contain Health color");
        assert!(svg.contains("Tech"), "should have Tech label (60% > 5%)");
        assert!(
            svg.contains("Health"),
            "should have Health label (30% > 5%)"
        );
        assert!(
            svg.contains("Energy"),
            "should have Energy label (10% > 5%)"
        );
        assert!(svg.contains("100"), "center should show total 100");
    }

    #[test]
    fn donut_small_slices_no_label() {
        let slices = vec![
            DonutSlice {
                label: "Big".into(),
                value: 97.0,
                color: "#228be6".into(),
            },
            DonutSlice {
                label: "Tiny".into(),
                value: 3.0,
                color: "#ff6b6b".into(),
            },
        ];
        let svg = donut_svg(&slices, 300, 300);
        assert_valid_svg(&svg);
        // "Tiny" is 3% which is < 5%, so it should NOT appear as a label text
        // But it still appears as a path arc
        let path_count = svg.matches("<path").count();
        assert_eq!(path_count, 2, "should have 2 arc paths");
    }

    #[test]
    fn donut_nan_values_filtered() {
        let slices = vec![
            DonutSlice {
                label: "Good".into(),
                value: 50.0,
                color: "#228be6".into(),
            },
            DonutSlice {
                label: "Bad".into(),
                value: f64::NAN,
                color: "#ff0000".into(),
            },
        ];
        let svg = donut_svg(&slices, 300, 300);
        assert_valid_svg(&svg);
        // Should render as single slice since NaN is filtered
        assert!(
            svg.contains("<circle"),
            "NaN-filtered should be single slice"
        );
        assert!(svg.contains("100%"), "single valid slice should show 100%");
    }

    #[test]
    fn donut_large_total_formatted() {
        let slices = vec![
            DonutSlice {
                label: "A".into(),
                value: 500_000.0,
                color: "#228be6".into(),
            },
            DonutSlice {
                label: "B".into(),
                value: 500_000.0,
                color: "#51cf66".into(),
            },
        ];
        let svg = donut_svg(&slices, 300, 300);
        assert_valid_svg(&svg);
        assert!(svg.contains("1.0M"), "total 1M should be formatted as 1.0M");
    }

    // ── correlation_color ─────────────────────────────────────────────────

    #[test]
    fn correlation_color_extremes() {
        let neg1 = correlation_color(-1.0);
        assert_eq!(
            neg1,
            "color-mix(in srgb, var(--ga-corr-neg) 100%, var(--ga-corr-zero))"
        );

        let zero = correlation_color(0.0);
        assert_eq!(
            zero,
            "color-mix(in srgb, var(--ga-corr-pos) 0%, var(--ga-corr-zero))"
        );

        let pos1 = correlation_color(1.0);
        assert_eq!(
            pos1,
            "color-mix(in srgb, var(--ga-corr-pos) 100%, var(--ga-corr-zero))"
        );
    }

    #[test]
    fn correlation_color_midpoints() {
        let neg_half = correlation_color(-0.5);
        // Should be between blue and white
        assert!(neg_half.contains("var(--ga-corr-neg) 75%"), "{neg_half}");
        assert_ne!(neg_half, correlation_color(-1.0), "should not be pure blue");
        assert_ne!(neg_half, correlation_color(0.0), "should not be pure white");

        let pos_half = correlation_color(0.5);
        assert!(pos_half.contains("var(--ga-corr-pos) 75%"), "{pos_half}");
        assert_ne!(pos_half, correlation_color(0.0), "should not be pure white");
        assert_ne!(pos_half, correlation_color(1.0), "should not be pure red");
    }

    #[test]
    fn correlation_color_nan_defaults_white() {
        let nan_color = correlation_color(f64::NAN);
        assert_eq!(
            nan_color,
            correlation_color(0.0),
            "NaN correlation should map to the neutral mid colour (0)"
        );
    }

    #[test]
    fn correlation_color_clamped() {
        let over = correlation_color(2.0);
        let at_one = correlation_color(1.0);
        assert_eq!(over, at_one, "values > 1 should clamp to 1");

        let under = correlation_color(-5.0);
        let at_neg_one = correlation_color(-1.0);
        assert_eq!(under, at_neg_one, "values < -1 should clamp to -1");
    }

    // ── correlation_matrix_svg ────────────────────────────────────────────

    #[test]
    fn correlation_matrix_empty() {
        let svg = correlation_matrix_svg(&[], &[], 400, 400);
        assert_valid_svg(&svg);
        assert!(!svg.contains("<rect"), "empty matrix should have no rects");
    }

    #[test]
    fn correlation_matrix_single_ticker() {
        let tickers = vec!["AAPL"];
        let matrix = vec![vec![1.0]];
        let svg = correlation_matrix_svg(&tickers, &matrix, 300, 300);
        assert_valid_svg(&svg);
        assert!(svg.contains("<rect"), "should have a cell rect");
        assert!(svg.contains("1.00"), "diagonal should show 1.00");
        assert!(svg.contains("AAPL"), "should show ticker label");
        // Diagonal cell should have stroke
        let diag_marker = r#"stroke="var(--ga-text)""#;
        assert!(svg.contains(diag_marker), "diagonal should be highlighted");
    }

    #[test]
    fn correlation_matrix_2x2() {
        let tickers = vec!["AAPL", "MSFT"];
        let matrix = vec![vec![1.0, 0.85], vec![0.85, 1.0]];
        let svg = correlation_matrix_svg(&tickers, &matrix, 400, 400);
        assert_valid_svg(&svg);
        let rect_count = svg.matches("<rect").count();
        assert_eq!(rect_count, 4, "2x2 matrix should have 4 rects");
        assert!(svg.contains("0.85"), "should display correlation values");
        assert!(svg.contains("AAPL"), "should have AAPL label");
        assert!(svg.contains("MSFT"), "should have MSFT label");
    }

    #[test]
    fn correlation_matrix_nan_handled() {
        let tickers = vec!["A", "B"];
        let matrix = vec![vec![1.0, f64::NAN], vec![f64::NAN, 1.0]];
        let svg = correlation_matrix_svg(&tickers, &matrix, 300, 300);
        assert_valid_svg(&svg);
    }

    #[test]
    fn correlation_matrix_uses_diverging_colors() {
        let tickers = vec!["A", "B"];
        let matrix = vec![vec![1.0, -0.9], vec![-0.9, 1.0]];
        let svg = correlation_matrix_svg(&tickers, &matrix, 400, 400);
        assert_valid_svg(&svg);
        // The -0.9 cells should be blueish, the diagonal (1.0) should be reddish
        // Just verify we get distinct fills
        let fill_count = svg.matches("fill=\"color-mix").count();
        assert!(fill_count >= 4, "should have fill colors on all cells");
    }

    // ── frontier_svg ──────────────────────────────────────────────────────

    #[test]
    fn frontier_empty() {
        let svg = frontier_svg(&[], None, 600, 400);
        assert_valid_svg(&svg);
        assert!(
            !svg.contains("<circle"),
            "empty frontier should have no circles"
        );
    }

    #[test]
    fn frontier_single_point() {
        let points = vec![FrontierPoint {
            risk: 10.0,
            ret: 8.0,
            label: "Min Var".into(),
            color: "#228be6".into(),
            highlighted: true,
        }];
        let svg = frontier_svg(&points, None, 600, 400);
        assert_valid_svg(&svg);
        assert!(svg.contains("<circle"), "should have a point circle");
        assert!(svg.contains("Min Var"), "should show highlighted label");
        assert!(svg.contains("Risk (%)"), "should have x-axis label");
        assert!(svg.contains("Return (%)"), "should have y-axis label");
    }

    #[test]
    fn frontier_multiple_points_with_curve() {
        let points = vec![
            FrontierPoint {
                risk: 5.0,
                ret: 4.0,
                label: "Min Var".into(),
                color: "#228be6".into(),
                highlighted: true,
            },
            FrontierPoint {
                risk: 10.0,
                ret: 7.0,
                label: "Mid".into(),
                color: "#51cf66".into(),
                highlighted: false,
            },
            FrontierPoint {
                risk: 15.0,
                ret: 9.0,
                label: "Max Sharpe".into(),
                color: "#ff6b6b".into(),
                highlighted: true,
            },
        ];
        let svg = frontier_svg(&points, None, 600, 400);
        assert_valid_svg(&svg);
        assert!(svg.contains("<path"), "should have frontier curve path");
        let circle_count = svg.matches("<circle").count();
        assert_eq!(circle_count, 3, "should have 3 point circles");
        // Highlighted points get text labels
        assert!(
            svg.contains(">Min Var<"),
            "highlighted point should have text label"
        );
        assert!(
            svg.contains(">Max Sharpe<"),
            "highlighted point should have text label"
        );
    }

    #[test]
    fn frontier_with_current_portfolio() {
        let points = vec![
            FrontierPoint {
                risk: 8.0,
                ret: 5.0,
                label: "A".into(),
                color: "#228be6".into(),
                highlighted: false,
            },
            FrontierPoint {
                risk: 12.0,
                ret: 8.0,
                label: "B".into(),
                color: "#228be6".into(),
                highlighted: false,
            },
        ];
        let current = FrontierPoint {
            risk: 10.0,
            ret: 6.0,
            label: "Current".into(),
            color: "#ff6b6b".into(),
            highlighted: true,
        };
        let svg = frontier_svg(&points, Some(current), 600, 400);
        assert_valid_svg(&svg);
        let circle_count = svg.matches("<circle").count();
        assert_eq!(
            circle_count, 3,
            "should have 2 frontier + 1 current circles"
        );
        assert!(
            svg.contains("Current"),
            "should show current portfolio label"
        );
    }

    #[test]
    fn frontier_nan_values_handled() {
        let points = vec![
            FrontierPoint {
                risk: f64::NAN,
                ret: 5.0,
                label: "A".into(),
                color: "#ccc".into(),
                highlighted: false,
            },
            FrontierPoint {
                risk: 10.0,
                ret: f64::NAN,
                label: "B".into(),
                color: "#ccc".into(),
                highlighted: false,
            },
        ];
        let svg = frontier_svg(&points, None, 600, 400);
        assert_valid_svg(&svg);
    }

    #[test]
    fn frontier_has_grid_lines() {
        let points = vec![FrontierPoint {
            risk: 5.0,
            ret: 3.0,
            label: "A".into(),
            color: "#ccc".into(),
            highlighted: false,
        }];
        let svg = frontier_svg(&points, None, 600, 400);
        assert_valid_svg(&svg);
        assert!(svg.contains("<line"), "should have grid lines");
        assert!(
            svg.contains(r#"stroke-width="0.5""#),
            "grid lines should be subtle"
        );
    }

    // ── fan_chart_svg ─────────────────────────────────────────────────────

    #[test]
    fn fan_chart_empty() {
        let svg = fan_chart_svg(&[], 600, 400);
        assert_valid_svg(&svg);
        assert!(
            !svg.contains("<path"),
            "empty fan chart should have no paths"
        );
    }

    #[test]
    fn fan_chart_single_year() {
        let years = vec![FanChartYear {
            year: 2026,
            p10: 80_000.0,
            p25: 90_000.0,
            p50: 100_000.0,
            p75: 110_000.0,
            p90: 120_000.0,
        }];
        let svg = fan_chart_svg(&years, 600, 400);
        assert_valid_svg(&svg);
        assert!(
            svg.contains("<path"),
            "single year should still produce paths"
        );
        assert!(svg.contains("2026"), "should show year label");
    }

    #[test]
    fn fan_chart_multiple_years() {
        let years = vec![
            FanChartYear {
                year: 2026,
                p10: 80_000.0,
                p25: 90_000.0,
                p50: 100_000.0,
                p75: 110_000.0,
                p90: 120_000.0,
            },
            FanChartYear {
                year: 2027,
                p10: 70_000.0,
                p25: 95_000.0,
                p50: 120_000.0,
                p75: 150_000.0,
                p90: 180_000.0,
            },
            FanChartYear {
                year: 2028,
                p10: 60_000.0,
                p25: 100_000.0,
                p50: 150_000.0,
                p75: 200_000.0,
                p90: 250_000.0,
            },
        ];
        let svg = fan_chart_svg(&years, 600, 400);
        assert_valid_svg(&svg);
        // Should have 3 paths: outer band, inner band, median line
        let path_count = svg.matches("<path").count();
        assert_eq!(
            path_count, 3,
            "should have outer band + inner band + median line, got {path_count}"
        );
        assert!(
            svg.contains("opacity=\"0.15\""),
            "outer band should have light opacity"
        );
        assert!(
            svg.contains("opacity=\"0.3\""),
            "inner band should have medium opacity"
        );
        assert!(svg.contains("var(--ga-series-1)"), "should use blue color");
    }

    #[test]
    fn fan_chart_dollar_formatting() {
        let years = vec![FanChartYear {
            year: 2026,
            p10: 500.0,
            p25: 1_000.0,
            p50: 5_000.0,
            p75: 50_000.0,
            p90: 1_500_000.0,
        }];
        let svg = fan_chart_svg(&years, 600, 400);
        assert_valid_svg(&svg);
        // Y-axis labels should use dollar formatting
        assert!(
            svg.contains("$"),
            "should have dollar signs in y-axis labels"
        );
    }

    #[test]
    fn fan_chart_nan_values_handled() {
        let years = vec![
            FanChartYear {
                year: 2026,
                p10: f64::NAN,
                p25: 90_000.0,
                p50: 100_000.0,
                p75: 110_000.0,
                p90: 120_000.0,
            },
            FanChartYear {
                year: 2027,
                p10: 80_000.0,
                p25: 95_000.0,
                p50: f64::NAN,
                p75: 150_000.0,
                p90: 180_000.0,
            },
        ];
        let svg = fan_chart_svg(&years, 600, 400);
        assert_valid_svg(&svg);
    }

    #[test]
    fn fan_chart_year_labels_thinned() {
        let years: Vec<FanChartYear> = (2020..2050)
            .map(|y| FanChartYear {
                year: y,
                p10: 100.0,
                p25: 200.0,
                p50: 300.0,
                p75: 400.0,
                p90: 500.0,
            })
            .collect();
        let svg = fan_chart_svg(&years, 600, 400);
        assert_valid_svg(&svg);
        // Should not show all 30 year labels
        let year_label_count = (2020..2050)
            .filter(|y| svg.contains(&format!(">{y}<")))
            .count();
        assert!(
            year_label_count < 30,
            "should thin year labels, got {year_label_count}"
        );
        assert!(
            year_label_count >= 4,
            "should show at least a few year labels, got {year_label_count}"
        );
    }

    // ── horizontal_bar_svg ────────────────────────────────────────────────

    #[test]
    fn horizontal_bar_empty() {
        let svg = horizontal_bar_svg(&[], 400, 300);
        assert_valid_svg(&svg);
        assert!(
            !svg.contains("<rect"),
            "empty bar chart should have no rects"
        );
    }

    #[test]
    fn horizontal_bar_single() {
        let bars = vec![HBar {
            label: "AAPL".into(),
            value: 42.5,
            color: "#228be6".into(),
        }];
        let svg = horizontal_bar_svg(&bars, 400, 200);
        assert_valid_svg(&svg);
        assert!(svg.contains("<rect"), "should have a bar rect");
        assert!(svg.contains("AAPL"), "should show label");
        assert!(svg.contains("42.5"), "should show value");
    }

    #[test]
    fn horizontal_bar_multiple_sorted() {
        let bars = vec![
            HBar {
                label: "Small".into(),
                value: 10.0,
                color: "#ccc".into(),
            },
            HBar {
                label: "Big".into(),
                value: 90.0,
                color: "#228be6".into(),
            },
            HBar {
                label: "Mid".into(),
                value: 50.0,
                color: "#51cf66".into(),
            },
        ];
        let svg = horizontal_bar_svg(&bars, 400, 300);
        assert_valid_svg(&svg);
        let rect_count = svg.matches("<rect").count();
        assert_eq!(rect_count, 3, "should have 3 bar rects");
        // Big should appear first (top) since bars are sorted by value descending
        let big_pos = svg.find("Big").expect("should contain Big");
        let small_pos = svg.find("Small").expect("should contain Small");
        assert!(
            big_pos < small_pos,
            "Big (90) should appear before Small (10) in SVG"
        );
    }

    #[test]
    fn horizontal_bar_nan_filtered() {
        let bars = vec![
            HBar {
                label: "Valid".into(),
                value: 50.0,
                color: "#228be6".into(),
            },
            HBar {
                label: "Invalid".into(),
                value: f64::NAN,
                color: "#ff0000".into(),
            },
        ];
        let svg = horizontal_bar_svg(&bars, 400, 200);
        assert_valid_svg(&svg);
        let rect_count = svg.matches("<rect").count();
        assert_eq!(rect_count, 1, "NaN bar should be filtered, leaving 1 rect");
    }

    #[test]
    fn horizontal_bar_all_nan() {
        let bars = vec![
            HBar {
                label: "A".into(),
                value: f64::NAN,
                color: "#ccc".into(),
            },
            HBar {
                label: "B".into(),
                value: f64::INFINITY,
                color: "#ccc".into(),
            },
        ];
        let svg = horizontal_bar_svg(&bars, 400, 200);
        assert_valid_svg(&svg);
        assert!(
            !svg.contains("<rect"),
            "all-invalid bars should produce empty SVG"
        );
    }

    // ── timeline_bar_svg ──────────────────────────────────────────────────

    #[test]
    fn timeline_bar_empty() {
        let svg = timeline_bar_svg(&[], 500, 300);
        assert_valid_svg(&svg);
        assert!(
            !svg.contains("<rect"),
            "empty timeline should have no rects"
        );
    }

    #[test]
    fn timeline_bar_single_period() {
        let periods = vec![TimelinePeriod {
            label: "Q1 2026".into(),
            target: 1000.0,
            executed: 750.0,
            color: "#51cf66".into(),
        }];
        let svg = timeline_bar_svg(&periods, 500, 200);
        assert_valid_svg(&svg);
        assert!(svg.contains("Q1 2026"), "should show period label");
        // Should have 2 rects: target (gray) + executed (green)
        let rect_count = svg.matches("<rect").count();
        assert_eq!(rect_count, 2, "should have target + executed rects");
        assert!(
            svg.contains("var(--ga-border)"),
            "should have gray target bar"
        );
        assert!(
            svg.contains("var(--ga-pos)"),
            "should have green executed bar"
        );
        assert!(svg.contains("75%"), "should show 75% completion");
    }

    #[test]
    fn timeline_bar_multiple_periods() {
        let periods = vec![
            TimelinePeriod {
                label: "Q1".into(),
                target: 100.0,
                executed: 80.0,
                color: "#51cf66".into(),
            },
            TimelinePeriod {
                label: "Q2".into(),
                target: 100.0,
                executed: 100.0,
                color: "#51cf66".into(),
            },
            TimelinePeriod {
                label: "Q3".into(),
                target: 100.0,
                executed: 50.0,
                color: "#51cf66".into(),
            },
        ];
        let svg = timeline_bar_svg(&periods, 500, 300);
        assert_valid_svg(&svg);
        let rect_count = svg.matches("<rect").count();
        assert_eq!(rect_count, 6, "3 periods x 2 bars each = 6 rects");
        assert!(svg.contains("100%"), "Q2 should show 100% completion");
        assert!(svg.contains("50%"), "Q3 should show 50% completion");
    }

    #[test]
    fn timeline_bar_nan_filtered() {
        let periods = vec![
            TimelinePeriod {
                label: "Good".into(),
                target: 100.0,
                executed: 50.0,
                color: "#ccc".into(),
            },
            TimelinePeriod {
                label: "Bad".into(),
                target: f64::NAN,
                executed: 50.0,
                color: "#ccc".into(),
            },
        ];
        let svg = timeline_bar_svg(&periods, 500, 200);
        assert_valid_svg(&svg);
        let rect_count = svg.matches("<rect").count();
        assert_eq!(
            rect_count, 2,
            "NaN period should be filtered, leaving 1 period x 2 rects"
        );
    }

    #[test]
    fn timeline_bar_zero_target() {
        let periods = vec![TimelinePeriod {
            label: "Zero".into(),
            target: 0.0,
            executed: 0.0,
            color: "#ccc".into(),
        }];
        let svg = timeline_bar_svg(&periods, 500, 200);
        assert_valid_svg(&svg);
        assert!(svg.contains("0%"), "zero target should show 0% completion");
    }

    // ── format_dollars (indirect via fan_chart) ───────────────────────────

    #[test]
    fn format_dollars_units() {
        assert_eq!(format_dollars(500.0), "$500");
        assert_eq!(format_dollars(1_500.0), "$2K"); // 1500/1000 = 1.5, {:.0} rounds to 2
        assert_eq!(format_dollars(5_000.0), "$5K");
        assert_eq!(format_dollars(2_500_000.0), "$2.5M");
        assert_eq!(format_dollars(0.0), "$0");
    }

    // ── Concentration decay tests ──────────────────────────────────────

    #[test]
    fn test_concentration_decay_basic() {
        let prog = SellingProgramme::build_schedule("AMZN", &[400.0], 1);
        let points = concentration_decay_points(0.75, 1000.0, &prog, 200.0, 66_667.0, 2026);
        // Starting point + 4 quarters
        assert_eq!(points.len(), 5);
        assert!(
            points.first().unwrap().concentrated_weight
                > points.last().unwrap().concentrated_weight
        );
    }

    #[test]
    fn test_concentration_decay_vol_decreases() {
        let prog = SellingProgramme::build_schedule("AMZN", &[400.0], 1);
        let points = concentration_decay_points(0.80, 1000.0, &prog, 200.0, 50_000.0, 2026);
        for i in 1..points.len() {
            assert!(
                points[i].estimated_annual_vol <= points[i - 1].estimated_annual_vol + 0.01,
                "Vol should be non-increasing: {} vs {}",
                points[i].estimated_annual_vol,
                points[i - 1].estimated_annual_vol
            );
        }
    }

    #[test]
    fn test_concentration_decay_shares_decrease() {
        let prog = SellingProgramme::build_schedule("AMZN", &[400.0], 1);
        let points = concentration_decay_points(0.75, 1000.0, &prog, 200.0, 66_667.0, 2026);
        for i in 1..points.len() {
            assert!(points[i].shares_remaining <= points[i - 1].shares_remaining);
        }
    }

    #[test]
    fn test_concentration_decay_empty_programme() {
        let prog = SellingProgramme {
            id: None,
            ticker: "X".into(),
            schedule: vec![],
            pace_rules: vec![],
            created_at: None,
        };
        let points = concentration_decay_points(0.50, 100.0, &prog, 100.0, 10_000.0, 2026);
        assert_eq!(points.len(), 1); // Just starting point
    }

    #[test]
    fn test_vol_interpolation_known_points() {
        // 1 stock => 34%
        assert!((interpolate_vol(1.0) - 34.0).abs() < f64::EPSILON);
        // 5 stocks => 21.4%
        assert!((interpolate_vol(5.0) - 21.4).abs() < f64::EPSILON);
        // 20 stocks => 15.8%
        assert!((interpolate_vol(20.0) - 15.8).abs() < f64::EPSILON);
    }

    #[test]
    fn test_concentration_decay_svg_valid() {
        let prog = SellingProgramme::build_schedule("AMZN", &[400.0], 1);
        let points = concentration_decay_points(0.75, 1000.0, &prog, 200.0, 66_667.0, 2026);
        let svg = concentration_decay_svg(&points, 600, 300);
        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("</svg>"));
    }

    #[test]
    fn test_concentration_decay_svg_has_data_points() {
        let prog = SellingProgramme::build_schedule("AMZN", &[400.0], 1);
        let points = concentration_decay_points(0.75, 1000.0, &prog, 200.0, 66_667.0, 2026);
        let svg = concentration_decay_svg(&points, 600, 300);
        assert!(
            svg.contains("<circle"),
            "SVG should have data point circles"
        );
    }

    #[test]
    fn test_concentration_decay_svg_empty() {
        let svg = concentration_decay_svg(&[], 600, 300);
        assert!(svg.contains("No data"));
    }
}
