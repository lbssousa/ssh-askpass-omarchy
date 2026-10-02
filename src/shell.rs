//! The connection to the dialog plugin running inside omarchy-shell.

use std::env;
use std::io::{self, Read};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use nix::sys::socket::{getsockopt, sockopt::PeerCredentials};
use nix::unistd::getuid;
use serde::Deserialize;
use zeroize::Zeroizing;

use crate::request::Mode;
use crate::text::percent_decode_into;

pub const MAX_RESPONSE_BYTES: usize = 64 * 1024;

pub fn socket_path() -> PathBuf {
    if let Some(p) = env::var_os("SSH_ASKPASS_OMARCHY_SOCKET") {
        return PathBuf::from(p);
    }
    let runtime = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", getuid())));
    runtime.join("ssh-askpass-omarchy.sock")
}

/// Connects and makes sure the listening end belongs to this user.
pub fn connect() -> io::Result<UnixStream> {
    let stream = UnixStream::connect(socket_path())?;
    let creds = getsockopt(&stream, PeerCredentials).map_err(io::Error::from)?;
    if creds.uid() != getuid().as_raw() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "ssh-askpass socket is owned by another user",
        ));
    }
    Ok(stream)
}

/// Reads up to the first newline into `buf` and returns the line's length.
/// `Ok(None)` means the plugin closed the connection without answering. The
/// caller supplies a fixed, zeroized buffer: no reallocation and no
/// BufReader, so no stray copies of the passphrase.
pub fn read_response_line(stream: &mut impl Read, buf: &mut [u8]) -> io::Result<Option<usize>> {
    let mut len = 0;
    loop {
        if let Some(i) = buf[..len].iter().position(|&b| b == b'\n') {
            return Ok(Some(i));
        }
        if len == buf.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "response too long",
            ));
        }
        match stream.read(&mut buf[len..]) {
            Ok(0) if len == 0 => return Ok(None),
            Ok(0) => return Ok(Some(len)),
            Ok(n) => len += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
}

/// The dialog's answer. `password` arrives percent-encoded (UTF-8), so the
/// JSON never needs unescaping and serde borrows it straight from the
/// zeroized read buffer instead of copying it. A string with JSON escapes
/// can't be borrowed, so serde refuses it.
#[derive(Deserialize)]
struct RawResponse<'a> {
    result: &'a str,
    #[serde(borrow)]
    password: Option<&'a str>,
    message: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Print these bytes (the passphrase) and exit 0.
    Passphrase(Zeroizing<Vec<u8>>),
    /// Exit 0 without printing anything.
    Accepted,
    /// Exit 1. The reason, if there is one, is picked from a fixed set:
    /// nothing the dialog sent is ever echoed to stderr.
    Rejected(Option<Reason>),
}

/// Why the dialog refused. A plain enum, so the text shown for it is always
/// one of our own literals (see [`Reason::text`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    Busy,
    TooLong,
    Unsupported,
    Other,
}

impl Reason {
    pub fn text(self) -> &'static str {
        match self {
            Reason::Busy => "another dialog is already open",
            Reason::TooLong => "the prompt is too long",
            Reason::Unsupported => "the dialog doesn't understand this version of the request",
            Reason::Other => "the dialog reported an error",
        }
    }
}

/// A reply that can't be used.
#[derive(Debug, PartialEq, Eq)]
pub enum ResponseError {
    /// Not the JSON object the protocol defines (or the password has JSON
    /// escapes, which are refused on purpose).
    Invalid,
    /// An "ok" for a passphrase prompt, with nothing to print.
    NothingToPrint,
}

impl ResponseError {
    pub fn describe(&self) -> &'static str {
        match self {
            ResponseError::Invalid => "invalid response from the dialog",
            ResponseError::NothingToPrint => "the dialog accepted the prompt without an answer",
        }
    }
}

/// Why the dialog refused. Only the messages the plugin is known to send
/// map to something specific.
fn reject_reason(result: &str, message: Option<&str>) -> Option<Reason> {
    match (result, message) {
        ("busy", _) => Some(Reason::Busy),
        (_, Some("request too large" | "prompt too long")) => Some(Reason::TooLong),
        (_, Some("invalid JSON" | "unsupported request")) => Some(Reason::Unsupported),
        (_, Some(_)) => Some(Reason::Other),
        (_, None) => None,
    }
}

/// Interprets one response line (without the trailing newline). Only
/// `"ok"` accepts anything; any other result, known or not, rejects. A
/// password the dialog sends in a mode that doesn't take one is dropped.
pub fn parse_response(mode: Mode, line: &[u8]) -> Result<Outcome, ResponseError> {
    let raw: RawResponse = serde_json::from_slice(line).map_err(|_| ResponseError::Invalid)?;
    if raw.result != "ok" {
        return Ok(Outcome::Rejected(reject_reason(
            raw.result,
            raw.message.as_deref(),
        )));
    }
    match mode {
        Mode::Passphrase => {
            let Some(encoded) = raw.password else {
                return Err(ResponseError::NothingToPrint);
            };
            let mut pw = Zeroizing::new(Vec::with_capacity(encoded.len()));
            percent_decode_into(encoded.as_bytes(), &mut pw);
            Ok(Outcome::Passphrase(pw))
        }
        Mode::Confirm | Mode::Notify => Ok(Outcome::Accepted),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pass(line: &str) -> Result<Outcome, ResponseError> {
        parse_response(Mode::Passphrase, line.as_bytes())
    }

    #[test]
    fn passphrase_is_percent_decoded() {
        let out = pass(r#"{"result":"ok","password":"p%C3%A1ss%25"}"#).unwrap();
        assert_eq!(
            out,
            Outcome::Passphrase(Zeroizing::new("páss%".as_bytes().to_vec()))
        );
    }

    #[test]
    fn decodes_tricky_passphrases_exactly() {
        // encodeURIComponent leaves only A-Z a-z 0-9 - _ . ! ~ * ' ( ) as is.
        let encode = |s: &str| {
            s.bytes().fold(String::new(), |mut out, b| {
                if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
                    out.push(b as char);
                } else {
                    out.push_str(&format!("%{b:02X}"));
                }
                out
            })
        };
        for pw in [
            "123456",
            "a%b\"c\\d\ne",
            "sénha çom acentos",
            "🔑🗝️",
            "%25%",
            " ",
            "",
        ] {
            let line = format!(r#"{{"result":"ok","password":"{}"}}"#, encode(pw));
            let Outcome::Passphrase(got) = pass(&line).unwrap() else {
                panic!("{pw:?}: not a passphrase");
            };
            assert_eq!(got.as_slice(), pw.as_bytes(), "{pw:?}");
        }
    }

    #[test]
    fn an_empty_passphrase_is_still_a_passphrase() {
        let out = pass(r#"{"result":"ok","password":""}"#).unwrap();
        assert_eq!(out, Outcome::Passphrase(Zeroizing::new(Vec::new())));
    }

    #[test]
    fn ok_without_a_password_is_an_error_in_passphrase_mode() {
        assert_eq!(
            pass(r#"{"result":"ok"}"#),
            Err(ResponseError::NothingToPrint)
        );
    }

    #[test]
    fn escaped_passwords_are_refused() {
        // An escaped string can't be borrowed: refusing it keeps unescaping
        // (and its unzeroized scratch copy) out of the picture.
        assert!(pass(r#"{"result":"ok","password":"a\"b"}"#).is_err());
        assert!(pass(r#"{"result":"ok","password":"a\u0041"}"#).is_err());
    }

    #[test]
    fn confirm_ok_prints_nothing() {
        let out = parse_response(Mode::Confirm, br#"{"result":"ok","password":"x"}"#).unwrap();
        assert_eq!(out, Outcome::Accepted);
        let out = parse_response(Mode::Confirm, br#"{"result":"ok"}"#).unwrap();
        assert_eq!(out, Outcome::Accepted);
    }

    #[test]
    fn notify_ok_is_accepted() {
        let out = parse_response(Mode::Notify, br#"{"result":"ok"}"#).unwrap();
        assert_eq!(out, Outcome::Accepted);
    }

    #[test]
    fn anything_but_ok_is_rejected_in_every_mode() {
        for mode in [Mode::Passphrase, Mode::Confirm, Mode::Notify] {
            for result in [
                "cancel", "busy", "error", "timeout", "OK", "Ok", "okay", "", "allow",
            ] {
                let line = format!(r#"{{"result":"{result}","password":"x"}}"#);
                let out = parse_response(mode, line.as_bytes()).unwrap();
                let rejected = matches!(out, Outcome::Rejected(_));
                assert!(rejected, "{mode:?} {result:?}");
            }
        }
    }

    #[test]
    fn rejections_are_explained_in_our_own_words() {
        let reason = |line: &str| match parse_response(Mode::Confirm, line.as_bytes()) {
            Ok(Outcome::Rejected(reason)) => reason,
            _ => panic!("not a rejection: {line}"),
        };
        assert_eq!(reason(r#"{"result":"cancel"}"#), None);
        assert_eq!(reason(r#"{"result":"busy"}"#), Some(Reason::Busy));
        assert_eq!(
            reason(r#"{"result":"error","message":"prompt too long"}"#),
            Some(Reason::TooLong)
        );
        assert_eq!(
            reason(r#"{"result":"error","message":"unsupported request"}"#),
            Some(Reason::Unsupported)
        );
        assert_eq!(Reason::Busy.text(), "another dialog is already open");
    }

    #[test]
    fn what_the_dialog_says_is_never_repeated() {
        // The text could hold anything, so only fixed texts leave the process.
        let line = "{\"result\":\"error\",\"message\":\"boom \\u001b[2J\"}";
        let out = parse_response(Mode::Confirm, line.as_bytes()).unwrap();
        assert_eq!(out, Outcome::Rejected(Some(Reason::Other)));
        assert_eq!(Reason::Other.text(), "the dialog reported an error");
    }

    #[test]
    fn malformed_responses_are_errors() {
        for line in [
            "",
            "nope",
            "[]",
            "null",
            r#"{}"#,
            r#"{"result":1}"#,
            r#"{"result":null}"#,
            r#"{"password":"x"}"#,
            r#"{"result":"ok","password":1}"#,
        ] {
            assert!(
                parse_response(Mode::Passphrase, line.as_bytes()).is_err(),
                "{line:?}"
            );
            assert!(
                parse_response(Mode::Confirm, line.as_bytes()).is_err(),
                "{line:?}"
            );
        }
    }

    #[test]
    fn reads_the_first_line() {
        let mut buf = [0u8; 64];
        let mut input: &[u8] = b"{\"result\":\"ok\"}\nrest";
        let n = read_response_line(&mut input, &mut buf).unwrap().unwrap();
        assert_eq!(&buf[..n], b"{\"result\":\"ok\"}");
    }

    #[test]
    fn accepts_a_line_closed_without_newline() {
        let mut buf = [0u8; 64];
        let mut input: &[u8] = br#"{"result":"cancel"}"#;
        let n = read_response_line(&mut input, &mut buf).unwrap().unwrap();
        assert_eq!(&buf[..n], br#"{"result":"cancel"}"#);
    }

    #[test]
    fn eof_without_data_is_none() {
        let mut buf = [0u8; 8];
        let mut input: &[u8] = b"";
        assert_eq!(read_response_line(&mut input, &mut buf).unwrap(), None);
    }

    #[test]
    fn overlong_responses_are_errors() {
        let mut buf = [0u8; 4];
        let mut input: &[u8] = b"abcdef";
        let err = read_response_line(&mut input, &mut buf).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_response_may_arrive_in_pieces() {
        struct Dribble<'a>(&'a [u8]);
        impl Read for Dribble<'_> {
            fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
                let n = self.0.len().min(1).min(out.len());
                out[..n].copy_from_slice(&self.0[..n]);
                self.0 = &self.0[n..];
                Ok(n)
            }
        }
        let mut buf = [0u8; 64];
        let n = read_response_line(&mut Dribble(b"{\"result\":\"ok\"}\n"), &mut buf)
            .unwrap()
            .unwrap();
        assert_eq!(&buf[..n], b"{\"result\":\"ok\"}");
    }

    #[test]
    fn interrupted_reads_are_retried() {
        struct Flaky(u8);
        impl Read for Flaky {
            fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
                self.0 += 1;
                if self.0 == 1 {
                    return Err(io::ErrorKind::Interrupted.into());
                }
                out[..2].copy_from_slice(b"x\n");
                Ok(2)
            }
        }
        let mut buf = [0u8; 8];
        assert_eq!(
            read_response_line(&mut Flaky(0), &mut buf).unwrap(),
            Some(1)
        );
    }

    #[test]
    fn other_read_errors_are_passed_on() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::ErrorKind::ConnectionReset.into())
            }
        }
        let mut buf = [0u8; 8];
        let err = read_response_line(&mut Broken, &mut buf).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::ConnectionReset);
    }
}
