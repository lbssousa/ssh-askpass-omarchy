//! Exercises the real QML plugin in a Quickshell instance built from dev/.
//! Needs a Wayland session with Omarchy installed (dev/ links the shell's
//! Commons and Ui), and briefly flashes dialogs on screen, so it is
//! ignored by default: run it with `just test-plugin`.

use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

struct Shell(Child);

impl Drop for Shell {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_shell(socket: &Path) -> Shell {
    let dev = Path::new(env!("CARGO_MANIFEST_DIR")).join("dev");
    let child = Command::new("quickshell")
        .arg("-p")
        .arg(dev)
        .env("SSH_ASKPASS_OMARCHY_SOCKET", socket)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("quickshell must be installed");
    let shell = Shell(child);
    let deadline = Instant::now() + Duration::from_secs(15);
    while UnixStream::connect(socket).is_err() {
        assert!(
            Instant::now() < deadline,
            "the plugin never started listening"
        );
        thread::sleep(Duration::from_millis(100));
    }
    shell
}

fn send(socket: &Path, request: &[u8]) -> UnixStream {
    let mut stream = UnixStream::connect(socket).unwrap();
    stream.write_all(request).unwrap();
    stream
}

/// Reads until the plugin closes the connection, which it does after
/// every answer.
fn answer(mut stream: UnixStream, wait: Duration) -> serde_json::Value {
    stream.set_read_timeout(Some(wait)).unwrap();
    let mut out = String::new();
    stream.read_to_string(&mut out).unwrap();
    assert!(out.ends_with('\n'), "unterminated answer {out:?}");
    serde_json::from_str(&out).unwrap()
}

fn ask(socket: &Path, request: &str) -> serde_json::Value {
    answer(send(socket, request.as_bytes()), Duration::from_secs(10))
}

/// True if the plugin keeps the connection open without saying anything,
/// i.e. it accepted the request and is waiting for the user.
fn dialog_is_open(stream: &mut UnixStream) -> bool {
    stream
        .set_read_timeout(Some(Duration::from_millis(1000)))
        .unwrap();
    let mut buf = [0u8; 64];
    matches!(
        stream.read(&mut buf),
        Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
    )
}

#[test]
#[ignore = "needs a Wayland session and Omarchy's shell; run with `just test-plugin`"]
fn plugin_protocol() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("askpass.sock");
    let _shell = start_shell(&socket);

    // Malformed requests are refused without opening a dialog.
    let r = ask(&socket, "not json\n");
    assert_eq!(
        (r["result"].as_str(), r["message"].as_str()),
        (Some("error"), Some("invalid JSON"))
    );
    let r = ask(&socket, "{\"v\":1,\"prompt\":\"p\"}\n");
    assert_eq!(
        r["message"], "unsupported request",
        "old protocol versions are refused"
    );
    for bad in [
        r#"{"v":2,"prompt":"p"}"#,
        r#"{"v":2,"mode":"frob"}"#,
        r#"{"v":2,"mode":"Confirm"}"#,
        r#"{"v":2,"mode":["confirm"]}"#,
        r#"{"v":2,"mode":"confirm","prompt":5}"#,
        r#"{"v":2,"mode":"confirm","prompt":{"a":1}}"#,
        "null",
        "[]",
    ] {
        let r = ask(&socket, &format!("{bad}\n"));
        assert_eq!(r["result"], "error", "{bad}");
        assert_eq!(r["message"], "unsupported request", "{bad}");
    }
    let huge = format!(
        "{{\"v\":2,\"mode\":\"notify\",\"prompt\":\"{}\"}}\n",
        "x".repeat(70_000)
    );
    assert_eq!(ask(&socket, &huge)["message"], "request too large");
    let long = format!(
        "{{\"v\":2,\"mode\":\"confirm\",\"prompt\":\"{}\"}}\n",
        "x".repeat(3_000)
    );
    assert_eq!(ask(&socket, &long)["message"], "prompt too long");

    // One dialog at a time, and a refused request never reads as a grant.
    let mut first = send(
        &socket,
        b"{\"v\":2,\"mode\":\"confirm\",\"prompt\":\"Allow?\"}\n",
    );
    assert!(dialog_is_open(&mut first), "a valid request opens a dialog");
    let r = ask(
        &socket,
        "{\"v\":2,\"mode\":\"confirm\",\"prompt\":\"Allow?\"}\n",
    );
    assert_eq!(r["result"], "busy");
    assert!(r.get("password").is_none());

    // A client that goes away closes its dialog and frees the plugin.
    drop(first);
    thread::sleep(Duration::from_millis(500));
    let mut again = send(
        &socket,
        b"{\"v\":2,\"mode\":\"notify\",\"prompt\":\"Confirm user presence for key k\"}\n",
    );
    assert!(dialog_is_open(&mut again), "the dialog was freed");
    drop(again);
    thread::sleep(Duration::from_millis(500));

    // A request with no prompt is fine in every mode.
    let mut bare = send(&socket, b"{\"v\":2,\"mode\":\"passphrase\"}\n");
    assert!(dialog_is_open(&mut bare));
}
