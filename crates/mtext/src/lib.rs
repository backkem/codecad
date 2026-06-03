//! Parser for AutoCAD MText formatting strings.
//!
//! Converts raw MText (inline formatting codes mixed with plain text)
//! into a flat `Vec<MTextSpan>` IR. No rendering, no font handling.

/// Parsed MText document: a sequence of styled text spans.
#[derive(Debug, Clone, PartialEq)]
pub struct MTextDoc {
    pub spans: Vec<MTextSpan>,
}

/// A run of text sharing the same formatting state.
#[derive(Debug, Clone, PartialEq)]
pub struct MTextSpan {
    pub text: String,
    pub style: MTextStyle,
}

/// Formatting state active during a span.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MTextStyle {
    pub underline: bool,
    pub overline: bool,
    pub strikethrough: bool,
    pub font: Option<String>,
    pub bold: bool,
    pub italic: bool,
    pub height: Option<f64>,
    pub height_factor: Option<f64>,
    pub width_factor: Option<f64>,
    pub color: Option<u8>,
    pub oblique: Option<f64>,
}

impl MTextDoc {
    /// Concatenate all span text, ignoring formatting.
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        for span in &self.spans {
            out.push_str(&span.text);
        }
        out
    }
}

/// Parse an MText formatting string into spans.
pub fn parse(input: &str) -> MTextDoc {
    let mut spans: Vec<MTextSpan> = Vec::new();
    let mut style = MTextStyle::default();
    let mut style_stack: Vec<MTextStyle> = Vec::new();
    let mut text_buf = String::new();
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                let code = match chars.peek() {
                    Some(&c) => c,
                    None => break, // trailing backslash, ignore
                };
                match code {
                    // Escape sequences: emit literal character
                    '\\' => {
                        chars.next();
                        text_buf.push('\\');
                    }
                    '{' => {
                        chars.next();
                        text_buf.push('{');
                    }
                    '}' => {
                        chars.next();
                        text_buf.push('}');
                    }

                    // Single-char codes (no semicolon)
                    'P' | 'p' => {
                        chars.next();
                        // Flush current text, emit newline as its own span
                        flush(&mut text_buf, &style, &mut spans);
                        text_buf.push('\n');
                        flush(&mut text_buf, &style, &mut spans);
                    }
                    '~' => {
                        chars.next();
                        text_buf.push(' ');
                    }
                    'N' | 'n' => {
                        chars.next();
                        // Column break, treat as newline for our purposes
                        flush(&mut text_buf, &style, &mut spans);
                        text_buf.push('\n');
                        flush(&mut text_buf, &style, &mut spans);
                    }
                    'X' | 'x' => {
                        chars.next();
                        // Paragraph wrap (dimensions), ignore
                    }

                    // Toggle codes (no arguments, no semicolon)
                    'L' => {
                        chars.next();
                        flush(&mut text_buf, &style, &mut spans);
                        style.underline = true;
                    }
                    'l' => {
                        chars.next();
                        flush(&mut text_buf, &style, &mut spans);
                        style.underline = false;
                    }
                    'O' => {
                        chars.next();
                        flush(&mut text_buf, &style, &mut spans);
                        style.overline = true;
                    }
                    'o' => {
                        chars.next();
                        flush(&mut text_buf, &style, &mut spans);
                        style.overline = false;
                    }
                    'K' => {
                        chars.next();
                        flush(&mut text_buf, &style, &mut spans);
                        style.strikethrough = true;
                    }
                    'k' => {
                        chars.next();
                        flush(&mut text_buf, &style, &mut spans);
                        style.strikethrough = false;
                    }

                    // Parameterized codes (terminated by ';')
                    'f' | 'F' => {
                        chars.next();
                        let param = consume_until_semicolon(&mut chars);
                        flush(&mut text_buf, &style, &mut spans);
                        parse_font_param(&param, &mut style);
                    }
                    'H' | 'h' => {
                        chars.next();
                        let param = consume_until_semicolon(&mut chars);
                        flush(&mut text_buf, &style, &mut spans);
                        parse_height_param(&param, &mut style);
                    }
                    'W' | 'w' => {
                        chars.next();
                        let param = consume_until_semicolon(&mut chars);
                        flush(&mut text_buf, &style, &mut spans);
                        if let Ok(val) = param.trim_end_matches('x').parse::<f64>() {
                            style.width_factor = Some(val);
                        }
                    }
                    'Q' | 'q' => {
                        chars.next();
                        let param = consume_until_semicolon(&mut chars);
                        flush(&mut text_buf, &style, &mut spans);
                        if let Ok(val) = param.parse::<f64>() {
                            style.oblique = Some(val);
                        }
                    }
                    'T' | 't' => {
                        chars.next();
                        let _param = consume_until_semicolon(&mut chars);
                        // Tracking/letter spacing, not stored in style
                    }
                    'C' | 'c' => {
                        chars.next();
                        let param = consume_until_semicolon(&mut chars);
                        flush(&mut text_buf, &style, &mut spans);
                        if let Ok(val) = param.parse::<u8>() {
                            style.color = Some(val);
                        }
                    }
                    'A' | 'a' => {
                        chars.next();
                        let _param = consume_until_semicolon(&mut chars);
                        // Vertical alignment, not stored
                    }
                    'S' | 's' => {
                        chars.next();
                        let param = consume_until_semicolon(&mut chars);
                        // Stacking: render as "top/bottom" plain text
                        if let Some(sep_pos) = param.find(['/', '^', '#']) {
                            let top = &param[..sep_pos];
                            let bottom = &param[sep_pos + 1..];
                            text_buf.push_str(top);
                            text_buf.push('/');
                            text_buf.push_str(bottom);
                        } else {
                            text_buf.push_str(&param);
                        }
                    }

                    // Unknown backslash code: if followed by a letter,
                    // consume until ';' (assume parameterized). Otherwise
                    // just emit the character.
                    _ => {
                        chars.next();
                        if code.is_ascii_alphabetic() {
                            let _param = consume_until_semicolon(&mut chars);
                        }
                        // Non-alphabetic unknown: skip the backslash + char
                    }
                }
            }

            '{' => {
                flush(&mut text_buf, &style, &mut spans);
                style_stack.push(style.clone());
            }
            '}' => {
                flush(&mut text_buf, &style, &mut spans);
                if let Some(prev) = style_stack.pop() {
                    style = prev;
                }
            }

            _ => {
                text_buf.push(ch);
            }
        }
    }

    flush(&mut text_buf, &style, &mut spans);

    MTextDoc { spans }
}

/// Push accumulated text as a span (if non-empty) and clear the buffer.
fn flush(buf: &mut String, style: &MTextStyle, spans: &mut Vec<MTextSpan>) {
    if !buf.is_empty() {
        spans.push(MTextSpan {
            text: std::mem::take(buf),
            style: style.clone(),
        });
    }
}

/// Consume characters until ';' (exclusive) or end of input.
/// Returns the consumed string (without the ';').
fn consume_until_semicolon(chars: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut param = String::new();
    for c in chars.by_ref() {
        if c == ';' {
            break;
        }
        param.push(c);
    }
    param
}

/// Parse `\f` font parameter: `fontname|b0|i0` or just `fontname`.
fn parse_font_param(param: &str, style: &mut MTextStyle) {
    let parts: Vec<&str> = param.split('|').collect();
    if let Some(&name) = parts.first() {
        if !name.is_empty() {
            style.font = Some(name.to_string());
        }
    }
    for part in parts.iter().skip(1) {
        if part.starts_with('b') || part.starts_with('B') {
            style.bold = part.ends_with('1');
        } else if part.starts_with('i') || part.starts_with('I') {
            style.italic = part.ends_with('1');
        }
    }
}

/// Parse `\H` height parameter: `3.5` (absolute) or `0.8x` (relative factor).
fn parse_height_param(param: &str, style: &mut MTextStyle) {
    if param.ends_with('x') {
        if let Ok(val) = param.trim_end_matches('x').parse::<f64>() {
            style.height_factor = Some(val);
            style.height = None;
        }
    } else if let Ok(val) = param.parse::<f64>() {
        style.height = Some(val);
        style.height_factor = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Plain text passthrough ───────────────────────────────

    #[test]
    fn plain_text_passthrough() {
        let doc = parse("Hello world");
        assert_eq!(doc.plain_text(), "Hello world");
        assert_eq!(doc.spans.len(), 1);
        assert_eq!(doc.spans[0].style, MTextStyle::default());
    }

    #[test]
    fn empty_input() {
        let doc = parse("");
        assert_eq!(doc.plain_text(), "");
        assert!(doc.spans.is_empty());
    }

    #[test]
    fn area_label() {
        let doc = parse("18,52 m²");
        assert_eq!(doc.plain_text(), "18,52 m²");
    }

    // ── Toggle codes ────────────────────────────────────────

    #[test]
    fn underline_toggle_preserves_text() {
        // The bug that prompted this crate: \L ate everything after it
        let doc = parse("\\LTUINBERGING");
        assert_eq!(doc.plain_text(), "TUINBERGING");
        assert_eq!(doc.spans.len(), 1);
        assert!(doc.spans[0].style.underline);
    }

    #[test]
    fn underline_on_off() {
        let doc = parse("before\\Lunder\\lafter");
        assert_eq!(doc.plain_text(), "beforeunderafter");
        assert_eq!(doc.spans.len(), 3);
        assert!(!doc.spans[0].style.underline);
        assert!(doc.spans[1].style.underline);
        assert!(!doc.spans[2].style.underline);
    }

    #[test]
    fn overline_toggle() {
        let doc = parse("\\Oover\\orest");
        assert_eq!(doc.plain_text(), "overrest");
        assert!(doc.spans[0].style.overline);
        assert!(!doc.spans[1].style.overline);
    }

    #[test]
    fn strikethrough_toggle() {
        let doc = parse("\\Kstrike\\knormal");
        assert_eq!(doc.plain_text(), "strikenormal");
        assert!(doc.spans[0].style.strikethrough);
        assert!(!doc.spans[1].style.strikethrough);
    }

    #[test]
    fn multiple_toggles_combined() {
        let doc = parse("\\L\\Oboth\\l\\ooff");
        assert_eq!(doc.plain_text(), "bothoff");
        // "both" span has underline + overline
        let both_span = doc.spans.iter().find(|s| s.text == "both").unwrap();
        assert!(both_span.style.underline);
        assert!(both_span.style.overline);
        // "off" span has neither
        let off_span = doc.spans.iter().find(|s| s.text == "off").unwrap();
        assert!(!off_span.style.underline);
        assert!(!off_span.style.overline);
    }

    // ── Parameterized codes ─────────────────────────────────

    #[test]
    fn font_with_bold() {
        let doc = parse("\\fArial|b1|i0;Bold text");
        assert_eq!(doc.plain_text(), "Bold text");
        assert_eq!(doc.spans[0].style.font.as_deref(), Some("Arial"));
        assert!(doc.spans[0].style.bold);
        assert!(!doc.spans[0].style.italic);
    }

    #[test]
    fn font_with_italic() {
        let doc = parse("\\fTimes New Roman|b0|i1;Italic");
        assert_eq!(doc.plain_text(), "Italic");
        assert_eq!(
            doc.spans[0].style.font.as_deref(),
            Some("Times New Roman")
        );
        assert!(!doc.spans[0].style.bold);
        assert!(doc.spans[0].style.italic);
    }

    #[test]
    fn font_name_only() {
        let doc = parse("\\fCourier;mono");
        assert_eq!(doc.plain_text(), "mono");
        assert_eq!(doc.spans[0].style.font.as_deref(), Some("Courier"));
    }

    #[test]
    fn height_absolute() {
        let doc = parse("\\H3.5;tall");
        assert_eq!(doc.plain_text(), "tall");
        assert_eq!(doc.spans[0].style.height, Some(3.5));
        assert_eq!(doc.spans[0].style.height_factor, None);
    }

    #[test]
    fn height_relative() {
        let doc = parse("\\H0.8x;small");
        assert_eq!(doc.plain_text(), "small");
        assert_eq!(doc.spans[0].style.height_factor, Some(0.8));
        assert_eq!(doc.spans[0].style.height, None);
    }

    #[test]
    fn width_factor() {
        let doc = parse("\\W1.5;wide");
        assert_eq!(doc.plain_text(), "wide");
        assert_eq!(doc.spans[0].style.width_factor, Some(1.5));
    }

    #[test]
    fn color_index() {
        let doc = parse("\\C1;red");
        assert_eq!(doc.plain_text(), "red");
        assert_eq!(doc.spans[0].style.color, Some(1));
    }

    #[test]
    fn oblique_angle() {
        let doc = parse("\\Q15;slanted");
        assert_eq!(doc.plain_text(), "slanted");
        assert_eq!(doc.spans[0].style.oblique, Some(15.0));
    }

    // ── Line breaks ─────────────────────────────────────────

    #[test]
    fn line_break() {
        let doc = parse("line1\\Pline2");
        assert_eq!(doc.plain_text(), "line1\nline2");
    }

    #[test]
    fn multiple_line_breaks() {
        let doc = parse("a\\P\\Pb");
        assert_eq!(doc.plain_text(), "a\n\nb");
    }

    #[test]
    fn non_breaking_space() {
        let doc = parse("hello\\~world");
        assert_eq!(doc.plain_text(), "hello world");
    }

    // ── Escape sequences ────────────────────────────────────

    #[test]
    fn escaped_backslash() {
        let doc = parse("path\\\\file");
        assert_eq!(doc.plain_text(), "path\\file");
    }

    #[test]
    fn escaped_braces() {
        let doc = parse("\\{text\\}");
        assert_eq!(doc.plain_text(), "{text}");
    }

    // ── Grouping ────────────────────────────────────────────

    #[test]
    fn group_reverts_style() {
        let doc = parse("before{\\LUNDERLINED}after");
        assert_eq!(doc.plain_text(), "beforeUNDERLINEDafter");
        let underlined = doc.spans.iter().find(|s| s.text == "UNDERLINED").unwrap();
        assert!(underlined.style.underline);
        let after = doc.spans.iter().find(|s| s.text == "after").unwrap();
        assert!(!after.style.underline);
    }

    #[test]
    fn group_reverts_font() {
        let doc = parse("plain{\\fArial|b1;bold}plain2");
        assert_eq!(doc.plain_text(), "plainboldplain2");
        let bold = doc.spans.iter().find(|s| s.text == "bold").unwrap();
        assert!(bold.style.bold);
        assert_eq!(bold.style.font.as_deref(), Some("Arial"));
        let p2 = doc.spans.iter().find(|s| s.text == "plain2").unwrap();
        assert!(!p2.style.bold);
        assert_eq!(p2.style.font, None);
    }

    #[test]
    fn nested_groups() {
        let doc = parse("a{\\Lb{\\Oc}d}e");
        assert_eq!(doc.plain_text(), "abcde");
        // a: no formatting
        let a = doc.spans.iter().find(|s| s.text == "a").unwrap();
        assert!(!a.style.underline);
        assert!(!a.style.overline);
        // b: underline
        let b = doc.spans.iter().find(|s| s.text == "b").unwrap();
        assert!(b.style.underline);
        assert!(!b.style.overline);
        // c: underline + overline
        let c = doc.spans.iter().find(|s| s.text == "c").unwrap();
        assert!(c.style.underline);
        assert!(c.style.overline);
        // d: back to underline only
        let d = doc.spans.iter().find(|s| s.text == "d").unwrap();
        assert!(d.style.underline);
        assert!(!d.style.overline);
        // e: no formatting
        let e = doc.spans.iter().find(|s| s.text == "e").unwrap();
        assert!(!e.style.underline);
        assert!(!e.style.overline);
    }

    #[test]
    fn empty_group() {
        let doc = parse("before{}after");
        assert_eq!(doc.plain_text(), "beforeafter");
    }

    // ── Stacking / fractions ────────────────────────────────

    #[test]
    fn stacking_slash() {
        let doc = parse("\\S1/2;");
        assert_eq!(doc.plain_text(), "1/2");
    }

    #[test]
    fn stacking_caret() {
        let doc = parse("\\S3^4;");
        assert_eq!(doc.plain_text(), "3/4");
    }

    // ── Real-world DWG strings ──────────────────────────────

    #[test]
    fn dwg_area_label() {
        let doc = parse("18,52 m²");
        assert_eq!(doc.plain_text(), "18,52 m²");
    }

    #[test]
    fn dwg_underlined_room() {
        let doc = parse("\\LTUINBERGING");
        assert_eq!(doc.plain_text(), "TUINBERGING");
        assert!(doc.spans[0].style.underline);
    }

    #[test]
    fn dwg_autobergplaats() {
        let doc = parse("\\LAUTOBERGPLAATS");
        assert_eq!(doc.plain_text(), "AUTOBERGPLAATS");
    }

    #[test]
    fn dwg_leefruimte() {
        let doc = parse("\\LLEEFRUIMTE");
        assert_eq!(doc.plain_text(), "LEEFRUIMTE");
    }

    #[test]
    fn dwg_bold_room_with_subtext() {
        let doc = parse("{\\fArial|b1;Room Name}\\Psubtext");
        assert_eq!(doc.plain_text(), "Room Name\nsubtext");
        let room = doc.spans.iter().find(|s| s.text == "Room Name").unwrap();
        assert!(room.style.bold);
        assert_eq!(room.style.font.as_deref(), Some("Arial"));
        let sub = doc.spans.iter().find(|s| s.text == "subtext").unwrap();
        assert!(!sub.style.bold);
    }

    #[test]
    fn dwg_complex_multiline() {
        // Bold title, newline, underlined room, newline, area
        let input = "{\\fArial|b1;TITLE}\\P\\LROOM\\P18,52 m²";
        let doc = parse(input);
        assert_eq!(doc.plain_text(), "TITLE\nROOM\n18,52 m²");
    }

    // ── Edge cases ──────────────────────────────────────────

    #[test]
    fn trailing_backslash() {
        let doc = parse("text\\");
        assert_eq!(doc.plain_text(), "text");
    }

    #[test]
    fn unmatched_close_brace() {
        // Gracefully handle extra closing brace
        let doc = parse("text}more");
        assert_eq!(doc.plain_text(), "textmore");
    }

    #[test]
    fn consecutive_format_codes() {
        let doc = parse("\\fArial;\\H2.5;\\C3;styled");
        assert_eq!(doc.plain_text(), "styled");
        assert_eq!(doc.spans[0].style.font.as_deref(), Some("Arial"));
        assert_eq!(doc.spans[0].style.height, Some(2.5));
        assert_eq!(doc.spans[0].style.color, Some(3));
    }

    #[test]
    fn format_code_mid_word() {
        let doc = parse("hel\\Llo");
        assert_eq!(doc.plain_text(), "hello");
        // "hel" is unstyled, "lo" is underlined
        assert_eq!(doc.spans[0].text, "hel");
        assert!(!doc.spans[0].style.underline);
        assert_eq!(doc.spans[1].text, "lo");
        assert!(doc.spans[1].style.underline);
    }

    #[test]
    fn column_break() {
        let doc = parse("col1\\Ncol2");
        assert_eq!(doc.plain_text(), "col1\ncol2");
    }

    #[test]
    fn deeply_nested_groups() {
        let doc = parse("a{b{c{d}c2}b2}a2");
        assert_eq!(doc.plain_text(), "abcdc2b2a2");
    }

    #[test]
    fn plain_text_method_joins_all() {
        let doc = parse("\\Lunder\\lplain\\Oover");
        let pt = doc.plain_text();
        assert_eq!(pt, "underplainover");
    }
}
