#![forbid(unsafe_code)]

use std::env;
use std::fs::File;
use std::io::{self, Write};
use std::os::fd::AsFd;
use std::process;

use ssh_askpass_omarchy::request::{Mode, encode_request};
use ssh_askpass_omarchy::secret;
use ssh_askpass_omarchy::shell::{
    MAX_RESPONSE_BYTES, Outcome, connect, parse_response, read_response_line,
};
use zeroize::Zeroizing;

/// Reports a failure and exits. Only our own fixed texts and errors from
/// the operating system get here; nothing the dialog sent does.
fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("ssh-askpass-omarchy: {msg}");
    process::exit(1);
}

fn main() {
    secret::harden();

    let mode = Mode::from_prompt_hint(env::var_os("SSH_ASKPASS_PROMPT").as_deref());
    // The prompt can hold anything (a key comment, say), so it isn't
    // assumed to be UTF-8.
    let prompt = env::args_os()
        .nth(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .unwrap_or_else(|| match mode {
            Mode::Passphrase => "Enter password:".into(),
            Mode::Confirm | Mode::Notify => String::new(),
        });

    let mut stream = match connect() {
        Ok(s) => s,
        // A notification is best-effort: OpenSSH carries on (and the key
        // still waits for its touch) whether or not we show anything.
        Err(_) if mode == Mode::Notify => process::exit(1),
        Err(err) => fail(format_args!("could not connect to shell dialog: {err}")),
    };

    if let Err(err) = stream.write_all(&encode_request(mode, &prompt)) {
        fail(format_args!("failed to send request: {err}"));
    }

    // In notify mode this blocks until OpenSSH kills us with SIGTERM once the
    // key is touched; closing the socket on exit closes the dialog.
    let mut buf = Zeroizing::new(vec![0u8; MAX_RESPONSE_BYTES]);
    let end = match read_response_line(&mut stream, &mut buf) {
        Ok(Some(end)) => end,
        Ok(None) => fail("dialog went away"),
        Err(err) => fail(format_args!("read error: {err}")),
    };

    match parse_response(mode, &buf[..end]) {
        Ok(Outcome::Passphrase(pw)) => {
            // Straight to fd 1 through an unbuffered File: std's stdout
            // buffer would keep a copy of the passphrase around unwiped.
            let stdout = io::stdout().as_fd().try_clone_to_owned().map(File::from);
            let Ok(mut stdout) = stdout else {
                process::exit(1);
            };
            if stdout.write_all(&pw).is_err() {
                process::exit(1);
            }
        }
        Ok(Outcome::Accepted) => {}
        Ok(Outcome::Rejected(Some(reason))) => fail(reason),
        Ok(Outcome::Rejected(None)) => process::exit(1),
        Err(err) => fail(err.describe()),
    }
}
