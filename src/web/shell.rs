//! Shared page shell and the single navigation table for the dashboard.
//!
//! Every full page is rendered through [`shell`]; there is exactly one copy of
//! the navigation ([`NAV`]) so pages can never drift apart.

use super::assets::versioned;

/// One navigation link.
pub struct NavItem {
    pub href: &'static str,
    pub label: &'static str,
}

/// A labelled group of navigation links.
pub struct NavGroup {
    pub label: &'static str,
    pub items: &'static [NavItem],
}

impl NavItem {
    pub const fn new(href: &'static str, label: &'static str) -> Self {
        Self { href, label }
    }
}

/// The complete navigation, in display order.
pub const NAV: &[NavGroup] = &[
    NavGroup {
        label: "Portfolio",
        items: &[
            NavItem::new("/portfolio", "Overview"),
            NavItem::new("/allocation", "Allocation"),
            NavItem::new("/concentration-decay", "Concentration Decay"),
            NavItem::new("/tax", "Tax"),
            NavItem::new("/selling", "Selling"),
            NavItem::new("/etf", "ETF"),
            NavItem::new("/correlation", "Correlation"),
        ],
    },
    NavGroup {
        label: "Markets",
        items: &[
            NavItem::new("/regime", "Regime"),
            NavItem::new("/geo-risk", "Geo Risk"),
            NavItem::new("/rotation", "Rotation"),
        ],
    },
    NavGroup {
        label: "Research",
        items: &[
            NavItem::new("/", "Screener"),
            NavItem::new("/watchlist", "Watchlist"),
            NavItem::new("/flagged", "Flagged"),
            NavItem::new("/analytics", "Analytics"),
            NavItem::new("/fundamentals", "Fundamentals"),
            NavItem::new("/valuation", "Valuation"),
        ],
    },
    NavGroup {
        label: "Tools",
        items: &[
            NavItem::new("/sizing", "Sizing"),
            NavItem::new("/optimize", "Optimize"),
            NavItem::new("/strategies", "Strategies"),
            NavItem::new("/decisions", "Decisions"),
        ],
    },
    NavGroup {
        label: "System",
        items: &[
            NavItem::new("/alerts", "Alerts"),
            NavItem::new("/settings", "Settings"),
        ],
    },
];

/// Minimal HTML escaping for text and attribute values; the single escaper
/// shared by every web module.
pub(crate) fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn render_nav(active: &str) -> String {
    let mut out = String::with_capacity(2048);
    for group in NAV {
        out.push_str("<div class=\"nav-group\"><span class=\"nav-group-label\">");
        out.push_str(&html_escape(group.label));
        out.push_str("</span>");
        for item in group.items {
            out.push_str("<a href=\"");
            out.push_str(item.href);
            out.push('"');
            if item.href == active {
                out.push_str(" aria-current=\"page\"");
            }
            out.push('>');
            out.push_str(&html_escape(item.label));
            out.push_str("</a>");
        }
        out.push_str("</div>");
    }
    out
}

/// Wrap `body` in the full HTML document.
///
/// `active` is the route path of the current page; the matching nav link gets
/// `aria-current="page"`. `title` is HTML-escaped and rendered as the page's
/// single `<h1>`; `body` must not contain another `<h1>`.
pub fn shell(active: &str, title: &str, body: &str) -> String {
    let title = html_escape(title);
    let nav = render_nav(active);
    let (pico, tokens, css, htmx, navjs) = (
        versioned("pico.min.css"),
        versioned("tokens.css"),
        versioned("ghost-alpha.css"),
        versioned("htmx.min.js"),
        versioned("nav.js"),
    );
    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Ghost Alpha — {title}</title>
  <link rel="stylesheet" href="{pico}">
  <link rel="stylesheet" href="{tokens}">
  <link rel="stylesheet" href="{css}">
  <script src="{htmx}" defer></script>
  <script src="{navjs}" defer></script>
</head>
<body>
  <a class="skip-link" href="#main-content">Skip to main content</a>
  <header class="container site-header"><strong class="brand">Ghost Alpha</strong></header>
  <nav class="container site-nav" aria-label="Main">
    <details class="nav-menu" open>
      <summary>Menu</summary>
      <div class="nav-groups">{nav}</div>
    </details>
  </nav>
  <main id="main-content" class="container">
    <h1>{title}</h1>
    {body}
  </main>
</body>
</html>"##
    )
}
