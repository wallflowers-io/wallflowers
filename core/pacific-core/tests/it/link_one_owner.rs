//! #144: one owner of the device's network. Every relay operation goes through the
//! process's link, so a run of ordinary operations, each on a fresh Node as the FFI
//! makes them, dials a bounded number of connections, not one or more per operation.

use crate::common::Harness;

/// The request pool's size per relay (`link::POOL`), plus the listen socket.
const BOUND: u64 = 3;

#[tokio::test]
async fn twenty_operations_on_fresh_nodes_dial_a_bounded_number_of_times() {
    let h = Harness::new(&["ada", "bo"]).await;
    let (ada, bo) = (0, 1);
    let forum = h.form_forum(ada, &[bo]).await;
    h.settle().await;

    let before = pacific_core::transport::dials();
    for i in 0..5 {
        h.obj_post(ada, &forum, &format!("post {i}")).await;
        h.sync(bo).await;
        h.rename_object(ada, &forum, &format!("forum {i}")).await;
        h.sync(ada).await;
    }
    let dialled = pacific_core::transport::dials() - before;
    assert!(dialled <= BOUND, "20 operations dialled the relay {dialled} times; the link holds at most {BOUND}");
}

use pacific_core::link::{station, Answer, Ask, LinkState};
use pacific_wire::address::Address;
use pacific_wire::{blob_b64, Frame};

/// A relay, and a proxy in front of it that cuts the FIRST connection right after it has
/// forwarded a frame `cut` picks: the relay has acted on it, the answer never arrives.
async fn relay_with_a_cut(cut: fn(&Frame) -> bool) -> (String, String) {
    use futures_util::{SinkExt, StreamExt};
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let relay = format!("ws://{}", l.local_addr().unwrap());
    tokio::spawn(async move { relay::serve(l).await });
    let p = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = format!("ws://{}", p.local_addr().unwrap());
    let upstream = relay.clone();
    tokio::spawn(async move {
        let mut first = true;
        while let Ok((tcp, _)) = p.accept().await {
            let (upstream, cutting) = (upstream.clone(), std::mem::replace(&mut first, false));
            tokio::spawn(async move {
                let Ok(client) = tokio_tungstenite::accept_async(tcp).await else { return };
                let Ok((server, _)) = tokio_tungstenite::connect_async(upstream.as_str()).await else { return };
                let ((mut c_tx, mut c_rx), (mut s_tx, mut s_rx)) = (client.split(), server.split());
                let up = async {
                    while let Some(Ok(m)) = c_rx.next().await {
                        let hit = cutting && m.to_text().ok().and_then(|t| Frame::from_json(t).ok()).is_some_and(|f| cut(&f));
                        // Forwarded, then cut at once: the relay acts on it (its bytes are
                        // ahead of the close on the wire), and no answer comes back.
                        if s_tx.send(m).await.is_err() || hit {
                            return;
                        }
                    }
                };
                let down = async {
                    while let Some(Ok(m)) = s_rx.next().await {
                        if c_tx.send(m).await.is_err() {
                            return;
                        }
                    }
                };
                tokio::select! { _ = up => {}, _ = down => {} }
            });
        }
    });
    (relay, proxy)
}

/// How many blobs the relay holds at `tag`, read on a session of its own.
async fn stored(relay: &str, tag: &str) -> usize {
    let mut s = pacific_core::transport::RelaySession::connect(relay).await.unwrap();
    s.drain(vec![tag.to_string()], 0).await.unwrap().len()
}

/// A publish whose answer is lost after the relay stored it is resent once, and the relay
/// holds it once.
#[tokio::test]
async fn a_publish_cut_after_the_relay_stored_it_is_resent_and_stored_once() {
    let (relay, proxy) = relay_with_a_cut(|f| matches!(f, Frame::Pub { commit: false, .. })).await;
    let at = Address::from_seed(&[0x51; 32]);
    let got = station(&proxy).ask(Ask::Publish(at.pub_frame(blob_b64(b"once"), false))).await;
    assert!(matches!(got, Ok(Answer::Seq(s)) if s > 0), "the resend was acked as the publish it was: {got:?}");
    assert_eq!(stored(&relay, &at.tag_hex()).await, 1);
}

/// A commit whose answer is lost after it won its slot still reads as won: the resend is
/// the same bytes, acked as a duplicate before the slot is looked at.
#[tokio::test]
async fn a_commit_cut_after_it_won_still_reads_as_won() {
    let (relay, proxy) = relay_with_a_cut(|f| matches!(f, Frame::Pub { commit: true, .. })).await;
    let at = Address::from_seed(&[0x52; 32]);
    let got = station(&proxy).ask(Ask::Commit(at.pub_frame(blob_b64(b"commit"), true))).await;
    assert!(matches!(got, Ok(Answer::Slot(Some(_)))), "a commit that won is not reported lost: {got:?}");
    assert_eq!(stored(&relay, &at.tag_hex()).await, 1);
}

/// A drain cut part-way is drained again, whole.
#[tokio::test]
async fn a_drain_cut_mid_way_is_drained_again_whole() {
    let (relay, proxy) = relay_with_a_cut(|f| matches!(f, Frame::Sub { tags, .. } if !tags.is_empty())).await;
    let at = Address::from_seed(&[0x53; 32]);
    let mut s = pacific_core::transport::RelaySession::connect(&relay).await.unwrap();
    for i in 0..3u8 {
        s.publish(&at, blob_b64(&[i])).await.unwrap();
    }
    let got = station(&proxy).ask(Ask::Drain(vec![at.tag_hex()], 0)).await;
    assert!(matches!(&got, Ok(Answer::Msgs(m)) if m.len() == 3), "{got:?}");
}

/// A relay that accepts and then says nothing is not waited on forever: the exchange fails,
/// as a transport error, and the station says it is offline.
#[tokio::test]
async fn a_relay_that_goes_silent_fails_the_exchange_and_reads_as_offline() {
    std::env::set_var("LINK_IDLE_MS", "300");
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", l.local_addr().unwrap());
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((tcp, _)) = l.accept().await {
            if let Ok(ws) = tokio_tungstenite::accept_async(tcp).await {
                held.push(ws); // accepted, and never answered
            }
        }
    });
    let st = station(&url);
    let started = std::time::Instant::now();
    let got = st.ask(Ask::Drain(vec!["00".repeat(32)], 0)).await;
    assert!(matches!(got, Err(pacific_core::CoreError::Transport(_))), "{got:?}");
    assert!(started.elapsed() < std::time::Duration::from_secs(5), "waited {:?}", started.elapsed());
    assert!(matches!(st.state(), LinkState::Offline { .. }), "{:?}", st.state());
}

/// Nothing listening: opening a mailbox fails at once, as dialling did before the link.
#[tokio::test]
async fn an_unreachable_relay_fails_to_open_and_reads_as_offline() {
    let st = station("ws://127.0.0.1:9");
    assert!(st.ready().await.is_err());
    assert!(matches!(st.state(), LinkState::Offline { .. } | LinkState::Connecting { .. }), "{:?}", st.state());
}
