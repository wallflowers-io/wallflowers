//! THE AUTH SERVICE, OFF THE ACTOR (O-69). The head's compare-and-set and the seal's
//! confirmation are round trips to the auth service; on the actor they held every read and
//! write queued behind them (about 0.7 s a write at 200 ms RTT). Here they run on a thread of
//! their own, in order, and only the newest of each is sent.
//!
//! D-34 (c) is kept as it was; only its waiting moved. The actor seals to disk before it asks
//! for a confirmation. A confirmation that finds the counter moved by another writer still
//! ends the session, when the actor hears `Confirm::Moved`. A seal ahead of its counter still
//! opens (NC-76). The compare-and-set's base is the newest counter this worker has itself
//! confirmed, so a confirmation the actor has not heard of yet is never taken for another
//! writer's.

use std::sync::mpsc::{channel, Sender};
use std::time::{Duration, Instant};

/// What the head's store answered.
#[derive(Clone, Debug)]
pub enum Head {
    Stored,
    Behind(u64),
    /// Another head is at this position; theirs, when it could be read, to tell this chain
    /// sealed by another session from a real fork.
    Fork(Option<(Vec<u8>, u64)>),
    Failed(String),
}

/// What the seal's confirmation answered.
#[derive(Clone, Debug)]
pub enum Confirm {
    Stored,
    Moved(u64),
    Failed(String),
}

#[derive(Clone, Debug)]
pub enum Authed {
    Head { position: u64, got: Head },
    Confirm { counter: u64, got: Confirm },
}

enum Job {
    Head { position: u64, sealed: Vec<u8>, reply: Option<Sender<Authed>> },
    Confirm { held: u64, counter: u64, reply: Option<Sender<Authed>> },
}

/// Where the worker asks, and as whom.
pub struct Setup {
    pub auth: String,
    pub host: String,
    pub door: String,
}

pub struct Worker {
    tx: Sender<Job>,
    moved: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Worker {
    /// A thread of its own, signing as the person whose seed it is given. `done` hears each
    /// answer not asked for by `now`.
    pub fn start(setup: Setup, seed: zeroize::Zeroizing<[u8; 32]>, done: Box<dyn Fn(Authed) + Send>) -> Worker {
        let (tx, rx) = channel::<Job>();
        let moved = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let found = moved.clone();
        std::thread::Builder::new()
            .name("auth".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("the auth runtime");
                let http = crate::auth::client();
                let id = pacific_core::identity::Identity::in_memory(*seed);
                drop(seed);
                let pk = hex::encode(id.identity_pk());
                let signed = |nonce: &str| hex::encode(id.sign(&pacific_core::identity::auth_payload(&setup.host, &id.identity_key(), nonce)));
                let mut confirmed: Option<u64> = None;
                while let Ok(first) = rx.recv() {
                    // THE NEWEST OF EACH: what queued behind a round trip is one ask, not many.
                    let mut jobs = vec![first];
                    jobs.extend(rx.try_iter());
                    let (mut head, mut conf): (Option<(u64, Vec<u8>)>, Option<(u64, u64)>) = (None, None);
                    let (mut head_to, mut conf_to) = (Vec::new(), Vec::new());
                    for j in jobs {
                        match j {
                            Job::Head { position, sealed, reply } => {
                                if head.as_ref().is_none_or(|(p, _)| position >= *p) {
                                    head = Some((position, sealed));
                                }
                                head_to.extend(reply);
                            }
                            Job::Confirm { held, counter, reply } => {
                                let (h, c) = conf.unwrap_or((held, counter));
                                conf = Some((h.max(held), c.max(counter)));
                                conf_to.extend(reply);
                            }
                        }
                    }
                    if let Some((position, sealed)) = head {
                        let got = crate::auth::again(|| {
                            rt.block_on(async {
                                let nonce = crate::auth::challenge(&http, &setup.auth, &setup.host).await?;
                                crate::auth::put_head(&http, &setup.auth, &pk, &nonce, &signed(&nonce), position, sealed.clone()).await
                            })
                        });
                        let got = match got {
                            Ok(crate::auth::Stored::Stored) => Head::Stored,
                            Ok(crate::auth::Stored::Behind { held }) => Head::Behind(held),
                            Ok(crate::auth::Stored::Fork) => Head::Fork(
                                rt.block_on(crate::auth::head(&http, &setup.auth, &pk)).ok().flatten().map(|h| (h.blob, h.declared_position)),
                            ),
                            Err(e) => Head::Failed(e),
                        };
                        answer(Authed::Head { position, got }, &head_to, &done);
                    }
                    if let Some((held, counter)) = conf {
                        let base = confirmed.map_or(held, |c| c.max(held));
                        let got = if base >= counter {
                            Confirm::Stored
                        } else {
                            let got = crate::auth::again(|| {
                                rt.block_on(async {
                                    let nonce = crate::auth::challenge(&http, &setup.auth, &setup.host).await?;
                                    crate::auth::put_seal(&http, &setup.auth, &pk, &setup.door, &nonce, &signed(&nonce), base, counter).await
                                })
                            });
                            match got {
                                Ok(crate::auth::Counted::Stored) => {
                                    confirmed = Some(counter);
                                    Confirm::Stored
                                }
                                Ok(crate::auth::Counted::Moved { held }) => {
                                    found.store(true, std::sync::atomic::Ordering::SeqCst);
                                    Confirm::Moved(held)
                                }
                                Err(e) => Confirm::Failed(e),
                            }
                        };
                        answer(Authed::Confirm { counter, got }, &conf_to, &done);
                    }
                }
            })
            .expect("the auth thread");
        Worker { tx, moved }
    }

    /// Another writer's counter was found: this process holds nothing to keep.
    pub fn moved(&self) -> bool {
        self.moved.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Store this head, after whatever is already asked.
    pub fn head(&self, position: u64, sealed: Vec<u8>) {
        let _ = self.tx.send(Job::Head { position, sealed, reply: None });
    }

    /// Confirm this seal's counter, after whatever is already asked.
    pub fn confirm(&self, held: u64, counter: u64) {
        let _ = self.tx.send(Job::Confirm { held, counter, reply: None });
    }

    /// Both, after whatever is already asked, answered here within `within`: the session's
    /// end waits on them.
    pub fn now(&self, head: Option<(u64, Vec<u8>)>, confirm: Option<(u64, u64)>, within: Duration) -> Vec<Authed> {
        let (tx, rx) = channel();
        let mut asked = 0;
        if let Some((position, sealed)) = head {
            asked += usize::from(self.tx.send(Job::Head { position, sealed, reply: Some(tx.clone()) }).is_ok());
        }
        if let Some((held, counter)) = confirm {
            asked += usize::from(self.tx.send(Job::Confirm { held, counter, reply: Some(tx.clone()) }).is_ok());
        }
        drop(tx);
        let until = Instant::now() + within;
        let mut out = Vec::new();
        while out.len() < asked {
            match rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
                Ok(a) => out.push(a),
                Err(_) => break,
            }
        }
        out
    }
}

/// To whoever asked by `now`, and always to the actor: its keeper and its head follow.
fn answer(a: Authed, to: &[Sender<Authed>], done: &dyn Fn(Authed)) {
    for t in to {
        let _ = t.send(a.clone());
    }
    done(a);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};

    const HOST: &str = "auth.test";

    /// An auth service on loopback that answers the challenge and stores whatever it is given,
    /// over keep-alive, and closes a connection left idle for `idle` (uvicorn does at 5 s).
    struct StandIn {
        url: String,
        /// The head's stores it was sent.
        heads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl StandIn {
        /// Its head's stores answered `head`, its other asks 200.
        fn start(idle: Duration, head: u16) -> StandIn {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", l.local_addr().unwrap());
            let heads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let seen = heads.clone();
            std::thread::spawn(move || {
                for s in l.incoming().flatten() {
                    let seen = seen.clone();
                    std::thread::spawn(move || serve(s, idle, head, &seen));
                }
            });
            StandIn { url, heads }
        }
    }

    fn serve(s: std::net::TcpStream, idle: Duration, head: u16, heads: &std::sync::atomic::AtomicUsize) {
        s.set_read_timeout(Some(idle)).unwrap();
        let mut w = s.try_clone().unwrap();
        let mut r = BufReader::new(s);
        let mut nonce = 0;
        loop {
            let mut line = String::new();
            // Idle past `idle`, or closed: the connection ends here, as the service ends it.
            if r.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            let (method, path) = {
                let mut p = line.split_whitespace();
                (p.next().unwrap_or("").to_string(), p.next().unwrap_or("").to_string())
            };
            let mut len = 0usize;
            loop {
                let mut h = String::new();
                if r.read_line(&mut h).unwrap_or(0) == 0 {
                    return;
                }
                if h == "\r\n" {
                    break;
                }
                if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0; len];
            if r.read_exact(&mut body).is_err() {
                return;
            }
            let (status, out) = if method == "GET" && path == "/auth/challenge" {
                nonce += 1;
                (200, format!(r#"{{"nonce":"n{nonce}","audience":"{HOST}"}}"#))
            } else if method == "PUT" && path.ends_with("/head") {
                heads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                (head, "{}".to_string())
            } else {
                (200, "{}".to_string())
            };
            let reply = format!("HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{out}", out.len());
            if w.write_all(reply.as_bytes()).is_err() {
                return;
            }
        }
    }

    fn worker(auth: &StandIn) -> Worker {
        let setup = Setup { auth: auth.url.clone(), host: HOST.into(), door: "door.test".into() };
        Worker::start(setup, zeroize::Zeroizing::new([7u8; 32]), Box::new(|_| {}))
    }

    /// The service closes an idle connection at a quarter of the pool's keep.
    fn closing() -> StandIn {
        StandIn::start(crate::auth::IDLE / 4, 200)
    }

    fn head(w: &Worker, position: u64) -> Vec<Authed> {
        w.now(Some((position, vec![position as u8])), None, Duration::from_secs(10))
    }

    /// NC-130: the service closed the connection the worker's pool keeps, while nothing polled the
    /// worker's runtime to see it close. Past the pool's keep, the next store takes a new
    /// connection and is stored: a write the session's end stores is not lost to a dead socket.
    #[test]
    fn a_head_after_the_auth_closed_an_idle_connection_is_stored() {
        let auth = closing();
        let w = worker(&auth);
        let first = head(&w, 1);
        assert!(matches!(first[..], [Authed::Head { got: Head::Stored, .. }]), "{first:?}");
        std::thread::sleep(crate::auth::IDLE + Duration::from_secs(1));
        let second = head(&w, 2);
        assert!(matches!(second[..], [Authed::Head { got: Head::Stored, .. }]), "{second:?}");
    }

    /// The same for the seal's confirmation.
    #[test]
    fn a_confirmation_after_the_auth_closed_an_idle_connection_is_stored() {
        let auth = closing();
        let w = worker(&auth);
        let first = w.now(None, Some((0, 1)), Duration::from_secs(10));
        assert!(matches!(first[..], [Authed::Confirm { got: Confirm::Stored, .. }]), "{first:?}");
        std::thread::sleep(crate::auth::IDLE + Duration::from_secs(1));
        let second = w.now(None, Some((1, 2)), Duration::from_secs(10));
        assert!(matches!(second[..], [Authed::Confirm { got: Confirm::Stored, .. }]), "{second:?}");
    }

    /// The pool alone, with no retry: past IDLE the client's next ask takes a new connection.
    #[test]
    fn the_auth_client_drops_an_idle_connection_before_the_service_can() {
        let auth = closing();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let http = crate::auth::client();
        rt.block_on(crate::auth::challenge(&http, &auth.url, HOST)).expect("the first");
        std::thread::sleep(crate::auth::IDLE + Duration::from_secs(1));
        rt.block_on(crate::auth::challenge(&http, &auth.url, HOST)).expect("on a new connection");
    }

    /// An answer is never asked again: a head the service refused is sent once.
    #[test]
    fn a_head_the_service_refused_is_not_sent_again() {
        let auth = StandIn::start(crate::auth::IDLE / 4, 500);
        let w = worker(&auth);
        let got = head(&w, 1);
        assert!(matches!(got[..], [Authed::Head { got: Head::Failed(_), .. }]), "{got:?}");
        assert_eq!(auth.heads.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    /// Within the pool's keep, a connection the service already closed is taken, and the send
    /// fails with no answer: asked once more, on a new connection, it is stored.
    #[test]
    fn a_head_sent_on_a_connection_the_service_closed_is_stored_on_the_retry() {
        let auth = closing();
        let w = worker(&auth);
        let first = head(&w, 1);
        assert!(matches!(first[..], [Authed::Head { got: Head::Stored, .. }]), "{first:?}");
        std::thread::sleep(crate::auth::IDLE / 2);
        let second = head(&w, 2);
        assert!(matches!(second[..], [Authed::Head { got: Head::Stored, .. }]), "{second:?}");
    }
}
