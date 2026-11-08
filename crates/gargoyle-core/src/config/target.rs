//! The one place where a check's configured target is made safe to show (ARCHITECTURE §6
//! "CheckSummary", §9.4).
//!
//! A URL may carry credentials in its userinfo (`user:pass@`) or in its query string
//! (`?token=...`), and a fragment is never needed to identify a target. Everything that shows a
//! check's target (API responses, log lines, `Debug` output) goes through [`display_url`] or
//! [`display_address`]; the raw URL is only ever handed to the HTTP client.
//!
//! Two layers, so that a password can leak only if both are wrong:
//! 1. [`url_problem`] rejects at config load every URL whose userinfo is ambiguous: an `@` in the
//!    path, query or fragment, more than one `@` in the authority, or a host that is not a plain
//!    `host[:port]` (as when a password contains an unencoded `/`, `?` or `#`, which makes the URL
//!    parser end the authority inside the password). Users percent-encode such characters.
//! 2. [`strip_url`] does not trust that: if the URL contains an `@` anywhere, it shows only
//!    `scheme://host[:port]` taken from after the last `@`, and nothing else of the URL.

/// Shown in place of a target that cannot be shown safely.
pub const INVALID_URL: &str = "(invalid URL)";

const AMBIGUOUS_AT: &str = "must contain at most one '@', ending the user name and password; percent-encode any other \
     '@' as %40";
const BAD_HOST: &str = "must name a host, optionally with a port; percent-encode '/', '?', '#' \
     and '@' in a user name or password";

/// The URL as the API and logs show it: [`INVALID_URL`] unless it passes [`url_problem`], else
/// [`strip_url`].
pub fn display_url(url: &str) -> String {
    if url_problem(url).is_some() {
        INVALID_URL.to_owned()
    } else {
        strip_url(url)
    }
}

/// A TCP address as the API and logs show it. Validation rejects an `@` in an address; should one
/// get through anyway, only what follows the last `@` is shown.
pub fn display_address(address: &str) -> String {
    address
        .rsplit_once('@')
        .map_or(address, |(_, rest)| rest)
        .to_owned()
}

/// `scheme://[userinfo@]host[:port][/path][?query][#fragment]` → `scheme://host[:port][/path]`,
/// and `scheme://host[:port]` only if the URL contains an `@` anywhere.
fn strip_url(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return INVALID_URL.to_owned();
    };
    if scheme.is_empty() || !scheme.chars().all(|c| c.is_ascii_alphanumeric()) {
        return INVALID_URL.to_owned();
    }
    let (host, path) = if let Some((_, after_at)) = rest.rsplit_once('@') {
        // Something credential-like is in there: the host after the last `@` and nothing else.
        (until_authority_end(after_at), "")
    } else {
        let host = until_authority_end(rest);
        let after = &rest[host.len()..];
        (
            host,
            &after[..after.find(['?', '#']).unwrap_or(after.len())],
        )
    };
    if is_host_port(host) {
        format!("{scheme}://{host}{path}")
    } else {
        INVALID_URL.to_owned()
    }
}

/// The authority ends at the first `/`, `?`, `#` or `\`, as for the WHATWG URL parser with
/// `http` and `https`.
fn until_authority_end(s: &str) -> &str {
    &s[..s.find(['/', '?', '#', '\\']).unwrap_or(s.len())]
}

/// `host`, `host:port` or `[ipv6]` / `[ipv6]:port`. Host names are letters, digits, `-`, `.` and
/// `_`; a port is digits that fit in a `u16`.
fn is_host_port(s: &str) -> bool {
    let (host_ok, port) = match s.strip_prefix('[') {
        Some(v6) => {
            let Some((addr, rest)) = v6.split_once(']') else {
                return false;
            };
            let port = match rest {
                "" => None,
                _ => match rest.strip_prefix(':') {
                    Some(port) => Some(port),
                    None => return false,
                },
            };
            let addr_ok = !addr.is_empty()
                && addr
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() || c == ':' || c == '.');
            (addr_ok, port)
        }
        None => {
            let (host, port) = match s.split_once(':') {
                Some((host, port)) => (host, Some(port)),
                None => (s, None),
            };
            let host_ok = !host.is_empty()
                && host
                    .chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '-' | '.' | '_'));
            (host_ok, port)
        }
    };
    host_ok
        && port.is_none_or(|p| {
            !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) && p.parse::<u16>().is_ok()
        })
}

/// Why a check URL is rejected at config load, if it is.
pub(super) fn url_problem(url: &str) -> Option<&'static str> {
    let Some(rest) = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
    else {
        return Some("must start with http:// or https://");
    };
    // The URL parser silently drops tabs and newlines and treats `\` like `/`.
    if url
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
    {
        return Some("must not contain whitespace, control characters or backslashes");
    }
    let authority = until_authority_end(rest);
    if rest[authority.len()..].contains('@') || authority.matches('@').count() > 1 {
        return Some(AMBIGUOUS_AT);
    }
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    (!is_host_port(host)).then_some(BAD_HOST)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `(url, rejected with, shown as)`. Shown is what [`display_url`] returns, which is
    /// [`INVALID_URL`] for every rejected URL.
    const ROWS: &[(&str, Option<&str>, &str)] = &[
        (
            "http://127.0.0.1:4533/ping",
            None,
            "http://127.0.0.1:4533/ping",
        ),
        ("http://host", None, "http://host"),
        ("https://host:8443/a/b#frag", None, "https://host:8443/a/b"),
        ("http://host?token=abc", None, "http://host"),
        ("http://host#x?y", None, "http://host"),
        ("http://host/x/?a=1&b=2", None, "http://host/x/"),
        ("http://[::1]:80/x?y", None, "http://[::1]:80/x"),
        ("http://my_host.lan/", None, "http://my_host.lan/"),
        // Userinfo: only the host is shown, without the path.
        ("http://user:pass@host/x?token=abc", None, "http://host"),
        ("http://u@host/", None, "http://host"),
        ("http://a:b@[::1]/", None, "http://[::1]"),
        (
            "http://user:p%40ss%2Fx@host:8080/p",
            None,
            "http://host:8080",
        ),
        // Reviewer cases: unencoded `/`, `?` or `#` in a password.
        ("http://admin:pa/ss@host/p", Some(AMBIGUOUS_AT), INVALID_URL),
        (
            "http://admin:12345/xyz@host/p",
            Some(AMBIGUOUS_AT),
            INVALID_URL,
        ),
        ("http://admin:pa?ss@host/p", Some(AMBIGUOUS_AT), INVALID_URL),
        ("http://admin:pa#ss@host/p", Some(AMBIGUOUS_AT), INVALID_URL),
        // More `@`s.
        ("http://us@er:p@ss@host/p", Some(AMBIGUOUS_AT), INVALID_URL),
        ("http://host/a@b", Some(AMBIGUOUS_AT), INVALID_URL),
        ("http://host/?u=a@b", Some(AMBIGUOUS_AT), INVALID_URL),
        ("http://host/#a@b", Some(AMBIGUOUS_AT), INVALID_URL),
        // Hosts that are not host[:port].
        ("http://admin:pa", Some(BAD_HOST), INVALID_URL),
        ("http://host:", Some(BAD_HOST), INVALID_URL),
        ("http://host:99999/", Some(BAD_HOST), INVALID_URL),
        ("http://", Some(BAD_HOST), INVALID_URL),
        ("http://user:pass@/x", Some(BAD_HOST), INVALID_URL),
        ("http://[::1/", Some(BAD_HOST), INVALID_URL),
        ("http://ho%73t/", Some(BAD_HOST), INVALID_URL),
        // Other.
        (
            "ftp://host",
            Some("must start with http:// or https://"),
            INVALID_URL,
        ),
        (
            "http://user:pa\nss@host/",
            Some("must not contain whitespace, control characters or backslashes"),
            INVALID_URL,
        ),
        (
            "http://evil\\@host/",
            Some("must not contain whitespace, control characters or backslashes"),
            INVALID_URL,
        ),
        (
            "http://host/a b",
            Some("must not contain whitespace, control characters or backslashes"),
            INVALID_URL,
        ),
    ];

    #[test]
    fn validation_and_display() {
        for (url, problem, shown) in ROWS {
            assert_eq!(url_problem(url), *problem, "problem for {url:?}");
            assert_eq!(display_url(url), *shown, "display of {url:?}");
        }
    }

    /// Without validation, the stripping alone still never shows anything before the last `@`,
    /// nor any path, query or fragment once an `@` is present.
    #[test]
    fn stripping_alone_never_shows_credentials() {
        let cases = [
            ("http://admin:pa/ss@host/p", "http://host"),
            ("http://admin:12345/xyz@host/p", "http://host"),
            ("http://admin:pa?ss@host/p", "http://host"),
            ("http://admin:pa#ss@host/p", "http://host"),
            ("http://us@er:p@ss@host:1/p?q#f", "http://host:1"),
            ("http://host/a@b?token=x", "http://b"),
            ("http://user:secret@host\\x", "http://host"),
            ("http://admin:pa/ss@/x", INVALID_URL),
            ("http://admin:pa/ss@ho st", INVALID_URL),
            ("no scheme", INVALID_URL),
            ("://host", INVALID_URL),
        ];
        for (url, expected) in cases {
            assert_eq!(strip_url(url), expected, "{url:?}");
            for secret in ["pa", "ss", "12345", "xyz", "secret", "token", "er"] {
                if !expected.contains(secret) {
                    assert!(!strip_url(url).contains(secret), "{secret} from {url:?}");
                }
            }
        }
    }

    #[test]
    fn addresses_never_show_userinfo() {
        assert_eq!(display_address("127.0.0.1:25565"), "127.0.0.1:25565");
        assert_eq!(display_address("user:pass@host:25"), "host:25");
        assert_eq!(display_address("a@b@host:25"), "host:25");
    }
}
