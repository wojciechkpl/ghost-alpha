//! Mermaid markdown emitter helpers.
//!
//! Strict Mermaid renderers (GitHub web, Obsidian) reject flowchart labels
//! containing `<br/>`, colons, slashes, parens, etc. unless wrapped in double
//! quotes. `quote_label_if_needed` enforces this. The lesson was learned the
//! hard way mid-session — see `analysis/INVESTMENT_STRATEGY.md` history for the
//! original 7 broken Mermaid blocks.

const SPECIAL_CHARS: &[char] = &[
    '<', '>', ':', '?', '&', '/', '(', ')', '%', '#', ',', '|', '=', '+',
];

/// Wrap `label` in double quotes if it contains any character that strict
/// Mermaid renderers reject in unquoted form. Idempotent: already-quoted
/// labels are returned unchanged.
///
/// Characters that trigger quoting:
///   `<`, `>`, `:`, `?`, `&`, `/`, `(`, `)`, `%`, `#`, `,`, `|`, `=`, `+`
///
/// Empty input returns empty string (no quoting).
pub fn quote_label_if_needed(label: &str) -> String {
    if label.is_empty() {
        return String::new();
    }
    // Idempotent: don't double-wrap already-quoted labels.
    if label.starts_with('"') && label.ends_with('"') && label.len() >= 2 {
        return label.to_string();
    }
    if label.chars().any(|c| SPECIAL_CHARS.contains(&c)) {
        format!("\"{}\"", label)
    } else {
        label.to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlowchartShape {
    /// `[...]` rectangle
    Rectangle,
    /// `{...}` diamond / decision
    Diamond,
    /// `(...)` rounded
    Round,
    /// `([...])` stadium
    Stadium,
}

/// Emit a flowchart node declaration: `id[label]`, `id{label}`, etc.
/// Wraps label in quotes if needed.
pub fn flowchart_node(id: &str, label: &str, shape: FlowchartShape) -> String {
    let l = quote_label_if_needed(label);
    match shape {
        FlowchartShape::Rectangle => format!("{}[{}]", id, l),
        FlowchartShape::Diamond => format!("{}{{{}}}", id, l),
        FlowchartShape::Round => format!("{}({})", id, l),
        FlowchartShape::Stadium => format!("{}([{}])", id, l),
    }
}

/// Emit a flowchart edge: `from --> to` or `from -->|edge_label| to` if edge_label is Some.
/// Edge labels do NOT need the same quote treatment as node labels — the `|...|` syntax
/// already delimits, so `:` and other chars are safe inside.
pub fn flowchart_edge(from: &str, to: &str, edge_label: Option<&str>) -> String {
    match edge_label {
        Some(lbl) => format!("{} -->|{}| {}", from, lbl, to),
        None => format!("{} --> {}", from, to),
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use FlowchartShape::{Diamond, Rectangle, Stadium};

    // ── quote_label_if_needed ─────────────────────────────────────────────────

    #[test]
    fn plain_label_returned_unchanged() {
        assert_eq!(quote_label_if_needed("Plain text"), "Plain text");
    }

    #[test]
    fn label_with_colon_quoted() {
        assert_eq!(
            quote_label_if_needed("Annual review: am I above cap?"),
            "\"Annual review: am I above cap?\""
        );
    }

    #[test]
    fn label_with_br_tag_quoted() {
        assert_eq!(
            quote_label_if_needed("Multi<br/>line label"),
            "\"Multi<br/>line label\""
        );
    }

    #[test]
    fn label_with_parens_quoted() {
        assert_eq!(
            quote_label_if_needed("Sleeve build (year 1)"),
            "\"Sleeve build (year 1)\""
        );
    }

    #[test]
    fn label_with_percent_quoted() {
        assert_eq!(
            quote_label_if_needed("AMZN 35% floor"),
            "\"AMZN 35% floor\""
        );
    }

    #[test]
    fn label_with_slash_quoted() {
        assert_eq!(quote_label_if_needed("HOLD/SELL"), "\"HOLD/SELL\"");
    }

    #[test]
    fn already_quoted_label_returned_unchanged() {
        // Idempotent: a label already wrapped in double-quotes must not be double-wrapped.
        assert_eq!(
            quote_label_if_needed("\"Already quoted\""),
            "\"Already quoted\""
        );
    }

    #[test]
    fn empty_label_returned_empty() {
        assert_eq!(quote_label_if_needed(""), "");
    }

    // ── flowchart_node ────────────────────────────────────────────────────────

    #[test]
    fn flowchart_node_rectangle_with_plain_label() {
        assert_eq!(flowchart_node("A", "Plain", Rectangle), "A[Plain]");
    }

    #[test]
    fn flowchart_node_rectangle_with_special_chars_quotes_label() {
        assert_eq!(
            flowchart_node("A", "Annual review: cap?", Rectangle),
            "A[\"Annual review: cap?\"]"
        );
    }

    #[test]
    fn flowchart_node_diamond_uses_curly_braces() {
        // `?` is a special char → label gets quoted inside the `{...}` delimiters.
        assert_eq!(
            flowchart_node("D", "Decision?", Diamond),
            "D{\"Decision?\"}"
        );
    }

    #[test]
    fn flowchart_node_stadium_uses_double_brackets() {
        assert_eq!(flowchart_node("S", "Plain", Stadium), "S([Plain])");
    }

    // ── flowchart_edge ────────────────────────────────────────────────────────

    #[test]
    fn flowchart_edge_no_label() {
        assert_eq!(flowchart_edge("A", "B", None), "A --> B");
    }

    #[test]
    fn flowchart_edge_with_plain_label() {
        assert_eq!(flowchart_edge("A", "B", Some("Yes")), "A -->|Yes| B");
    }

    #[test]
    fn flowchart_edge_with_special_chars_does_not_quote() {
        // Edge labels are delimited by `|...|` which already handles special chars
        // — the Mermaid parser reads until the closing `|`, so `:`, `?`, etc. are
        // safe without additional quoting. No wrapping applied here.
        assert_eq!(
            flowchart_edge("A", "B", Some("Cap hit?")),
            "A -->|Cap hit?| B"
        );
    }

    // ── Integration smoke ─────────────────────────────────────────────────────

    #[test]
    fn composes_full_flowchart_block() {
        // Caller provides the `flowchart TD` header; helpers produce node/edge lines.
        let header = "flowchart TD\n";
        let n_a = flowchart_node("A", "Start<br/>Phase", Rectangle); // special char → quoted
        let n_b = flowchart_node("B", "Evaluate", Diamond);
        let n_c = flowchart_node("C", "Done", Rectangle);
        let e_ab = flowchart_edge("A", "B", None);
        let e_bc = flowchart_edge("B", "C", Some("Yes"));

        let diagram = format!("{header}{n_a}\n{n_b}\n{n_c}\n{e_ab}\n{e_bc}\n");

        assert!(diagram.starts_with("flowchart TD\n"));
        // Node A label contains `<br/>` so must be quoted inside brackets.
        assert!(diagram.contains("A[\"Start<br/>Phase\"]"));
        assert!(diagram.contains("B -->|Yes| C"));
    }
}
