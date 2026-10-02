//! Runs the real binary against a fake dialog plugin on a throwaway socket.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixListener;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_ssh-askpass-omarchy");

/// What the fake dialog does with the next request.
enum Answer {
    /// Replies with this line (a newline is added) and closes, like the plugin.
    Line(String),
    /// Writes these bytes as they are and closes.
    Raw(Vec<u8>),
    /// Reads the request, then closes without answering.
    Close,
    /// Reads the request and keeps the connection open until the client goes.
    Hang,
}

struct FakeDialog {
    socket: PathBuf,
    requests: mpsc::Receiver<serde_json::Value>,
    /// Receives () once the client has closed its end after a `Hang`.
    released: mpsc::Receiver<()>,
    _dir: tempfile::TempDir,
}

fn fake_dialog(answer: Answer) -> FakeDialog {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("dialog.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let (tx, requests) = mpsc::channel();
    let (released_tx, released) = mpsc::channel();
    thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut line = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut line)
            .unwrap();
        let _ = tx.send(serde_json::from_str(&line).unwrap_or(serde_json::Value::Null));
        match answer {
            Answer::Line(l) => {
                let _ = stream.write_all(format!("{l}\n").as_bytes());
            }
            Answer::Raw(bytes) => {
                let _ = stream.write_all(&bytes);
            }
            Answer::Close => {}
            Answer::Hang => {
                let mut sink = [0u8; 16];
                while matches!(stream.read(&mut sink), Ok(n) if n > 0) {}
                let _ = released_tx.send(());
            }
        }
    });
    FakeDialog {
        socket,
        requests,
        released,
        _dir: dir,
    }
}

fn command(socket: &PathBuf, hint: Option<&str>) -> Command {
    let mut cmd = Command::new(BIN);
    cmd.env("SSH_ASKPASS_OMARCHY_SOCKET", socket)
        .env_remove("SSH_ASKPASS_PROMPT")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(hint) = hint {
        cmd.env("SSH_ASKPASS_PROMPT", hint);
    }
    cmd
}

fn run(dialog: &FakeDialog, hint: Option<&str>, prompt: &str) -> Output {
    command(&dialog.socket, hint).arg(prompt).output().unwrap()
}

fn request(dialog: &FakeDialog) -> serde_json::Value {
    dialog
        .requests
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
}

fn encode(pw: &str) -> String {
    pw.bytes().fold(String::new(), |mut out, b| {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
        out
    })
}

fn wait_for(child: &mut Child, limit: Duration) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().unwrap() {
            return Some(status);
        }
        thread::sleep(Duration::from_millis(20));
    }
    None
}

fn kill_with(child: &Child, signal: &str) {
    let status = Command::new("kill")
        .args([&format!("-{signal}"), &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn passphrase_goes_to_stdout_exactly() {
    for pw in [
        "123456",
        "a%b\"c\\d\ne",
        "sénha çom acentos",
        "🔑🗝️",
        "%25%",
        " ",
    ] {
        let dialog = fake_dialog(Answer::Line(format!(
            r#"{{"result":"ok","password":"{}"}}"#,
            encode(pw)
        )));
        let out = run(&dialog, None, "Enter PIN for ED25519-SK key /k:");
        assert!(out.status.success(), "{pw:?}");
        assert_eq!(out.stdout, pw.as_bytes(), "{pw:?}");
        assert!(out.stderr.is_empty());
    }
}

#[test]
fn passphrase_request_carries_version_mode_and_prompt() {
    let dialog = fake_dialog(Answer::Line(r#"{"result":"cancel"}"#.into()));
    run(&dialog, None, "Enter passphrase for key '/k':");
    let r = request(&dialog);
    assert_eq!(r["v"], 2);
    assert_eq!(r["mode"], "passphrase");
    assert_eq!(r["prompt"], "Enter passphrase for key '/k':");
}

#[test]
fn missing_prompt_argument_gets_a_default() {
    let dialog = fake_dialog(Answer::Line(r#"{"result":"cancel"}"#.into()));
    let out = command(&dialog.socket, None).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(request(&dialog)["prompt"], "Enter password:");
}

#[test]
fn cancel_is_a_failure_with_no_output() {
    let dialog = fake_dialog(Answer::Line(r#"{"result":"cancel"}"#.into()));
    let out = run(&dialog, None, "p");
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
}

#[test]
fn confirm_allow_exits_zero_and_prints_nothing() {
    let dialog = fake_dialog(Answer::Line(r#"{"result":"ok"}"#.into()));
    let out = run(&dialog, Some("confirm"), "Allow use of key k?");
    assert!(out.status.success());
    assert!(out.stdout.is_empty(), "nothing on stdout in confirm mode");
    assert_eq!(request(&dialog)["mode"], "confirm");
}

#[test]
fn confirm_never_prints_a_password_even_if_sent() {
    let dialog = fake_dialog(Answer::Line(r#"{"result":"ok","password":"x"}"#.into()));
    let out = run(&dialog, Some("confirm"), "Allow?");
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
}

#[test]
fn confirm_denies_on_everything_but_ok() {
    for reply in [
        r#"{"result":"cancel"}"#,
        r#"{"result":"busy"}"#,
        r#"{"result":"error","message":"boom"}"#,
        r#"{"result":"OK"}"#,
        r#"{"result":"ok""#,
        "garbage",
        "",
    ] {
        let dialog = fake_dialog(Answer::Line(reply.into()));
        let out = run(&dialog, Some("confirm"), "Allow?");
        assert_eq!(out.status.code(), Some(1), "{reply:?}");
        assert!(out.stdout.is_empty(), "{reply:?}");
    }
}

#[test]
fn confirm_denies_when_the_dialog_goes_away() {
    for answer in [Answer::Close, Answer::Raw(Vec::new())] {
        let dialog = fake_dialog(answer);
        let out = run(&dialog, Some("confirm"), "Allow?");
        assert_eq!(out.status.code(), Some(1));
    }
}

#[test]
fn confirm_denies_when_there_is_no_dialog() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("nothing.sock");
    let out = command(&socket, Some("confirm"))
        .arg("Allow?")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("could not connect"));
}

#[test]
fn notify_without_a_dialog_exits_quietly() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("nothing.sock");
    let out = command(&socket, Some("none"))
        .arg("Confirm user presence")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(
        out.stderr.is_empty(),
        "a missing dialog must not spam ssh's stderr"
    );
}

#[test]
fn notify_stays_open_until_killed_and_then_closes_the_connection() {
    let dialog = fake_dialog(Answer::Hang);
    let mut child = command(&dialog.socket, Some("none"))
        .arg("Confirm user presence for key ED25519-SK SHA256:x")
        .spawn()
        .unwrap();
    let r = request(&dialog);
    assert_eq!(r["mode"], "notify");
    assert_eq!(
        wait_for(&mut child, Duration::from_millis(500)),
        None,
        "still waiting"
    );
    // What OpenSSH does once the key is touched.
    kill_with(&child, "TERM");
    let status = wait_for(&mut child, Duration::from_secs(5)).expect("exits on SIGTERM");
    assert_eq!(status.signal(), Some(15));
    dialog
        .released
        .recv_timeout(Duration::from_secs(5))
        .expect("the dialog sees the connection close");
}

#[test]
fn notify_exits_when_the_user_dismisses_it() {
    let dialog = fake_dialog(Answer::Line(r#"{"result":"cancel"}"#.into()));
    let out = run(&dialog, Some("none"), "Some notice");
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
}

#[test]
fn unknown_prompt_hints_ask_for_a_passphrase() {
    for hint in ["", "CONFIRM", "None", "sure"] {
        let dialog = fake_dialog(Answer::Line(r#"{"result":"ok","password":"pw"}"#.into()));
        let out = run(&dialog, Some(hint), "p");
        assert_eq!(out.stdout, b"pw", "{hint:?}");
        assert_eq!(request(&dialog)["mode"], "passphrase", "{hint:?}");
    }
}

#[test]
fn prompt_is_cleaned_before_it_reaches_the_dialog() {
    let dialog = fake_dialog(Answer::Line(r#"{"result":"cancel"}"#.into()));
    run(
        &dialog,
        Some("confirm"),
        "Allow use of key evil\u{202E}k?\r\n\u{1b}[2J\n\n\n\nKey fingerprint SHA256:x.",
    );
    assert_eq!(
        request(&dialog)["prompt"],
        "Allow use of key evilk?\n[2J\nKey fingerprint SHA256:x."
    );
}

#[test]
fn long_prompts_are_cut() {
    let dialog = fake_dialog(Answer::Line(r#"{"result":"cancel"}"#.into()));
    run(&dialog, None, &"x".repeat(100_000));
    assert_eq!(request(&dialog)["prompt"].as_str().unwrap().len(), 2000);
}

#[test]
fn a_prompt_that_is_not_utf8_does_not_crash_the_binary() {
    let dialog = fake_dialog(Answer::Line(r#"{"result":"cancel"}"#.into()));
    let out = command(&dialog.socket, None)
        .arg(std::ffi::OsStr::from_bytes(
            b"Enter passphrase for key (caf\xE9):",
        ))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "an error exit, not an abort");
    assert!(request(&dialog)["prompt"].as_str().unwrap().contains("caf"));
}

#[test]
fn oversized_responses_are_refused() {
    let dialog = fake_dialog(Answer::Raw(vec![b'x'; 200_000]));
    let out = run(&dialog, None, "p");
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("too long"));
}

#[test]
fn passphrase_is_refused_when_it_is_json_escaped() {
    let dialog = fake_dialog(Answer::Line(r#"{"result":"ok","password":"a\"b"}"#.into()));
    let out = run(&dialog, None, "p");
    assert_eq!(out.status.code(), Some(1));
    assert!(
        out.stdout.is_empty(),
        "nothing is printed for a refused reply"
    );
}

#[test]
fn passphrase_mode_never_succeeds_without_a_password() {
    let dialog = fake_dialog(Answer::Line(r#"{"result":"ok"}"#.into()));
    let out = run(&dialog, None, "p");
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
}

#[test]
fn dialog_messages_reach_stderr_cleaned() {
    let dialog = fake_dialog(Answer::Line(
        "{\"result\":\"error\",\"message\":\"bad\\u001b[31m request\"}".into(),
    ));
    let out = run(&dialog, None, "p");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("bad[31m request"), "{err:?}");
    assert!(!err.contains('\u{1b}'));
}

#[test]
fn the_process_is_not_dumpable() {
    // A non-dumpable process hides /proc/<pid>/mem from other processes
    // of the same user (root with CAP_SYS_PTRACE can still read it).
    if nix::unistd::geteuid().is_root() {
        return;
    }
    let dialog = fake_dialog(Answer::Hang);
    let mut child = command(&dialog.socket, Some("none"))
        .arg("x")
        .spawn()
        .unwrap();
    request(&dialog);
    let mem = std::fs::File::open(format!("/proc/{}/mem", child.id()));
    kill_with(&child, "KILL");
    let _ = child.wait();
    assert!(mem.is_err(), "the process's memory must not be readable");
}
