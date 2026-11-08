//! The connection loop (ARCHITECTURE §9.5). `axum::serve` sets no hyper timer, so a client
//! could hold a connection open forever by never finishing its headers (slowloris), and it
//! accepts any number of connections. This loop serves HTTP/1 with a header read timeout,
//! which also closes idle keep-alive connections, and a cap on open connections.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::service::TowerToHyperService;
use tokio::net::TcpListener;
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinSet;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// From the start of a request (or of waiting for the next one on a kept-alive
    /// connection) until its headers are complete.
    pub header_read_timeout: Duration,
    /// Beyond this, new connections wait in the kernel's accept queue.
    pub max_connections: usize,
    /// How long shutdown waits for in-flight responses before dropping their connections. A
    /// client that never reads its response would otherwise stall shutdown forever.
    pub drain_grace: Duration,
}

pub const LIMITS: Limits = Limits {
    header_read_timeout: Duration::from_secs(10),
    // A browser opens up to 6; far below the default open-file limit of 1024, so a flood of
    // connections cannot starve the collector of file descriptors.
    max_connections: 256,
    drain_grace: Duration::from_secs(5),
};

/// Request line plus headers; larger requests get 431. A browser's request is about 1 KB.
const MAX_HEADER_SIZE: usize = 64 * 1024;
/// After a failed accept (e.g. out of file descriptors), so the loop does not spin.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

/// Serves `app` until `shutdown` completes, then stops accepting, gives in-flight requests
/// `drain_grace` to finish, and returns once every connection is closed or dropped.
pub async fn serve(
    listener: TcpListener,
    app: Router,
    limits: Limits,
    shutdown: impl Future<Output = ()>,
) {
    let mut builder = http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(limits.header_read_timeout)
        .max_header_size(MAX_HEADER_SIZE);
    let slots = Arc::new(Semaphore::new(limits.max_connections));
    let (stop_tx, stop_rx) = watch::channel(());
    let mut connections = JoinSet::new();
    tokio::pin!(shutdown);

    loop {
        let slot = tokio::select! {
            () = &mut shutdown => break,
            // Never closed.
            slot = Arc::clone(&slots).acquire_owned() => match slot {
                Ok(slot) => slot,
                Err(_) => break,
            },
        };
        let stream = tokio::select! {
            () = &mut shutdown => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => stream,
                Err(e) => {
                    tracing::warn!(error = %e, "cannot accept connection");
                    tokio::time::sleep(ACCEPT_BACKOFF).await;
                    continue;
                }
            },
        };
        let conn =
            builder.serve_connection(TokioIo::new(stream), TowerToHyperService::new(app.clone()));
        let mut stop = stop_rx.clone();
        connections.spawn(async move {
            let _slot = slot;
            tokio::pin!(conn);
            tokio::select! {
                result = conn.as_mut() => return log_end(result),
                _ = stop.changed() => conn.as_mut().graceful_shutdown(),
            }
            log_end(conn.await);
        });
        // Reap finished connections so the set does not grow with every one ever served.
        while connections.try_join_next().is_some() {}
    }

    drop(listener);
    // Idle connections close at once; busy ones after their response.
    drop(stop_tx);
    let drained = tokio::time::timeout(limits.drain_grace, async {
        while connections.join_next().await.is_some() {}
    })
    .await;
    if drained.is_err() {
        tracing::warn!(
            open = connections.len(),
            "closing connections still busy after the shutdown grace period"
        );
        connections.shutdown().await;
    }
}

fn log_end(result: hyper::Result<()>) {
    if let Err(e) = result {
        // Timeouts, resets and malformed requests: routine, and the client's doing.
        tracing::debug!(error = %e, "connection ended with an error");
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::time::Instant;

    use axum::routing::get;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;
    use tokio::sync::oneshot;

    use super::*;

    const REQUEST: &[u8] = b"GET /ok HTTP/1.1\r\nHost: x\r\n\r\n";
    /// Far more than the socket buffers hold, so the response cannot be written unread.
    const BIG: usize = 32 << 20;

    struct Server {
        addr: SocketAddr,
        stop: Option<oneshot::Sender<()>>,
        done: tokio::task::JoinHandle<()>,
    }

    async fn start(limits: Limits) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new()
            .route("/ok", get(|| async { "ok" }))
            .route("/big", get(|| async { "x".repeat(BIG) }));
        let (stop, stopped) = oneshot::channel::<()>();
        let done = tokio::spawn(serve(listener, app, limits, async {
            stopped.await.ok();
        }));
        Server {
            addr,
            stop: Some(stop),
            done,
        }
    }

    /// Reads until the server closes or `limit` passes; returns what was read and whether
    /// the server closed.
    async fn read_until_close(stream: &mut TcpStream, limit: Duration) -> (String, bool) {
        let mut buf = Vec::new();
        let closed = tokio::time::timeout(limit, async {
            let mut chunk = [0u8; 1024];
            loop {
                match stream.read(&mut chunk).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                }
            }
        })
        .await
        .is_ok();
        (String::from_utf8_lossy(&buf).into_owned(), closed)
    }

    async fn read_response(stream: &mut TcpStream, limit: Duration) -> Option<String> {
        let mut buf = Vec::new();
        tokio::time::timeout(limit, async {
            let mut chunk = [0u8; 1024];
            while !String::from_utf8_lossy(&buf).ends_with("ok") {
                let n = stream.read(&mut chunk).await.unwrap();
                assert!(n > 0, "closed before a response");
                buf.extend_from_slice(&chunk[..n]);
            }
        })
        .await
        .ok()
        .map(|()| String::from_utf8_lossy(&buf).into_owned())
    }

    const SHORT: Limits = Limits {
        header_read_timeout: Duration::from_millis(200),
        max_connections: 8,
        drain_grace: Duration::from_millis(300),
    };

    #[tokio::test]
    async fn serves_requests() {
        let server = start(SHORT).await;
        let mut c = TcpStream::connect(server.addr).await.unwrap();
        c.write_all(REQUEST).await.unwrap();
        let resp = read_response(&mut c, Duration::from_secs(2)).await.unwrap();
        assert!(resp.starts_with("HTTP/1.1 200"), "{resp}");
    }

    #[tokio::test]
    async fn unfinished_headers_are_cut_off() {
        let server = start(SHORT).await;
        let mut c = TcpStream::connect(server.addr).await.unwrap();
        let start = Instant::now();
        c.write_all(b"GET /ok HTTP/1.1\r\nHost: x\r\n")
            .await
            .unwrap();
        let (_, closed) = read_until_close(&mut c, Duration::from_secs(3)).await;
        assert!(closed, "slowloris connection still open");
        assert!(
            start.elapsed() >= Duration::from_millis(150),
            "{:?}",
            start.elapsed()
        );
    }

    #[tokio::test]
    async fn idle_keep_alive_connections_are_closed() {
        let server = start(SHORT).await;
        let mut c = TcpStream::connect(server.addr).await.unwrap();
        c.write_all(REQUEST).await.unwrap();
        read_response(&mut c, Duration::from_secs(2)).await.unwrap();
        let (_, closed) = read_until_close(&mut c, Duration::from_secs(3)).await;
        assert!(closed, "idle connection still open");
    }

    #[tokio::test]
    async fn oversized_headers_are_refused() {
        let server = start(SHORT).await;
        let mut c = TcpStream::connect(server.addr).await.unwrap();
        let big = format!(
            "GET /ok HTTP/1.1\r\nHost: x\r\nX-Big: {}\r\n\r\n",
            "a".repeat(MAX_HEADER_SIZE)
        );
        // The server may close while this is still being written.
        let _ = c.write_all(big.as_bytes()).await;
        let (resp, closed) = read_until_close(&mut c, Duration::from_secs(3)).await;
        assert!(closed);
        assert!(
            resp.is_empty() || resp.starts_with("HTTP/1.1 431"),
            "{resp}"
        );
    }

    #[tokio::test]
    async fn connections_beyond_the_cap_wait_for_a_free_slot() {
        let server = start(Limits {
            max_connections: 1,
            header_read_timeout: Duration::from_secs(30),
            ..SHORT
        })
        .await;
        let mut first = TcpStream::connect(server.addr).await.unwrap();
        first.write_all(b"GET /ok HTTP/1.1\r\n").await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Connects (the kernel queues it) but is not served while the first holds the slot.
        let mut second = TcpStream::connect(server.addr).await.unwrap();
        second.write_all(REQUEST).await.unwrap();
        assert!(
            read_response(&mut second, Duration::from_millis(300))
                .await
                .is_none()
        );
        drop(first);
        let resp = read_response(&mut second, Duration::from_secs(2))
            .await
            .unwrap();
        assert!(resp.contains("200 OK"), "{resp}");
    }

    #[tokio::test]
    async fn shutdown_closes_idle_connections_and_returns() {
        let mut server = start(SHORT).await;
        let mut c = TcpStream::connect(server.addr).await.unwrap();
        c.write_all(REQUEST).await.unwrap();
        read_response(&mut c, Duration::from_secs(2)).await.unwrap();

        let _ = server.stop.take().unwrap().send(());
        tokio::time::timeout(Duration::from_secs(2), &mut server.done)
            .await
            .expect("serve returns promptly")
            .unwrap();
        let (_, closed) = read_until_close(&mut c, Duration::from_secs(1)).await;
        assert!(closed);
        assert!(
            TcpStream::connect(server.addr).await.is_err(),
            "listener closed"
        );
    }

    /// M5.5 R2: a client that never reads its response must not stall shutdown.
    #[tokio::test]
    async fn shutdown_drops_stalled_connections_after_the_grace_period() {
        let mut server = start(SHORT).await;
        let mut c = TcpStream::connect(server.addr).await.unwrap();
        c.write_all(b"GET /big HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        // Let the server fill the socket buffers and block writing.
        tokio::time::sleep(Duration::from_millis(200)).await;

        let start = Instant::now();
        let _ = server.stop.take().unwrap().send(());
        tokio::time::timeout(Duration::from_secs(2), &mut server.done)
            .await
            .expect("serve returns despite the unread response")
            .unwrap();
        let elapsed = start.elapsed();
        assert!(
            elapsed >= SHORT.drain_grace,
            "waited for the grace: {elapsed:?}"
        );
        drop(c);
    }
}
