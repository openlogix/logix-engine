//! Text forms from docs/tool-api.md (Table 1, logical types).

/// Converts text, such as tool output, to the "multiline text" form: lines
/// separated by LF only, no trailing newline, no terminal color codes, and
/// no control characters other than LF and tab.
pub fn to_multiline_text(s: &str) -> String {
    let s = strip_escape_sequences(s);
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
    let s: String = s
        .chars()
        .filter(|&c| c == '\n' || c == '\t' || !c.is_control())
        .collect();
    s.trim_end_matches('\n').to_string()
}

/// Converts text to the single-line "text" form: line breaks and tabs become
/// spaces, and other control characters and terminal codes are dropped.
pub fn to_text(s: &str) -> String {
    to_multiline_text(s)
        .chars()
        .map(|c| if c == '\n' || c == '\t' { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_string()
}

// Removes ANSI escape sequences: CSI (ESC [ ... final byte), OSC (ESC ] ...
// BEL or ESC \), and two-character escapes.
fn strip_escape_sequences(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&c) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiline_text_uses_lf_and_drops_color_codes() {
        assert_eq!(to_multiline_text("a\r\nb\rc\n\n"), "a\nb\nc");
        assert_eq!(to_multiline_text("\x1b[1;31mred\x1b[0m ok"), "red ok");
        assert_eq!(to_multiline_text("\x1b]0;title\x07x"), "x");
        assert_eq!(to_multiline_text("\x1b]8;;url\x1b\\link"), "link");
        assert_eq!(to_multiline_text("tab\there\x00\x08"), "tab\there");
        assert_eq!(to_multiline_text("ünï 😀"), "ünï 😀");
    }

    #[test]
    fn text_is_a_single_line() {
        assert_eq!(to_text(" a\nb\tc\r\n"), "a b c");
    }
}
