#![forbid(unsafe_code)]

//! Client half of ssh-askpass-omarchy: maps the OpenSSH askpass call onto a
//! request for the shell plugin and turns the plugin's answer into what
//! OpenSSH expects (stdout and exit status).

pub mod request;
pub mod secret;
pub mod shell;
pub mod text;
