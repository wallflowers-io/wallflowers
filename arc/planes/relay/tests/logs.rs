//! What one connection writes to the log. Per-connection lines at INFO, and a client's
//! hang-up at WARN, filled wallflowers-01's disk on 8 Nov 2026 and stopped the store.

use std::io::Write;
use std::sync::{Arc, Mutex};

use tokio::net::TcpListener;
use tokio_tungstenite::client_async;

use relay::store::Store;
use relay::{Limits, Retention};

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// current_thread: the relay's tasks run on this thread, under this subscriber.
#[tokio::test]
async fn a_client_that_hangs_up_is_debug_not_a_warning() {
    let log = Captured::default();
    let w = log.clone();
    let _g = tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_ansi(false)
            .with_writer(move || w.clone())
            .finish(),
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let at = listener.local_addr().unwrap();
    let store = Store::open_in_memory().unwrap();
    tokio::spawn(relay::serve_with_policy(listener, Box::new(store), Retention::default(), Limits::default(), None));

    // Upgrade, then drop the socket with no close frame — what arc-node did every 2s.
    let tcp = tokio::net::TcpStream::connect(at).await.unwrap();
    let (ws, _) = client_async(format!("ws://{at}"), tcp).await.unwrap();
    drop(ws);

    let mut text = String::new();
    for _ in 0..100 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        text = String::from_utf8(log.0.lock().unwrap().clone()).unwrap();
        if text.contains("connection closed") {
            break;
        }
    }
    let lines: Vec<&str> = text.lines().filter(|l| l.contains("connection")).collect();
    assert!(lines.iter().any(|l| l.contains("connection closed")), "the close was not logged: {text}");
    assert!(lines.iter().all(|l| l.contains("DEBUG")), "a connection's life is DEBUG: {lines:#?}");
}
