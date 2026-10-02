//! Text handling shared by the binary and the fuzz targets: cleaning what
//! the dialog displays, and decoding the passphrase it sends back.

/// Longest prompt forwarded to the dialog, in bytes. OpenSSH prompts are a
/// line or two; this leaves room for a long key path and comment.
pub const MAX_PROMPT_BYTES: usize = 2000;

/// Most lines of a prompt that reach the dialog. A prompt can carry text
/// from outside (a key's comment, a destination constraint from a
/// forwarded agent), and a pile of newlines would push the real question
/// out of the card.
pub const MAX_PROMPT_LINES: usize = 6;

/// Characters that are invisible or reorder the text around them, and so
/// can disguise what a prompt says.
fn is_deceptive(c: char) -> bool {
    matches!(
        c,
        // Bidirectional marks and overrides.
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
        // Zero-width characters, word joiner and invisible operators.
        | '\u{200B}'..='\u{200D}' | '\u{2060}'..='\u{2064}' | '\u{FEFF}'
        // Line and paragraph separators (not "control", yet they break lines).
        | '\u{2028}' | '\u{2029}'
        // Tag characters, which render as nothing.
        | '\u{E0000}'..='\u{E007F}'
    )
}

/// Cleans text that comes from outside before the dialog shows it:
/// control characters (newlines and tabs are kept) and deceptive Unicode
/// are removed, carriage returns count as line breaks, runs of blank lines
/// collapse, and the result is cut at a character boundary to at most
/// [`MAX_PROMPT_LINES`] lines and [`MAX_PROMPT_BYTES`] bytes.
pub fn sanitize_prompt(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(MAX_PROMPT_BYTES));
    let mut lines = 1;
    let mut chars = text.chars().peekable();
    while let Some(mut c) = chars.next() {
        if c == '\r' {
            // CRLF is one break; a lone CR is a break too.
            if chars.peek() == Some(&'\n') {
                continue;
            }
            c = '\n';
        }
        if c == '\n' {
            // Blank lines don't help anyone read a prompt.
            if out.is_empty() || out.ends_with('\n') {
                continue;
            }
            if lines == MAX_PROMPT_LINES {
                break;
            }
            lines += 1;
        } else if (c.is_control() && c != '\t') || is_deceptive(c) {
            continue;
        }
        if out.len() + c.len_utf8() > MAX_PROMPT_BYTES {
            break;
        }
        out.push(c);
    }
    out.truncate(out.trim_end().len());
    out
}

/// Undoes JavaScript's `encodeURIComponent`: `%XX` becomes a byte and
/// everything else is copied as it is. A `%` that doesn't start two hex
/// digits stays a literal `%`, so malformed input is never dropped silently.
pub fn percent_decode_into(bytes: &[u8], out: &mut Vec<u8>) {
    let hex = |b: u8| char::from(b).to_digit(16);
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(&[h1, h2]) = bytes.get(i + 1..i + 3)
            && let (Some(n1), Some(n2)) = (hex(h1), hex(h2))
        {
            out.push((n1 << 4 | n2) as u8);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_ordinary_prompts() {
        let p = "Enter passphrase for key '/home/u/.ssh/id_ed25519' (u@host): ";
        assert_eq!(sanitize_prompt(p), p.trim_end());
        assert_eq!(
            sanitize_prompt("Senha de açúcar: 🔑"),
            "Senha de açúcar: 🔑"
        );
        assert_eq!(sanitize_prompt(""), "");
    }

    #[test]
    fn keeps_newlines_and_tabs_between_text() {
        assert_eq!(
            sanitize_prompt("Allow use of key k?\nKey fingerprint SHA256:x."),
            "Allow use of key k?\nKey fingerprint SHA256:x."
        );
        assert_eq!(sanitize_prompt("a\tb"), "a\tb");
    }

    #[test]
    fn strips_control_characters() {
        assert_eq!(
            sanitize_prompt("a\u{0}b\u{7}c\u{1b}[31md\u{7f}e"),
            "abc[31mde"
        );
        assert_eq!(sanitize_prompt("a\u{85}b\u{9b}c"), "abc");
    }

    #[test]
    fn carriage_returns_are_line_breaks_not_overwrites() {
        assert_eq!(sanitize_prompt("one\r\ntwo"), "one\ntwo");
        assert_eq!(sanitize_prompt("Allow?\rDeny?"), "Allow?\nDeny?");
    }

    #[test]
    fn strips_bidi_and_invisible_characters() {
        for c in [
            '\u{202E}',
            '\u{202A}',
            '\u{2066}',
            '\u{2069}',
            '\u{200E}',
            '\u{200F}',
            '\u{061C}',
            '\u{200B}',
            '\u{200D}',
            '\u{2060}',
            '\u{2064}',
            '\u{FEFF}',
            '\u{2028}',
            '\u{2029}',
            '\u{E0041}',
            '\u{E007F}',
        ] {
            assert_eq!(
                sanitize_prompt(&format!("a{c}b")),
                "ab",
                "U+{:04X}",
                c as u32
            );
        }
    }

    #[test]
    fn collapses_blank_lines_and_trims() {
        assert_eq!(sanitize_prompt("\n\n  a\n\n\n\nb\n\n"), "  a\nb");
        assert_eq!(sanitize_prompt("\n\n\n"), "");
        assert_eq!(sanitize_prompt("a   "), "a");
    }

    #[test]
    fn limits_the_number_of_lines() {
        let many = (1..=20)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let out = sanitize_prompt(&many);
        assert_eq!(out.lines().count(), MAX_PROMPT_LINES);
        assert!(out.ends_with(&format!("line {MAX_PROMPT_LINES}")));
    }

    #[test]
    fn limits_the_length_at_a_character_boundary() {
        let out = sanitize_prompt(&"é".repeat(MAX_PROMPT_BYTES));
        assert_eq!(out.len(), MAX_PROMPT_BYTES);
        assert!(out.chars().all(|c| c == 'é'));
        let out = sanitize_prompt(&format!("{}🔑", "a".repeat(MAX_PROMPT_BYTES - 2)));
        assert_eq!(
            out.len(),
            MAX_PROMPT_BYTES - 2,
            "the 4-byte char doesn't fit"
        );
    }

    #[test]
    fn sanitizing_twice_changes_nothing() {
        for text in ["a\u{202E}b\r\n\r\nc", "\n\n x \u{0}\n", "é\u{200B}\t"] {
            let once = sanitize_prompt(text);
            assert_eq!(sanitize_prompt(&once), once);
        }
    }

    fn decode(s: &str) -> Vec<u8> {
        let mut out = Vec::new();
        percent_decode_into(s.as_bytes(), &mut out);
        out
    }

    #[test]
    fn decodes_percent_escapes() {
        assert_eq!(decode("p%C3%A1ss%25"), "páss%".as_bytes());
        assert_eq!(decode("%41%61"), b"Aa");
        assert_eq!(decode("%0a%00"), b"\n\0");
        assert_eq!(decode(""), b"");
    }

    #[test]
    fn leaves_malformed_escapes_alone() {
        assert_eq!(decode("%"), b"%");
        assert_eq!(decode("%4"), b"%4");
        assert_eq!(decode("%G1"), b"%G1");
        assert_eq!(decode("100%"), b"100%");
        assert_eq!(decode("%%41"), b"%A");
    }

    #[test]
    fn decoding_is_not_recursive() {
        assert_eq!(decode("%2541"), b"%41");
    }

    #[test]
    fn decodes_to_non_utf8_bytes_unchanged() {
        assert_eq!(decode("%FF%FE"), [0xFF, 0xFE]);
    }
}
