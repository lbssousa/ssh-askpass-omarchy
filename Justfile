set shell := ["bash", "-uc"]

dev_socket := env("XDG_RUNTIME_DIR", "/run/user/1000") / "ssh-askpass-omarchy-dev.sock"

default:
    @just --list

# Debug build.
build:
    cargo build

# Unit + integration tests, formatting and clippy.
test:
    cargo test --locked
    cargo fmt --check
    cargo clippy --all-targets --locked -- -D warnings

# Protocol tests against the real QML plugin in a throwaway Quickshell
# instance (needs this Wayland session; dialogs flash on screen).
test-plugin:
    cargo test --locked --test plugin -- --ignored

# Fuzz one target (response, sanitize, percent_decode) for `secs` seconds;
# needs `rustup toolchain install nightly` and `cargo install cargo-fuzz`.
# New inputs go to fuzz/corpus/, crashes to fuzz/artifacts/.
fuzz target="response" secs="60":
    mkdir -p fuzz/corpus/{{target}}
    cargo +nightly fuzz run {{target}} fuzz/corpus/{{target}} fuzz/seeds/{{target}} -- -max_total_time={{secs}} -max_len=8192

# Run the dialog in a separate Quickshell instance (dev/), listening on its
# own socket, so QML edits only need this restarted — not omarchy-shell.
dev:
    SSH_ASKPASS_OMARCHY_SOCKET={{dev_socket}} quickshell -p dev

# Send a sample request (passphrase, confirm or touch) through `just dev`.
# touch stays up for 5s, then gets killed the way OpenSSH does it.
try mode="passphrase":
    cargo build -q
    export SSH_ASKPASS_OMARCHY_SOCKET={{dev_socket}}; \
    case "{{mode}}" in \
      passphrase) target/debug/ssh-askpass-omarchy "Enter PIN for ED25519-SK key ~/.ssh/id_ed25519_sk:"; echo " (exit $?)" ;; \
      confirm) SSH_ASKPASS_PROMPT=confirm target/debug/ssh-askpass-omarchy $'Allow use of key ~/.ssh/id_ed25519?\nKey fingerprint SHA256:example.'; echo "exit $?" ;; \
      touch) SSH_ASKPASS_PROMPT=none timeout -s TERM 5 target/debug/ssh-askpass-omarchy "Confirm user presence for key ED25519-SK SHA256:example"; echo "exit $?" ;; \
      *) echo "unknown mode: {{mode}}" >&2; exit 1 ;; \
    esac
