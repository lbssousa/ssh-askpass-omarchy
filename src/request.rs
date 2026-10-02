use std::ffi::OsStr;

use serde::Serialize;

use crate::text::sanitize_prompt;

/// Protocol version spoken with the plugin; a plugin from another version
/// rejects the request instead of misreading the answer.
pub const PROTOCOL_VERSION: u32 = 2;

/// What OpenSSH wants from this call, from `SSH_ASKPASS_PROMPT`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// No hint: ask for a passphrase or PIN and print it.
    Passphrase,
    /// `confirm` (ssh-agent with `ssh-add -c`): exit 0 allows, anything else denies.
    Confirm,
    /// `none` (e.g. "Confirm user presence for key …"): show the message
    /// until OpenSSH kills us with SIGTERM.
    Notify,
}

impl Mode {
    /// Anything but the two hints OpenSSH sends is a passphrase request,
    /// which is the only mode that never grants anything without the user
    /// typing.
    pub fn from_prompt_hint(hint: Option<&OsStr>) -> Mode {
        match hint.and_then(OsStr::to_str) {
            Some("confirm") => Mode::Confirm,
            Some("none") => Mode::Notify,
            _ => Mode::Passphrase,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Passphrase => "passphrase",
            Mode::Confirm => "confirm",
            Mode::Notify => "notify",
        }
    }
}

#[derive(Serialize)]
struct Request<'a> {
    v: u32,
    mode: &'static str,
    prompt: &'a str,
}

/// One NDJSON request line for the plugin. The prompt is cleaned first
/// (see [`sanitize_prompt`]); the JSON encoding keeps it on one line.
pub fn encode_request(mode: Mode, prompt: &str) -> Vec<u8> {
    let prompt = sanitize_prompt(prompt);
    let req = Request {
        v: PROTOCOL_VERSION,
        mode: mode.as_str(),
        prompt: &prompt,
    };
    // Serializing a struct of strings can't fail.
    let mut line = serde_json::to_vec(&req).expect("request serializes");
    line.push(b'\n');
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStrExt;

    #[test]
    fn mode_from_hint() {
        assert_eq!(Mode::from_prompt_hint(None), Mode::Passphrase);
        assert_eq!(
            Mode::from_prompt_hint(Some(OsStr::new("confirm"))),
            Mode::Confirm
        );
        assert_eq!(
            Mode::from_prompt_hint(Some(OsStr::new("none"))),
            Mode::Notify
        );
    }

    #[test]
    fn unknown_hints_ask_for_a_passphrase() {
        for hint in ["", "CONFIRM", "None", " none", "confirm ", "whatever", "0"] {
            assert_eq!(
                Mode::from_prompt_hint(Some(OsStr::new(hint))),
                Mode::Passphrase,
                "{hint:?}"
            );
        }
        let not_utf8 = OsStr::from_bytes(b"conf\xFFirm");
        assert_eq!(Mode::from_prompt_hint(Some(not_utf8)), Mode::Passphrase);
    }

    #[test]
    fn mode_names_match_the_plugin() {
        assert_eq!(Mode::Passphrase.as_str(), "passphrase");
        assert_eq!(Mode::Confirm.as_str(), "confirm");
        assert_eq!(Mode::Notify.as_str(), "notify");
    }

    #[test]
    fn request_line() {
        let line = encode_request(Mode::Notify, "Confirm user presence for key");
        assert_eq!(
            line,
            b"{\"v\":2,\"mode\":\"notify\",\"prompt\":\"Confirm user presence for key\"}\n"
        );
    }

    #[test]
    fn request_is_a_single_line_whatever_the_prompt_holds() {
        let line = encode_request(Mode::Confirm, "a\nb\r\nc\u{2028}d\"\\e\u{0}");
        assert_eq!(line.iter().filter(|&&b| b == b'\n').count(), 1);
        assert_eq!(line.last(), Some(&b'\n'));
        let v: serde_json::Value = serde_json::from_slice(&line).unwrap();
        assert_eq!(v["prompt"], "a\nb\ncd\"\\e");
        assert_eq!(
            (v["v"].as_u64(), v["mode"].as_str()),
            (Some(2), Some("confirm"))
        );
    }

    #[test]
    fn request_prompt_is_sanitized_and_bounded() {
        let long = "x".repeat(10_000);
        let line = encode_request(Mode::Passphrase, &format!("\u{202E}{long}"));
        let v: serde_json::Value = serde_json::from_slice(&line).unwrap();
        let prompt = v["prompt"].as_str().unwrap();
        assert!(!prompt.contains('\u{202E}'));
        assert_eq!(prompt.len(), crate::text::MAX_PROMPT_BYTES);
    }
}
