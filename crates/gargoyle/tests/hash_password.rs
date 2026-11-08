//! `gargoyle hash-password` with the password on stdin (M5.2).

#![allow(clippy::unwrap_used)] // test code

use std::io::Write;
use std::process::{Command, Output, Stdio};

use argon2::Argon2;
use argon2::password_hash::PasswordVerifier;
use argon2::password_hash::phc::PasswordHash;

fn hash_password(stdin: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_gargoyle"))
        .arg("hash-password")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn prints_only_a_verifiable_phc_string() {
    let out = hash_password(b"s3cret pass\n");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    let phc = stdout.strip_suffix('\n').unwrap();
    assert!(!phc.contains('\n'), "{stdout:?}");
    assert!(phc.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{phc}");
    let parsed = PasswordHash::new(phc).unwrap();
    assert!(
        Argon2::default()
            .verify_password(b"s3cret pass", &parsed)
            .is_ok()
    );
    // The password itself is never echoed.
    assert!(!String::from_utf8_lossy(&out.stderr).contains("s3cret"));
}

#[test]
fn empty_input_fails() {
    for input in [&b""[..], b"\n"] {
        let out = hash_password(input);
        assert!(!out.status.success(), "{input:?}");
        assert!(out.stdout.is_empty());
    }
}
