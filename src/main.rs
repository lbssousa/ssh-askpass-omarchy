#![forbid(unsafe_code)]

use std::env;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process;

use nix::sys::socket::{getsockopt, sockopt::PeerCredentials};
use nix::unistd::getuid;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const PROTOCOL_VERSION: u32 = 1;

#[derive(Serialize)]
struct Request<'a> {
    v: u32,
    prompt: &'a str,
}

#[derive(Deserialize)]
struct RawResponse<'a> {
    result: &'a str,
    #[serde(borrow)]
    password: Option<&'a str>,
    message: Option<String>,
}

fn socket_path() -> PathBuf {
    if let Some(p) = env::var_os("SSH_ASKPASS_OMARCHY_SOCKET") {
        return PathBuf::from(p);
    }
    let runtime = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", getuid())));
    runtime.join("ssh-askpass-omarchy.sock")
}

fn connect() -> io::Result<UnixStream> {
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

fn percent_decode_into(bytes: &[u8], out: &mut Vec<u8>) {
    let mut iter = bytes.iter().copied();
    while let Some(b) = iter.next() {
        if b == b'%' {
            if let (Some(h1), Some(h2)) = (iter.next(), iter.next()) {
                if let (Some(n1), Some(n2)) = (
                    char::from(h1).to_digit(16),
                    char::from(h2).to_digit(16),
                ) {
                    out.push((n1 << 4 | n2) as u8);
                    continue;
                }
            }
        }
        out.push(b);
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let prompt = args.get(1).map(|s| s.as_str()).unwrap_or("Enter password:");

    let mut stream = match connect() {
        Ok(s) => s,
        Err(err) => {
            eprintln!("ssh-askpass-omarchy: could not connect to shell dialog: {}", err);
            process::exit(1);
        }
    };

    let req = Request {
        v: PROTOCOL_VERSION,
        prompt,
    };

    let mut line = match serde_json::to_vec(&req) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("ssh-askpass-omarchy: failed to serialize request: {}", err);
            process::exit(1);
        }
    };
    line.push(b'\n');

    if let Err(err) = stream.write_all(&line) {
        eprintln!("ssh-askpass-omarchy: failed to send request: {}", err);
        process::exit(1);
    }

    let mut buf = Zeroizing::new(vec![0u8; MAX_RESPONSE_BYTES]);
    let mut len = 0;
    let end = loop {
        if let Some(i) = buf[..len].iter().position(|&b| b == b'\n') {
            break i;
        }
        if len == buf.len() {
            eprintln!("ssh-askpass-omarchy: response too long");
            process::exit(1);
        }
        match stream.read(&mut buf[len..]) {
            Ok(0) if len == 0 => {
                eprintln!("ssh-askpass-omarchy: dialog went away");
                process::exit(1);
            }
            Ok(0) => break len,
            Ok(n) => len += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => {
                eprintln!("ssh-askpass-omarchy: read error: {}", e);
                process::exit(1);
            }
        }
    };

    let raw: RawResponse = match serde_json::from_slice(&buf[..end]) {
        Ok(r) => r,
        Err(err) => {
            eprintln!("ssh-askpass-omarchy: invalid json response: {}", err);
            process::exit(1);
        }
    };

    if raw.result != "ok" {
        if let Some(msg) = raw.message {
            eprintln!("ssh-askpass-omarchy: dialog error: {}", msg);
        }
        process::exit(1);
    }

    if let Some(encoded) = raw.password {
        let mut pw = Zeroizing::new(Vec::with_capacity(encoded.len()));
        percent_decode_into(encoded.as_bytes(), &mut pw);
        let mut stdout = io::stdout().lock();
        let _ = stdout.write_all(&pw);
        let _ = stdout.flush();
    }
}
