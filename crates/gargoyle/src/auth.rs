//! HTTP Basic auth (ARCHITECTURE §9.3, ADR 0005).
//!
//! The password is checked against an argon2id hash. argon2 is deliberately slow, so:
//! - a verified `Authorization` header is remembered (as its SHA-256, in memory only) and the
//!   10s polling of a logged-in browser never runs argon2 again;
//! - at most [`MAX_CONCURRENT_VERIFICATIONS`] verifications run at once, also when clients
//!   disconnect mid-verification, and at most [`MAX_QUEUED`] more wait; beyond that a request is
//!   refused without running argon2;
//! - failed verifications draw on a global budget ([`FAILURE_BURST`], refilled at
//!   [`FAILURES_PER_SEC`]); while it is empty every uncached header is refused without argon2, so
//!   online guessing is capped at that rate whatever the client does;
//! - a wrong password is answered only after [`FAILURE_DELAY`].

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use argon2::password_hash::PasswordVerifier;
use argon2::password_hash::phc::PasswordHash;
use argon2::{Algorithm, Argon2};
use axum::Json;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64ct::Encoding;
use gargoyle_core::api::ApiError;
use gargoyle_core::config::{AuthConfig, AuthMode};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, TryAcquireError};

/// The only route reachable without credentials: uptime probes need it, and it reveals nothing
/// beyond the version.
pub const EXEMPT_PATH: &str = "/api/v1/health";
pub const CHALLENGE: &str = "Basic realm=\"Gargoyle\", charset=\"UTF-8\"";
pub const FAILURE_DELAY: Duration = Duration::from_secs(1);
pub const MAX_CONCURRENT_VERIFICATIONS: usize = 2;
/// One waiter per running verification: enough for a real login to get through while another
/// one runs, without letting a flood queue up.
pub const MAX_QUEUED: usize = MAX_CONCURRENT_VERIFICATIONS;
/// A burst of typos is fine; a guessing attack gets 10 tries per second, about 860 000 per day,
/// which a long random password makes useless.
pub const FAILURE_BURST: f64 = 20.0;
pub const FAILURES_PER_SEC: f64 = 10.0;
/// One browser presents one header; a few more allow for a changed password or a second
/// device. When full the cache is cleared, which costs each client one argon2 run.
const CACHE_CAPACITY: usize = 16;

pub struct Authenticator {
    /// SHA-256 of the configured user name, so the comparison is constant-time and does not
    /// leak the name's length.
    username_digest: [u8; 32],
    hash: PasswordHash,
    verified: Mutex<HashSet<[u8; 32]>>,
    /// Owned permits move into the blocking task, so a verification holds its permit until
    /// argon2 has finished, even if the request that started it is gone.
    verifications: Arc<Semaphore>,
    /// Requests waiting for a permit.
    queued: AtomicUsize,
    failures: Mutex<TokenBucket>,
    failure_delay: Duration,
    /// argon2 runs so far; tests use it to prove polling hits the cache.
    argon2_runs: AtomicU64,
    #[cfg(test)]
    probe: test_support::Probe,
}

/// What a presented `Authorization` header gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Valid,
    /// Wrong or malformed credentials: answered after [`FAILURE_DELAY`].
    Invalid,
    /// Not checked, because too many verifications are running or waiting, or the failure
    /// budget is spent. Answered at once: delaying would only hold more connections open.
    Refused,
}

impl Authenticator {
    /// `phc` must be an argon2id PHC string; anything else is a startup error.
    pub fn new(username: &str, phc: &str) -> anyhow::Result<Self> {
        let hash = PasswordHash::new(phc.trim())
            .map_err(|e| anyhow::anyhow!("invalid password hash: {e}"))?;
        if hash.algorithm != Algorithm::Argon2id.ident() {
            bail!("password hash must be argon2id (generate one with `gargoyle hash-password`)");
        }
        Ok(Self {
            username_digest: Sha256::digest(username.as_bytes()).into(),
            hash,
            verified: Mutex::new(HashSet::new()),
            verifications: Arc::new(Semaphore::new(MAX_CONCURRENT_VERIFICATIONS)),
            queued: AtomicUsize::new(0),
            failures: Mutex::new(TokenBucket::new(FAILURE_BURST, FAILURES_PER_SEC)),
            failure_delay: FAILURE_DELAY,
            argon2_runs: AtomicU64::new(0),
            #[cfg(test)]
            probe: test_support::Probe::default(),
        })
    }

    /// The authenticator for `config`, or `None` when auth is off. Reads `password_hash_file`
    /// once; a changed file takes effect on restart.
    pub fn from_config(config: &AuthConfig) -> anyhow::Result<Option<Arc<Self>>> {
        if config.mode == AuthMode::None {
            return Ok(None);
        }
        let phc = match (&config.password_hash, &config.password_hash_file) {
            (Some(hash), _) => hash.clone(),
            (None, Some(path)) => read_hash_file(path)?,
            // Config validation rejects basic mode without a hash source.
            (None, None) => bail!("[auth] mode = \"basic\" requires a password hash"),
        };
        Self::new(&config.username, &phc)
            .context("[auth]")
            .map(|a| Some(Arc::new(a)))
    }

    #[cfg(test)]
    pub fn with_failure_delay(mut self, delay: Duration) -> Self {
        self.failure_delay = delay;
        self
    }

    #[cfg(test)]
    pub fn with_failure_budget(self, burst: f64, per_sec: f64) -> Self {
        *lock(&self.failures) = TokenBucket::new(burst, per_sec);
        self
    }

    #[cfg(test)]
    pub fn with_slow_verify(mut self, extra: Duration) -> Self {
        self.probe.slow = extra;
        self
    }

    #[cfg(test)]
    pub fn argon2_runs(&self) -> u64 {
        self.argon2_runs.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub fn max_in_flight(&self) -> usize {
        self.probe.max_in_flight.load(Ordering::SeqCst)
    }

    /// Checks `header` (the raw `Authorization` value).
    pub async fn check(self: &Arc<Self>, header: &[u8]) -> Verdict {
        let key: [u8; 32] = Sha256::digest(header).into();
        if lock(&self.verified).contains(&key) {
            return Verdict::Valid;
        }
        let Some((user, password)) = parse_basic(header) else {
            return Verdict::Invalid;
        };
        if !lock(&self.failures).has_token(Instant::now()) {
            tracing::debug!("auth failure budget spent; refusing without verification");
            return Verdict::Refused;
        }
        let Some(permit) = self.permit().await else {
            tracing::debug!("auth verifications busy; refusing without verification");
            return Verdict::Refused;
        };
        let this = Arc::clone(self);
        let ok = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            this.verify(&user, &password)
        })
        .await
        .unwrap_or(false);
        if ok {
            let mut cache = lock(&self.verified);
            if cache.len() >= CACHE_CAPACITY {
                cache.clear();
            }
            cache.insert(key);
            Verdict::Valid
        } else {
            lock(&self.failures).take(Instant::now());
            Verdict::Invalid
        }
    }

    /// A verification permit, waiting only if fewer than [`MAX_QUEUED`] already wait.
    async fn permit(&self) -> Option<OwnedSemaphorePermit> {
        match Arc::clone(&self.verifications).try_acquire_owned() {
            Ok(permit) => return Some(permit),
            Err(TryAcquireError::Closed) => return None,
            Err(TryAcquireError::NoPermits) => {}
        }
        let _place = QueuePlace::take(&self.queued)?;
        // The semaphore is never closed; if it were, refusing is the safe answer.
        Arc::clone(&self.verifications).acquire_owned().await.ok()
    }

    /// argon2 runs even for a wrong user name, so the response time does not reveal it.
    fn verify(&self, user: &str, password: &str) -> bool {
        #[cfg(test)]
        let _in_flight = self.probe.enter();
        let user_digest: [u8; 32] = Sha256::digest(user.as_bytes()).into();
        let user_ok: bool = user_digest.ct_eq(&self.username_digest).into();
        self.argon2_runs.fetch_add(1, Ordering::Relaxed);
        let password_ok = Argon2::default()
            .verify_password(password.as_bytes(), &self.hash)
            .is_ok();
        user_ok & password_ok
    }
}

/// A panic while holding one of these locks cannot leave its value inconsistent (every update
/// is a single call), so a poisoned lock is still usable.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A place in the verification queue, given back when dropped: also when the waiting request
/// is cancelled.
struct QueuePlace<'a>(&'a AtomicUsize);

impl<'a> QueuePlace<'a> {
    fn take(queued: &'a AtomicUsize) -> Option<Self> {
        let place = Self(queued);
        // Taken first and given back by the drop if the queue was already full.
        (queued.fetch_add(1, Ordering::SeqCst) < MAX_QUEUED).then_some(place)
    }
}

impl Drop for QueuePlace<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Failed verifications allowed: up to `burst` at once, refilled at `per_sec`.
struct TokenBucket {
    tokens: f64,
    burst: f64,
    per_sec: f64,
    last: Instant,
}

impl TokenBucket {
    fn new(burst: f64, per_sec: f64) -> Self {
        Self {
            tokens: burst,
            burst,
            per_sec,
            last: Instant::now(),
        }
    }

    fn refill(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.per_sec).min(self.burst);
        self.last = now;
    }

    fn has_token(&mut self, now: Instant) -> bool {
        self.refill(now);
        self.tokens >= 1.0
    }

    fn take(&mut self, now: Instant) {
        self.refill(now);
        self.tokens = (self.tokens - 1.0).max(0.0);
    }
}

/// `Basic <base64(user:password)>` → (user, password). The scheme is case-insensitive
/// (RFC 7617); the credentials must be UTF-8, as announced by `charset="UTF-8"`.
fn parse_basic(header: &[u8]) -> Option<(String, String)> {
    let header = std::str::from_utf8(header).ok()?;
    let (scheme, encoded) = header.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("basic") {
        return None;
    }
    let decoded = base64ct::Base64::decode_vec(encoded.trim()).ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    let (user, password) = decoded.split_once(':')?;
    Some((user.to_owned(), password.to_owned()))
}

fn read_hash_file(path: &Path) -> anyhow::Result<String> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read password_hash_file {}", path.display()))?;
    warn_if_world_readable(path);
    Ok(text.trim().to_owned())
}

fn warn_if_world_readable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path)
        && meta.permissions().mode() & 0o004 != 0
    {
        tracing::warn!(
            path = %path.display(),
            "password_hash_file is world-readable; restrict it (chmod 640, group gargoyle)"
        );
    }
}

/// Middleware: every route except [`EXEMPT_PATH`] needs valid credentials.
pub async fn require_auth(
    State(auth): State<Arc<Authenticator>>,
    req: Request,
    next: Next,
) -> Response {
    if req.uri().path() == EXEMPT_PATH {
        return next.run(req).await;
    }
    match req.headers().get(header::AUTHORIZATION) {
        // A browser's first request carries no credentials; it gets the challenge at once.
        None => challenge(),
        Some(value) => match auth.check(value.as_bytes()).await {
            Verdict::Valid => next.run(req).await,
            Verdict::Invalid => {
                tokio::time::sleep(auth.failure_delay).await;
                challenge()
            }
            Verdict::Refused => challenge(),
        },
    }
}

fn challenge() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static(CHALLENGE),
        )],
        Json(ApiError::new("unauthorized", "authentication required")),
    )
        .into_response()
}

#[cfg(test)]
pub mod test_support {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use argon2::password_hash::PasswordHasher;
    use argon2::{Algorithm, Argon2, Params, Version};

    /// Counts verifications running at once, and can make each one slower (as a costly hash
    /// would be) so that tests can overlap them.
    #[derive(Default)]
    pub struct Probe {
        pub slow: Duration,
        in_flight: AtomicUsize,
        pub max_in_flight: AtomicUsize,
    }

    impl Probe {
        pub fn enter(&self) -> InFlight<'_> {
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_in_flight.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(self.slow);
            InFlight(&self.in_flight)
        }
    }

    pub struct InFlight<'a>(&'a AtomicUsize);

    impl Drop for InFlight<'_> {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// A real argon2id PHC string with minimal cost, so tests stay fast.
    pub fn cheap_hash(password: &str) -> String {
        let params = Params::new(8, 1, 1, None).expect("valid minimal argon2 params");
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            .hash_password(password.as_bytes())
            .expect("hashing with a random salt works")
            .to_string()
    }

    pub fn basic(user: &str, password: &str) -> String {
        use base64ct::Encoding;
        format!(
            "Basic {}",
            base64ct::Base64::encode_string(format!("{user}:{password}").as_bytes())
        )
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{basic, cheap_hash};
    use super::*;

    fn auth() -> Arc<Authenticator> {
        Arc::new(
            Authenticator::new("admin", &cheap_hash("s3cret"))
                .unwrap()
                .with_failure_delay(Duration::ZERO),
        )
    }

    #[test]
    fn parses_basic_credentials() {
        assert_eq!(
            parse_basic(basic("admin", "pa:ss wörd").as_bytes()),
            Some(("admin".into(), "pa:ss wörd".into()))
        );
        assert_eq!(
            parse_basic(b"basic YWRtaW46eA=="),
            Some(("admin".into(), "x".into()))
        );
        for bad in [
            &b"Bearer YWRtaW46eA=="[..],
            b"Basic",
            b"Basic !!!",
            b"Basic YWRtaW4=", // no colon
            b"Basic /w==",     // not UTF-8
            b"\xff",
        ] {
            assert_eq!(parse_basic(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn rejects_non_argon2id_hashes() {
        assert!(Authenticator::new("admin", "hunter2").is_err());
        let argon2i = cheap_hash("x").replacen("$argon2id$", "$argon2i$", 1);
        assert!(Authenticator::new("admin", &argon2i).is_err());
        // Surrounding whitespace, as from a file with a trailing newline, is fine.
        assert!(Authenticator::new("admin", &format!("{}\n", cheap_hash("x"))).is_ok());
    }

    #[tokio::test]
    async fn verifies_user_and_password() {
        let auth = auth();
        let check = async |user: &str, pw: &str| auth.check(basic(user, pw).as_bytes()).await;
        assert_eq!(check("admin", "s3cret").await, Verdict::Valid);
        assert_eq!(check("admin", "wrong").await, Verdict::Invalid);
        assert_eq!(check("root", "s3cret").await, Verdict::Invalid);
        assert_eq!(check("admin", "").await, Verdict::Invalid);
        assert_eq!(check("", "s3cret").await, Verdict::Invalid);
        assert_eq!(auth.check(b"Basic !!!").await, Verdict::Invalid);
    }

    #[tokio::test]
    async fn verified_header_is_cached_but_failures_are_not() {
        let auth = auth();
        let good = basic("admin", "s3cret");
        for _ in 0..5 {
            assert_eq!(auth.check(good.as_bytes()).await, Verdict::Valid);
        }
        assert_eq!(auth.argon2_runs(), 1);
        let bad = basic("admin", "wrong");
        for _ in 0..3 {
            assert_eq!(auth.check(bad.as_bytes()).await, Verdict::Invalid);
        }
        assert_eq!(auth.argon2_runs(), 4);
    }

    #[tokio::test]
    async fn full_cache_is_cleared_not_grown() {
        let auth = auth();
        // Different spellings of the same credentials are different headers.
        let good = basic("admin", "s3cret");
        let encoded = &good["Basic ".len()..];
        for i in 0..=CACHE_CAPACITY {
            let header = format!("Basic{}{encoded}", " ".repeat(i + 1));
            assert_eq!(auth.check(header.as_bytes()).await, Verdict::Valid, "{i}");
        }
        assert!(lock(&auth.verified).len() <= CACHE_CAPACITY);
    }

    fn slow_auth(extra: Duration) -> Arc<Authenticator> {
        Arc::new(
            Authenticator::new("admin", &cheap_hash("s3cret"))
                .unwrap()
                .with_failure_delay(Duration::ZERO)
                .with_slow_verify(extra)
                .with_failure_budget(1000.0, 1000.0),
        )
    }

    /// M5.5 #1: a client that disconnects mid-verification (its request future is dropped)
    /// must not free the permit while argon2 still runs, or a flood of aborted requests runs
    /// unbounded argon2 in parallel.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn cancelled_requests_cannot_exceed_the_verification_cap() {
        let auth = slow_auth(Duration::from_millis(300));
        for i in 0..20 {
            let header = basic("admin", &format!("wrong{i}"));
            let check = auth.check(header.as_bytes());
            assert!(
                tokio::time::timeout(Duration::from_millis(5), check)
                    .await
                    .is_err()
            );
        }
        // Let the abandoned verifications finish.
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(auth.max_in_flight(), MAX_CONCURRENT_VERIFICATIONS);
        assert_eq!(auth.argon2_runs(), MAX_CONCURRENT_VERIFICATIONS as u64);
        assert_eq!(
            auth.queued.load(Ordering::SeqCst),
            0,
            "queue places given back"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn beyond_the_queue_requests_are_refused_without_argon2() {
        let auth = slow_auth(Duration::from_millis(300));
        let mut waiting = Vec::new();
        for i in 0..MAX_CONCURRENT_VERIFICATIONS + MAX_QUEUED {
            let auth = Arc::clone(&auth);
            waiting.push(tokio::spawn(async move {
                auth.check(basic("admin", &format!("wrong{i}")).as_bytes())
                    .await
            }));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            auth.check(basic("admin", "s3cret").as_bytes()).await,
            Verdict::Refused
        );
        for task in waiting {
            assert_eq!(task.await.unwrap(), Verdict::Invalid);
        }
        assert_eq!(
            auth.argon2_runs(),
            (MAX_CONCURRENT_VERIFICATIONS + MAX_QUEUED) as u64
        );
        assert!(auth.max_in_flight() <= MAX_CONCURRENT_VERIFICATIONS);
        // Once idle, the correct password works again.
        assert_eq!(
            auth.check(basic("admin", "s3cret").as_bytes()).await,
            Verdict::Valid
        );
    }

    #[tokio::test]
    async fn spent_failure_budget_refuses_without_argon2() {
        let auth = Arc::new(
            Authenticator::new("admin", &cheap_hash("s3cret"))
                .unwrap()
                .with_failure_delay(Duration::ZERO)
                .with_failure_budget(2.0, 0.0),
        );
        let good = basic("admin", "s3cret");
        assert_eq!(auth.check(good.as_bytes()).await, Verdict::Valid);
        for i in 0..2 {
            let bad = basic("admin", &format!("wrong{i}"));
            assert_eq!(auth.check(bad.as_bytes()).await, Verdict::Invalid);
        }
        // Spent: even the right password is not checked, but a cached header still works.
        let other_spelling = format!("Basic  {}", &good["Basic ".len()..]);
        assert_eq!(
            auth.check(other_spelling.as_bytes()).await,
            Verdict::Refused
        );
        assert_eq!(auth.check(good.as_bytes()).await, Verdict::Valid);
        assert_eq!(auth.argon2_runs(), 3);
    }

    #[test]
    fn token_bucket_refills_up_to_its_burst() {
        let start = Instant::now();
        let mut bucket = TokenBucket::new(2.0, 10.0);
        bucket.last = start;
        bucket.take(start);
        bucket.take(start);
        assert!(!bucket.has_token(start));
        assert!(!bucket.has_token(start + Duration::from_millis(50)));
        assert!(bucket.has_token(start + Duration::from_millis(100)));
        assert!(bucket.has_token(start + Duration::from_secs(60)));
        bucket.take(start + Duration::from_secs(60));
        bucket.take(start + Duration::from_secs(60));
        assert!(
            !bucket.has_token(start + Duration::from_secs(60)),
            "capped at the burst"
        );
    }

    #[test]
    fn hash_file_is_read_and_trimmed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("password");
        let hash = cheap_hash("s3cret");
        std::fs::write(&path, format!("  {hash}\n\n")).unwrap();
        let config = AuthConfig {
            mode: AuthMode::Basic,
            username: "admin".into(),
            password_hash: None,
            password_hash_file: Some(path),
        };
        assert!(Authenticator::from_config(&config).unwrap().is_some());

        let missing = AuthConfig {
            password_hash_file: Some(dir.path().join("nope")),
            ..config
        };
        let err = format!("{:#}", Authenticator::from_config(&missing).err().unwrap());
        assert!(err.contains("password_hash_file"), "{err}");
    }

    #[test]
    fn no_authenticator_when_auth_is_off() {
        assert!(
            Authenticator::from_config(&AuthConfig::default())
                .unwrap()
                .is_none()
        );
    }
}
