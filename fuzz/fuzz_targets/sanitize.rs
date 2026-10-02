//! Cleaning the prompt: whatever text comes in, what the dialog gets has
//! no control characters (but newlines and tabs), no text-reordering
//! characters, stays within its limits and is stable under a second pass.

#![no_main]

use libfuzzer_sys::fuzz_target;
use ssh_askpass_omarchy::request::{Mode, encode_request};
use ssh_askpass_omarchy::text::{MAX_PROMPT_BYTES, MAX_PROMPT_LINES, sanitize_prompt};

fuzz_target!(|text: &str| {
    let out = sanitize_prompt(text);
    assert!(out.len() <= MAX_PROMPT_BYTES);
    assert!(out.lines().count() <= MAX_PROMPT_LINES);
    assert!(!out.contains('\r'));
    assert!(!out.contains("\n\n"), "no blank lines");
    assert!(!out.starts_with('\n'));
    assert!(!out.ends_with(char::is_whitespace));
    for c in out.chars() {
        assert!(c == '\n' || c == '\t' || !c.is_control(), "U+{:04X}", c as u32);
        assert!(
            !matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200B}'..='\u{200F}'),
            "U+{:04X}",
            c as u32
        );
    }
    assert_eq!(sanitize_prompt(&out), out);

    // The request is always exactly one JSON line.
    for mode in [Mode::Passphrase, Mode::Confirm, Mode::Notify] {
        let line = encode_request(mode, text);
        assert_eq!(line.iter().filter(|&&b| b == b'\n').count(), 1);
        assert_eq!(line.last(), Some(&b'\n'));
    }
});
