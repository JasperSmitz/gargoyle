//! `gargoyle hash-password` (M5.2): prints the argon2id PHC string for `[auth] password_hash`.

use std::io::{BufRead, IsTerminal, Write};

use anyhow::{Context, bail};
use argon2::password_hash::PasswordHasher;
use argon2::{Algorithm, Argon2, Params, Version};
use rustix::termios::{self, LocalModes, OptionalActions, Termios};

/// OWASP's argon2id recommendation: 19 MiB, 2 iterations, 1 lane. Verification reads the
/// parameters from the hash, so raising these later does not break existing hashes.
const M_COST_KIB: u32 = 19_456;
const T_COST: u32 = 2;
const P_COST: u32 = 1;

pub fn run() -> anyhow::Result<()> {
    let stdin = std::io::stdin();
    let password = if stdin.is_terminal() {
        prompt_with_confirmation()?
    } else {
        first_line(stdin.lock())?
    };
    println!("{}", hash(&password)?);
    Ok(())
}

pub fn hash(password: &str) -> anyhow::Result<String> {
    if password.is_empty() {
        bail!("the password must not be empty");
    }
    let params = Params::new(M_COST_KIB, T_COST, P_COST, None)
        .map_err(|e| anyhow::anyhow!("argon2 parameters: {e}"))?;
    let phc = Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password(password.as_bytes())
        .map_err(|e| anyhow::anyhow!("cannot hash the password: {e}"))?;
    Ok(phc.to_string())
}

/// For `echo "$PW" | gargoyle hash-password`: the first line, without its line ending. Spaces
/// are kept; they may be part of the password.
fn first_line(mut input: impl BufRead) -> anyhow::Result<String> {
    let mut line = String::new();
    input
        .read_line(&mut line)
        .context("cannot read the password from stdin")?;
    let line = line.strip_suffix('\n').unwrap_or(&line);
    Ok(line.strip_suffix('\r').unwrap_or(line).to_owned())
}

fn prompt_with_confirmation() -> anyhow::Result<String> {
    let password = prompt("Password: ")?;
    if password.is_empty() {
        bail!("the password must not be empty");
    }
    if prompt("Confirm password: ")? != password {
        bail!("the passwords do not match");
    }
    Ok(password)
}

/// Reads one line from the terminal on stdin with echo off; the prompt goes to stderr so
/// stdout holds only the hash.
fn prompt(label: &str) -> anyhow::Result<String> {
    let stdin = std::io::stdin();
    eprint!("{label}");
    std::io::stderr().flush().ok();
    let original = termios::tcgetattr(&stdin).context("cannot read terminal settings")?;
    let mut silent = original.clone();
    silent.local_modes.remove(LocalModes::ECHO);
    // Still echo the Enter, so the next prompt starts on a new line.
    silent.local_modes.insert(LocalModes::ECHONL);
    termios::tcsetattr(&stdin, OptionalActions::Now, &silent)
        .context("cannot turn off terminal echo")?;
    let _restore = RestoreEcho(original);
    first_line(stdin.lock())
}

/// Puts the terminal back even when reading fails.
struct RestoreEcho(Termios);

impl Drop for RestoreEcho {
    fn drop(&mut self) {
        // Nothing useful can be done if this fails; the shell resets the terminal anyway.
        let _ = termios::tcsetattr(std::io::stdin(), OptionalActions::Now, &self.0);
    }
}

#[cfg(test)]
mod tests {
    use argon2::password_hash::PasswordVerifier;
    use argon2::password_hash::phc::PasswordHash;

    use super::*;

    #[test]
    fn first_line_strips_only_the_line_ending() {
        let cases: [(&[u8], &str); 5] = [
            (b"hunter2\n", "hunter2"),
            (b"hunter2\r\n", "hunter2"),
            (b"hunter2", "hunter2"),
            (b" pass word \nsecond line\n", " pass word "),
            (b"", ""),
        ];
        for (input, want) in cases {
            assert_eq!(first_line(input).unwrap(), want, "{input:?}");
        }
    }

    #[test]
    fn hashes_with_owasp_parameters_and_verifies() {
        let phc = hash("correct horse").unwrap();
        assert!(phc.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"), "{phc}");
        let parsed = PasswordHash::new(&phc).unwrap();
        assert!(
            Argon2::default()
                .verify_password(b"correct horse", &parsed)
                .is_ok()
        );
        assert!(
            Argon2::default()
                .verify_password(b"wrong", &parsed)
                .is_err()
        );
        // A fresh salt every time.
        assert_ne!(phc, hash("correct horse").unwrap());
    }

    #[test]
    fn empty_password_is_refused() {
        assert!(hash("").is_err());
    }
}
