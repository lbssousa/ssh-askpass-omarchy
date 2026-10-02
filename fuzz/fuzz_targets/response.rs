//! Parsing the dialog's reply: any bytes must yield an outcome or an
//! error, never a panic, and only an "ok" reply may produce one that lets
//! ssh proceed.

#![no_main]

use libfuzzer_sys::fuzz_target;
use ssh_askpass_omarchy::request::Mode;
use ssh_askpass_omarchy::shell::{MAX_RESPONSE_BYTES, Outcome, parse_response, read_response_line};

fuzz_target!(|data: &[u8]| {
    let mut buf = vec![0u8; MAX_RESPONSE_BYTES];
    let Ok(Some(end)) = read_response_line(&mut &data[..], &mut buf) else {
        return;
    };
    assert!(end <= MAX_RESPONSE_BYTES);
    assert!(!buf[..end].contains(&b'\n'), "a response is one line");

    let granting_text = String::from_utf8_lossy(&buf[..end]).contains("\"ok\"");
    for mode in [Mode::Passphrase, Mode::Confirm, Mode::Notify] {
        let Ok(outcome) = parse_response(mode, &buf[..end]) else {
            continue;
        };
        match outcome {
            Outcome::Passphrase(pw) => {
                assert_eq!(mode, Mode::Passphrase, "only a passphrase prompt prints");
                assert!(granting_text, "a grant needs an \"ok\" result");
                // Percent-decoding only ever shrinks its input.
                assert!(pw.len() <= end);
            }
            Outcome::Accepted => {
                assert_ne!(mode, Mode::Passphrase);
                assert!(granting_text, "a grant needs an \"ok\" result");
            }
            Outcome::Rejected(Some(msg)) => assert!(!msg.contains('\u{1b}')),
            Outcome::Rejected(None) => {}
        }
    }
});
