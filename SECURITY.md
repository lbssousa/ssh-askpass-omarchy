# Security policy

## Supported versions

Only the latest release on `main` gets security fixes.

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub's
[private vulnerability reporting](https://github.com/lbssousa/ssh-askpass-omarchy/security/advisories/new),
not in a public issue. Include the version (in `Cargo.toml`), your Omarchy
version (`pacman -Q omarchy`), what you did, and what happened. You should
get an answer within a week.

## Threat model

ssh-askpass-omarchy asks for SSH key passphrases and FIDO PINs, confirms
`ssh-add -c` key uses, and shows security key touch requests, on behalf of
OpenSSH. The main assets are the passphrase or PIN and the decision to
allow a key to sign. A PIN that reaches a security key corrupted also costs
something: each wrong PIN uses up one of a limited number of tries.

**In scope:**

- Leaking the passphrase or PIN: to other users, to swap or core dumps, or
  through buffers that outlive the prompt.
- Corrupting the passphrase between the dialog and OpenSSH (encoding,
  framing, version mismatches).
- Granting by accident: a confirmation (`SSH_ASKPASS_PROMPT=confirm`) that
  succeeds without the user choosing **Allow**, including when the dialog
  fails, goes away or answers something unexpected.
- Text from outside (a key's comment, a destination constraint from a
  forwarded agent) that disguises what the dialog shows.
- Denial of service against the binary by malformed input from the dialog.

**Out of scope, by design:**

- **Processes running as the same user.** They can already set
  `SSH_ASKPASS`, run ssh themselves, or draw a look-alike layer-shell
  overlay. Same-uid is the trust boundary, as it is for ssh-agent itself.
  The socket only accepts peers with the user's uid.
- **The passphrase inside omarchy-shell.** It passes through a QML
  `TextInput` and the JavaScript heap, which can't be wiped. The fields are
  cleared right after answering. This is the same exposure as the shell's
  polkit dialog.
- **A hung shell.** OpenSSH doesn't time its askpass out, so a shell that
  accepts the connection but never answers keeps the askpass waiting. ssh
  (or the user, with Ctrl-C) can still cancel it.
- **Memory pressure on the shell.** A same-uid client can send an endless
  request line to the plugin's socket.
- **Security key touch prompts when OpenSSH doesn't call the askpass.** See
  the README: that is about whether a dialog shows up, not a leak.

## Hardening in place

- `mlockall`, non-dumpable process (no core dumps, no same-uid ptrace).
- Zeroized buffers for every copy of the passphrase in the binary, and an
  unbuffered write to stdout, so no stdio buffer keeps it.
- The passphrase travels percent-encoded, so it's never JSON-unescaped into
  scratch memory; an escaped reply is refused. Protocol versioning makes
  mismatched halves fail closed.
- Confirmations fail closed: only a literal `ok` from the dialog allows;
  an error, a busy shell, a closed connection or garbage all deny. **Deny**
  is the default button, and Enter never allows.
- The prompt is cleaned before it reaches the dialog: control characters,
  text-reordering and zero-width characters are removed, blank lines
  collapse, and it's cut to 6 lines and 2000 bytes. It's rendered as plain
  text, never rich text. What the dialog sends back is never echoed to
  stderr: failures are reported as fixed texts.
- The prompt argument isn't assumed to be UTF-8, so a key comment with
  stray bytes can't crash the binary.
- Bounded input: 64 KiB dialog replies and 16 KiB plugin requests.
- The response parser and the prompt cleaner are fuzzed with cargo-fuzz
  (`fuzz/`): on every pull request that touches the code, and for longer
  every week.
- The code forbids `unsafe`, and dependencies are checked by cargo-deny
  (advisories, licenses, sources), CodeQL and Dependabot.
