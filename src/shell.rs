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
use crate::text::{percent_decode_into, sanitize_prompt};

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
    /// Exit 1; the plugin may have said why.
    Rejected(Option<String>),
}

/// Interprets one response line (without the trailing newline). Only
/// `"ok"` accepts anything; any other result, known or not, rejects. A
/// password the dialog sends in a mode that doesn't take one is dropped.
pub fn parse_response(mode: Mode, line: &[u8]) -> Result<Outcome, String> {
    let raw: RawResponse =
        serde_json::from_slice(line).map_err(|err| format!("invalid json response: {err}"))?;
    if raw.result != "ok" {
        return Ok(Outcome::Rejected(raw.message.map(|m| sanitize_prompt(&m))));
    }
    match mode {
        Mode::Passphrase => {
            let encoded = raw.password.ok_or("dialog sent no password")?;
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

    fn pass(line: &str) -> Result<Outcome, String> {
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
        assert!(pass(r#"{"result":"ok"}"#).is_err());
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
                assert_eq!(out, Outcome::Rejected(None), "{mode:?} {result:?}");
            }
        }
    }

    #[test]
    fn rejections_carry_a_cleaned_message() {
        let out = parse_response(
            Mode::Confirm,
            "{\"result\":\"error\",\"message\":\"bo\\u001bom\"}".as_bytes(),
        )
        .unwrap();
        assert_eq!(out, Outcome::Rejected(Some("boom".into())));
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
