//! Decoding the passphrase: decoding never grows its input, and anything
//! JavaScript's encodeURIComponent produces comes back byte for byte.

#![no_main]

use libfuzzer_sys::fuzz_target;
use ssh_askpass_omarchy::text::percent_decode_into;

fn encode_uri_component(text: &str) -> String {
    text.bytes().fold(String::new(), |mut out, b| {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
        out
    })
}

fuzz_target!(|data: &[u8]| {
    let mut out = Vec::new();
    percent_decode_into(data, &mut out);
    assert!(out.len() <= data.len());

    if let Ok(text) = std::str::from_utf8(data) {
        let mut round = Vec::new();
        percent_decode_into(encode_uri_component(text).as_bytes(), &mut round);
        assert_eq!(round, text.as_bytes());
    }
});
