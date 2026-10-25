//! RED-phase contract for dashboard quick win Q5: `web/static/tokens.css`.
//!
//! Contract for `web/static/tokens.css` (embedded, served at /static/tokens.css):
//! * `:root { ... }` = LIGHT set.
//! * `@media (prefers-color-scheme: dark) { :root { ... } }` (selector may also be
//!   `:root:not([data-theme=light])`) = DARK set.
//! * `[data-theme=light] { ... }` and `[data-theme=dark] { ... }` (optionally
//!   `:root[data-theme=...]`, quotes optional) each define the COMPLETE colour set so a
//!   user override beats the media query. They must appear AFTER the media block.
//! * All colour token values are hex literals (`#rgb` or `#rrggbb`).
//!
//! Required colour tokens: see `COLOR_TOKENS` + `badge_tokens()`.
//! Required non-colour tokens in `:root`: --ga-space-1..4, --ga-radius, --ga-font-chart (>= 11px).

use std::collections::BTreeMap;

const TOKENS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/web/static/tokens.css");
const CSS_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/web/static/ghost-alpha.css");

const COLOR_TOKENS: &[&str] = &[
    "--ga-bg",
    "--ga-surface",
    "--ga-border",
    "--ga-text",
    "--ga-muted",
    "--ga-pos",
    "--ga-neg",
    "--ga-warn",
    "--ga-info",
    "--ga-focus",
    "--ga-series-1",
    "--ga-series-2",
    "--ga-series-3",
    "--ga-series-4",
];

const BADGES: &[&str] = &[
    "critical",
    "warning",
    "info",
    "bull",
    "bear",
    "sideways",
    "grade-aplus",
    "grade-a",
    "grade-b",
    "grade-c",
    "grade-d",
    "grade-f",
];

fn badge_tokens() -> Vec<String> {
    BADGES
        .iter()
        .flat_map(|b| [format!("--ga-badge-{b}-bg"), format!("--ga-badge-{b}-fg")])
        .collect()
}

type Vars = BTreeMap<String, String>;

struct Block {
    selector: String,
    media: Option<String>,
    vars: Vars,
    order: usize,
}

fn strip_comments(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("/*") {
        out.push_str(&rest[..i]);
        match rest[i..].find("*/") {
            Some(e) => rest = &rest[i + e + 2..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != '"' && *c != '\'')
        .collect()
}

fn parse_decls(body: &str) -> Vars {
    body.split(';')
        .filter_map(|d| {
            let (k, v) = d.split_once(':')?;
            let k = k.trim();
            k.starts_with("--")
                .then(|| (k.to_string(), v.trim().to_string()))
        })
        .collect()
}

fn parse_blocks(css: &str, media: Option<&str>, counter: &mut usize, out: &mut Vec<Block>) {
    let mut rest = css;
    while let Some(open) = rest.find('{') {
        let selector = rest[..open].trim().to_string();
        let mut depth = 0usize;
        let mut end = None;
        for (i, c) in rest[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(open + i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let end = end.expect("unbalanced braces in css");
        let body = &rest[open + 1..end];
        if selector.starts_with("@media") {
            parse_blocks(body, Some(&norm(&selector)), counter, out);
        } else {
            *counter += 1;
            out.push(Block {
                selector: norm(&selector),
                media: media.map(str::to_string),
                vars: parse_decls(body),
                order: *counter,
            });
        }
        rest = &rest[end + 1..];
    }
}

struct Themes {
    light: Vars,
    dark_media: Vars,
    dark_forced: Vars,
    light_forced: Vars,
    media_order: usize,
    forced_orders: Vec<usize>,
}

fn load() -> Themes {
    let css = strip_comments(
        &std::fs::read_to_string(TOKENS_PATH).expect("web/static/tokens.css must exist"),
    );
    let mut blocks = Vec::new();
    parse_blocks(&css, None, &mut 0, &mut blocks);
    let find = |pred: &dyn Fn(&Block) -> bool, what: &str| -> Vec<&Block> {
        let v: Vec<&Block> = blocks.iter().filter(|b| pred(b)).collect();
        assert!(!v.is_empty(), "tokens.css lacks {what}");
        v
    };
    let light = find(
        &|b| b.media.is_none() && b.selector == ":root",
        ":root block",
    );
    let dark_m = find(
        &|b| {
            b.media.as_deref() == Some("@media(prefers-color-scheme:dark)")
                && (b.selector == ":root" || b.selector == ":root:not([data-theme=light])")
        },
        "@media (prefers-color-scheme: dark) :root block",
    );
    let fd = find(
        &|b| {
            b.media.is_none()
                && (b.selector == "[data-theme=dark]" || b.selector == ":root[data-theme=dark]")
        },
        "[data-theme=dark] block",
    );
    let fl = find(
        &|b| {
            b.media.is_none()
                && (b.selector == "[data-theme=light]" || b.selector == ":root[data-theme=light]")
        },
        "[data-theme=light] block",
    );
    let merge = |bs: &[&Block]| -> Vars { bs.iter().flat_map(|b| b.vars.clone()).collect() };
    Themes {
        light: merge(&light),
        dark_media: merge(&dark_m),
        dark_forced: merge(&fd),
        light_forced: merge(&fl),
        media_order: dark_m.iter().map(|b| b.order).max().unwrap(),
        forced_orders: fd.iter().chain(fl.iter()).map(|b| b.order).collect(),
    }
}

// ───────────────────────── WCAG ─────────────────────────

fn hex(v: &str) -> (f64, f64, f64) {
    let h = v
        .trim()
        .strip_prefix('#')
        .unwrap_or_else(|| panic!("colour token must be a hex literal, got {v:?}"));
    let h: String = match h.len() {
        3 => h.chars().flat_map(|c| [c, c]).collect(),
        6 => h.to_string(),
        _ => panic!("bad hex colour {v:?}"),
    };
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).unwrap() as f64 / 255.0;
    (p(0), p(2), p(4))
}

fn lum(v: &str) -> f64 {
    let (r, g, b) = hex(v);
    let lin = |c: f64| {
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
}

fn contrast(a: &str, b: &str) -> f64 {
    let (la, lb) = (lum(a), lum(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

#[test]
fn test_wcag_helper_when_black_on_white_should_be_21_to_1() {
    // Self-check of the formula so a bug here cannot hide a real failure.
    assert!((contrast("#000", "#ffffff") - 21.0).abs() < 1e-9);
    assert!((contrast("#777", "#fff") - 4.48).abs() < 0.01);
}

fn effective(t: &Themes, dark: bool) -> &Vars {
    if dark {
        &t.dark_forced
    } else {
        &t.light_forced
    }
}

fn all_color_tokens() -> Vec<String> {
    COLOR_TOKENS
        .iter()
        .map(|s| s.to_string())
        .chain(badge_tokens())
        .collect()
}

// ───────────────────────── structure ─────────────────────────

#[test]
fn test_tokens_when_parsed_should_define_every_colour_token_in_all_four_blocks() {
    let t = load();
    for (name, vars) in [
        ("light :root", &t.light),
        ("dark media", &t.dark_media),
        ("[data-theme=dark]", &t.dark_forced),
        ("[data-theme=light]", &t.light_forced),
    ] {
        for tok in all_color_tokens() {
            let v = vars
                .get(&tok)
                .unwrap_or_else(|| panic!("{name} block is missing {tok}"));
            hex(v); // must be a hex literal
        }
    }
}

#[test]
fn test_tokens_when_dark_blocks_compared_should_media_and_forced_dark_agree_and_forced_light_matches_root(
) {
    let t = load();
    for tok in all_color_tokens() {
        assert_eq!(
            t.dark_media[&tok], t.dark_forced[&tok],
            "dark mismatch {tok}"
        );
        assert_eq!(t.light[&tok], t.light_forced[&tok], "light mismatch {tok}");
    }
}

#[test]
fn test_tokens_when_cascade_ordered_should_put_forced_themes_after_media_query() {
    let t = load();
    assert!(
        t.forced_orders.iter().all(|o| *o > t.media_order),
        "[data-theme] blocks must follow the prefers-color-scheme block so they win"
    );
}

#[test]
fn test_tokens_when_light_and_dark_compared_should_actually_differ() {
    let t = load();
    assert!(lum(&t.light["--ga-bg"]) > 0.6, "light bg should be light");
    assert!(
        lum(&t.dark_forced["--ga-bg"]) < 0.1,
        "dark bg should be dark"
    );
    assert!(lum(&t.light["--ga-text"]) < lum(&t.light["--ga-bg"]));
    assert!(lum(&t.dark_forced["--ga-text"]) > lum(&t.dark_forced["--ga-bg"]));
}

#[test]
fn test_tokens_when_root_parsed_should_define_spacing_radius_and_chart_font_size() {
    let t = load();
    for tok in [
        "--ga-space-1",
        "--ga-space-2",
        "--ga-space-3",
        "--ga-space-4",
        "--ga-radius",
        "--ga-font-chart",
    ] {
        assert!(t.light.contains_key(tok), "missing {tok}");
    }
    let px: f64 = t.light["--ga-font-chart"]
        .trim()
        .strip_suffix("px")
        .expect("--ga-font-chart in px")
        .parse()
        .unwrap();
    assert!(px >= 11.0, "chart text must be >= 11px");
}

// ───────────────────────── contrast ─────────────────────────

fn check_theme(dark: bool) {
    let t = load();
    let v = effective(&t, dark);
    let theme = if dark { "dark" } else { "light" };
    let mut failures = Vec::new();
    let mut chk = |fg: &str, bg: &str, min: f64| {
        let r = contrast(&v[fg], &v[bg]);
        if r < min {
            failures.push(format!(
                "{theme}: {fg} {} on {bg} {} = {r:.2}:1 < {min}:1",
                v[fg], v[bg]
            ));
        }
    };
    // Text on page/surface backgrounds: AA normal text 4.5:1.
    for bg in ["--ga-bg", "--ga-surface"] {
        for fg in [
            "--ga-text",
            "--ga-muted",
            "--ga-pos",
            "--ga-neg",
            "--ga-warn",
            "--ga-info",
        ] {
            chk(fg, bg, 4.5);
        }
        // UI components / graphical objects / focus ring: 3:1.
        for fg in [
            "--ga-focus",
            "--ga-series-1",
            "--ga-series-2",
            "--ga-series-3",
            "--ga-series-4",
        ] {
            chk(fg, bg, 3.0);
        }
    }
    // Badge text on badge background: 4.5:1.
    for b in BADGES {
        chk(
            &format!("--ga-badge-{b}-fg"),
            &format!("--ga-badge-{b}-bg"),
            4.5,
        );
    }
    assert!(
        failures.is_empty(),
        "contrast failures:\n{}",
        failures.join("\n")
    );
}

#[test]
fn test_contrast_when_light_theme_should_meet_wcag_aa() {
    check_theme(false);
}

#[test]
fn test_contrast_when_dark_theme_should_meet_wcag_aa() {
    check_theme(true);
}

#[test]
fn test_contrast_when_media_dark_set_should_meet_wcag_aa_too() {
    // The prefers-color-scheme block is what most users see; check it directly.
    let t = load();
    for bg in ["--ga-bg", "--ga-surface"] {
        for fg in [
            "--ga-text",
            "--ga-muted",
            "--ga-pos",
            "--ga-neg",
            "--ga-warn",
            "--ga-info",
        ] {
            let r = contrast(&t.dark_media[fg], &t.dark_media[bg]);
            assert!(r >= 4.5, "media dark {fg} on {bg} = {r:.2}");
        }
    }
}

// ───────────────────────── ghost-alpha.css uses tokens ─────────────────────────

fn app_css() -> String {
    strip_comments(&std::fs::read_to_string(CSS_PATH).unwrap())
}

#[test]
fn test_dashboard_css_when_scanned_should_contain_no_hex_colour_literals() {
    let css = app_css();
    let b = css.as_bytes();
    let mut bad = Vec::new();
    for i in 0..b.len() {
        if b[i] == b'#' {
            let run: String = css[i + 1..]
                .chars()
                .take_while(|c| c.is_ascii_hexdigit())
                .collect();
            let next = css[i + 1 + run.len()..].chars().next();
            let boundary = next
                .map(|c| !(c.is_alphanumeric() || c == '-' || c == '_'))
                .unwrap_or(true);
            if matches!(run.len(), 3 | 4 | 6 | 8) && boundary {
                bad.push(format!("#{run}"));
            }
        }
    }
    assert!(bad.is_empty(), "hex literals in ghost-alpha.css: {bad:?}");
}

#[test]
fn test_dashboard_css_when_scanned_should_only_reference_defined_ga_tokens() {
    let css = app_css();
    let tokens = strip_comments(&std::fs::read_to_string(TOKENS_PATH).unwrap());
    let mut used = std::collections::BTreeSet::new();
    let mut rest = css.as_str();
    while let Some(i) = rest.find("var(--ga-") {
        let a = &rest[i + 4..];
        let end = a.find([')', ',']).unwrap();
        used.insert(a[..end].trim().to_string());
        rest = &a[end..];
    }
    assert!(
        used.len() >= 8,
        "ghost-alpha.css should use many --ga tokens, got {used:?}"
    );
    for u in used {
        assert!(
            tokens.contains(&format!("{u}:")),
            "{u} used in ghost-alpha.css but not defined in tokens.css"
        );
    }
}

/// Concatenated declaration bodies of every rule whose selector list (comma-split,
/// pseudo-element suffix included) contains exactly `sel`.
fn rule_bodies(css: &str, sel: &str) -> String {
    let mut rest = css;
    let mut out = String::new();
    while let Some(open) = rest.find('{') {
        let selector = rest[..open].trim().trim_start_matches('}').trim();
        let close = rest[open..].find('}').unwrap() + open;
        let body = &rest[open + 1..close];
        if selector.starts_with('@') {
            // descend into at-rule contents: just skip the at-rule header
            rest = &rest[open + 1..];
            continue;
        }
        if selector.split(',').any(|s| norm(s) == norm(sel)) {
            out.push_str(body);
            out.push(';');
        }
        rest = &rest[close + 1..];
    }
    out
}

#[test]
fn test_badges_when_styled_should_use_per_badge_foreground_and_background_tokens() {
    let css = app_css();
    for b in BADGES {
        let body = rule_bodies(&css, &format!(".badge-{b}"));
        assert!(
            !body.is_empty(),
            ".badge-{b} rule missing in ghost-alpha.css"
        );
        assert!(
            body.contains(&format!("var(--ga-badge-{b}-bg)")),
            ".badge-{b} bg token"
        );
        assert!(
            body.contains(&format!("var(--ga-badge-{b}-fg)")),
            ".badge-{b} fg token"
        );
        // Inline <style> in the shell used to supply the padding; it must live here now.
        assert!(
            body.contains("padding") || !rule_bodies(&css, ".badge").is_empty(),
            ".badge-{b} lost its padding"
        );
    }
}

#[test]
fn test_badges_when_status_colour_used_should_also_carry_an_icon_not_colour_only() {
    let css = app_css();
    let mut icons = std::collections::BTreeSet::new();
    for b in ["bull", "bear", "sideways", "critical", "warning"] {
        let body = rule_bodies(&css, &format!(".badge-{b}::before"));
        let content = body
            .split(';')
            .find_map(|d| {
                d.trim()
                    .strip_prefix("content:")
                    .map(|v| v.trim().to_string())
            })
            .unwrap_or_else(|| panic!(".badge-{b}::before needs a content: icon"));
        assert!(
            content.len() > 2 && content != "\"\"" && content != "''",
            ".badge-{b} icon empty"
        );
        icons.insert(content);
    }
    assert_eq!(icons.len(), 5, "each status badge needs a distinct icon");
}

// ───────────────────────── Pico mapping (M3) and target size (M4) ─────────────────────────

fn combined_css() -> String {
    strip_comments(&std::fs::read_to_string(TOKENS_PATH).unwrap())
        + "\n"
        + &strip_comments(&std::fs::read_to_string(CSS_PATH).unwrap())
}

const PICO_MAP: &[(&str, &str)] = &[
    ("--pico-background-color", "var(--ga-bg)"),
    ("--pico-color", "var(--ga-text)"),
    ("--pico-muted-color", "var(--ga-muted)"),
    ("--pico-card-background-color", "var(--ga-surface)"),
];

/// Blocks (across tokens.css + ghost-alpha.css) that match a Pico theme selector and, if
/// `media_dark`, sit inside `@media (prefers-color-scheme: dark)`; selector lists are split.
fn pico_mapped(selector: &str, media_dark: bool) -> bool {
    let css = combined_css();
    let mut blocks = Vec::new();
    parse_blocks(&css, None, &mut 0, &mut blocks);
    PICO_MAP.iter().all(|(k, v)| {
        blocks.iter().any(|b| {
            let in_media = b
                .media
                .as_deref()
                .is_some_and(|m| m.contains("prefers-color-scheme:dark"));
            in_media == media_dark
                && b.selector.split(',').any(|s| s == selector)
                && b.vars.get(*k).map(|x| norm(x)) == Some(norm(v))
        })
    })
}

#[test]
fn test_pico_when_light_theme_should_map_pico_variables_to_tokens_with_pico_specificity() {
    // Pico's light block is `:root:not([data-theme=dark])`; our mapping must live under the
    // same selector (later in the cascade) or it loses to Pico on specificity.
    assert!(pico_mapped(":root:not([data-theme=dark])", false));
    assert!(pico_mapped("[data-theme=light]", false));
}

#[test]
fn test_pico_when_dark_theme_should_map_pico_variables_for_forced_and_media_dark() {
    assert!(pico_mapped("[data-theme=dark]", false));
    assert!(pico_mapped(":root:not([data-theme])", true));
}

fn len_px(v: &str) -> Option<f64> {
    let v = v.trim();
    if let Some(x) = v.strip_suffix("px") {
        x.trim().parse().ok()
    } else if let Some(x) = v.strip_suffix("rem") {
        x.trim().parse::<f64>().ok().map(|n| n * 16.0)
    } else {
        None
    }
}

/// Largest min-height/min-block-size (px) across rules whose selector list satisfies `pred`.
fn min_height_for(pred: &dyn Fn(&str) -> bool) -> f64 {
    let css = app_css();
    let mut best: f64 = 0.0;
    let mut rest = css.as_str();
    while let Some(open) = rest.find('{') {
        let selector = rest[..open].trim().trim_start_matches('}').trim();
        let close = rest[open..]
            .find('}')
            .map(|c| c + open)
            .unwrap_or(rest.len());
        if !selector.starts_with('@') {
            if selector.split(',').any(|s| pred(s.trim())) {
                for d in rest[open + 1..close].split(';') {
                    if let Some((k, v)) = d.split_once(':') {
                        if matches!(k.trim(), "min-height" | "min-block-size") {
                            best = best.max(len_px(v).unwrap_or(0.0));
                        }
                    }
                }
            }
            rest = &rest[close + 1..];
        } else {
            rest = &rest[open + 1..];
        }
    }
    best
}

#[test]
fn test_target_size_when_nav_links_styled_should_be_at_least_24px_tall() {
    let h =
        min_height_for(&|s| s.contains(".site-nav") && s.split_whitespace().last() == Some("a"));
    assert!(h >= 24.0, "nav links min-height is {h}px, need >= 24px");
}

#[test]
fn test_target_size_when_buttons_styled_should_be_at_least_24px_tall() {
    let h = min_height_for(&|s| s.contains("button"));
    assert!(h >= 24.0, "buttons min-height is {h}px, need >= 24px");
}
