# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions
follow [Semantic Versioning](https://semver.org/). Version 0.1.0 was never
tagged.

## [Unreleased]

This will be 0.2.0.

### Added
- **Security key touch prompt.** When OpenSSH runs the askpass with
  `SSH_ASKPASS_PROMPT=none` for "Confirm user presence for key …", the
  dialog shows a pulsing key and "Touch your security key" until OpenSSH
  kills the askpass. Other notifications get a message with **Dismiss**.
- **Key use confirmations.** `SSH_ASKPASS_PROMPT=confirm` (`ssh-add -c`)
  shows **Deny** / **Allow**. It used to show a passphrase field, and any
  Enter allowed the use.
- Protocol v2: requests carry a `mode` (`passphrase`, `confirm`, `notify`).
  A plugin and a binary from different versions refuse each other.
- Integration tests against a fake dialog (`tests/askpass.rs`), protocol
  tests against the real QML plugin (`tests/plugin.rs`, `just test-plugin`)
  and cargo-fuzz targets for the reply parser, the prompt cleaner and the
  passphrase decoding (`fuzz/`, `just fuzz`).
- GitHub security tooling: CI, CodeQL, cargo-deny, OpenSSF Scorecard,
  Dependabot and fuzzing workflows, a security policy and a contribution
  guide.

### Security
- The binary is non-dumpable (no core dumps, no same-uid ptrace) and locks
  its memory (`mlockall`).
- The passphrase is written to stdout through an unbuffered file. It used
  to pass through std's stdout buffer, which kept a copy of it unwiped.
- Confirmations fail closed: only a literal `ok` allows; errors, a busy
  shell, a closed connection or garbage deny. **Deny** is the default
  button and Enter never allows.
- A passphrase reply with JSON escapes is refused instead of unescaped
  into unwiped scratch memory, and `ok` without a password is an error.
- The prompt is cleaned before it is shown: control characters,
  bidirectional overrides, zero-width characters and line separators are
  removed, blank lines collapse, and it is cut to 6 lines and 2000 bytes.
  Messages from the dialog are cleaned before they reach stderr.
- A prompt that isn't valid UTF-8 no longer aborts the binary (the build
  uses `panic = "abort"`).
- The plugin refuses requests with a non-string or overlong prompt and
  lowers its request size limit to 16 KiB. It only hands the typed text over
  for an answered passphrase prompt.
- The crate forbids `unsafe` code (`#![forbid(unsafe_code)]`).
- `Cargo.lock` is now tracked, so builds and CI use the audited versions.
