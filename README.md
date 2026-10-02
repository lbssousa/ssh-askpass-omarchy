# ssh-askpass-omarchy

[![ci](https://github.com/lbssousa/ssh-askpass-omarchy/actions/workflows/ci.yml/badge.svg)](https://github.com/lbssousa/ssh-askpass-omarchy/actions/workflows/ci.yml)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/lbssousa/ssh-askpass-omarchy/badge)](https://scorecard.dev/viewer/?uri=github.com/lbssousa/ssh-askpass-omarchy)

An SSH Askpass dialog for the Omarchy shell, styled like the polkit agent.
It asks for passphrases and FIDO PINs, confirms `ssh-add -c` key uses, and
shows security key touch requests, in the same overlay as the shell's
polkit agent.

## How it works

This consists of two parts:
1. An Omarchy (Quickshell) plugin (`lbssousa.ssh_askpass`) that listens on a UNIX socket for passphrase requests and displays the UI dialog.
2. A Rust binary (`ssh-askpass-omarchy`) that is executed by OpenSSH. It receives the prompt message as an argument, forwards it to the plugin's socket via JSON, waits for the user input, and outputs the resulting passphrase to stdout.

## Modes

OpenSSH tells the askpass what it wants through `SSH_ASKPASS_PROMPT`:

| `SSH_ASKPASS_PROMPT` | When | Dialog | Result |
|---|---|---|---|
| unset | passphrases, FIDO PINs (`Enter PIN for … key …`) | password field | passphrase on stdout |
| `confirm` | ssh-agent using a key added with `ssh-add -c` | **Deny** / **Allow** (Enter does not allow) | exit 0 allows, exit 1 denies |
| `none` | notifications, e.g. `Confirm user presence for key …` | **Touch your security key** card for user-presence prompts; a message with **Dismiss** otherwise | OpenSSH kills the askpass (SIGTERM) when it's done, which closes the dialog |

The touch dialog shows up only when OpenSSH runs the askpass, never for other
FIDO uses such as WebAuthn in a browser. If ssh-agent does the signing, it
needs `SSH_ASKPASS` in its environment too. Agents that don't run an askpass
(Proton Pass, for example) show nothing.

Stock OpenSSH also prints the touch request to the terminal, instead of
running the askpass, whenever ssh runs in one, even with
`SSH_ASKPASS_REQUIRE=prefer`. [omarchy-setup](https://github.com/lbssousa/omarchy-setup)
carries a small OpenSSH patch for that (`just openssh-askpass`).

### Protocol between the two halves (v2)

Request: `{"v":2,"mode":"passphrase"|"confirm"|"notify","prompt":"…"}`

Response: `{"result":"ok"|"cancel"|"busy"|"error","password"?,"message"?}`.
`password` (percent-encoded) is only sent in `passphrase` mode. The plugin
refuses other protocol versions and unknown modes.

### Security notes

See [SECURITY.md](SECURITY.md) for the threat model and how to report a
vulnerability. In short:

- **Memory:** the binary locks its memory (`mlockall`) and turns off core
  dumps and ptrace (non-dumpable). Every copy of the passphrase it makes
  lives in a zeroized buffer: the socket read buffer and the decoded
  passphrase. It writes to stdout unbuffered, so no stdio buffer keeps it.
- **Socket:** it lives in the user's 0700 runtime directory, and the
  binary checks that the listening peer runs as the same user.
- **Confirmations fail closed:** only a literal `ok` allows. **Deny** is
  the default button and Enter never allows.
- **Displayed text:** the prompt is stripped of control characters,
  text-reordering and zero-width characters, limited in size, and rendered
  as plain text, never rich text.
- **Inside omarchy-shell:** the passphrase goes through a QML `TextInput`
  and the JavaScript heap, which can't be wiped. The field is cleared as
  soon as the answer is sent. This is the same trade-off as the polkit
  dialog.

## Environment Variables

To use this with SSH, you need to set the `SSH_ASKPASS` and `SSH_ASKPASS_REQUIRE` variables.
```bash
export SSH_ASKPASS=/path/to/ssh-askpass-omarchy
export SSH_ASKPASS_REQUIRE=prefer
```

## Development

| Recipe | What it does |
|---|---|
| `just test` | Unit tests, integration tests against a fake dialog, `cargo fmt --check` and clippy |
| `just test-plugin` | Protocol tests against the real QML plugin in a throwaway Quickshell instance (needs the Wayland session; dialogs flash on screen) |
| `just fuzz [target] [secs]` | Fuzzes the reply parser, the prompt cleaner or the passphrase decoder with cargo-fuzz (needs nightly Rust and `cargo install cargo-fuzz`) |
| `just dev` | Runs the dialog in a separate Quickshell instance (`dev/`, which links the shell's `Commons`/`Ui`) on its own socket |
| `just try [mode]` | Sends a sample `passphrase`, `confirm` or `touch` request through the `just dev` instance |

## Feedback and contributing

- **Bugs and feature requests:** [GitHub issues](https://github.com/lbssousa/ssh-askpass-omarchy/issues).
- **Security vulnerabilities:** report them privately, as described in
  [SECURITY.md](SECURITY.md).
- **Contributions:** pull requests are welcome. See
  [CONTRIBUTING.md](CONTRIBUTING.md) for the rules: tests with every change,
  signed commits, and CI passing.
- **Release notes:** [CHANGELOG.md](CHANGELOG.md).

## License

[MIT](LICENSE).
