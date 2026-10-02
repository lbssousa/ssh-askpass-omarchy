# ssh-askpass-omarchy

An SSH Askpass dialog for the Omarchy shell, styled like the polkit agent.

## How it works

This consists of two parts:
1. An Omarchy (Quickshell) plugin (`lbssousa.ssh_askpass`) that listens on a UNIX socket for passphrase requests and displays the UI dialog.
2. A Rust binary (`ssh-askpass-omarchy`) that is executed by OpenSSH. It receives the prompt message as an argument, forwards it to the plugin's socket via JSON, waits for the user input, and outputs the resulting passphrase to stdout.

## Environment Variables

To use this with SSH, you need to set the `SSH_ASKPASS` and `SSH_ASKPASS_REQUIRE` variables.
```bash
export SSH_ASKPASS=/path/to/ssh-askpass-omarchy
export SSH_ASKPASS_REQUIRE=prefer
```
