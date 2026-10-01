//! What a drain hands back (ICD-10). A subscription streams live for as long as its
//! socket lives, so a session that drained other tags earlier is handed their
//! messages too: those are not this drain's (NC-35). And a drain that reached no
//! transport is a failure, not an empty mailbox (NC-34).

mod common;

use common::Harness;
use pacific_core::router::{Router, Routes};
use pacific_wire::address::Address;

#[tokio::test]
async fn a_drain_hands_back_only_the_tags_it_asked_for() {
    let h = Harness::new(&["ada"]).await;
    let routes = Routes::parse(&h.relay);
    let (a, b) = (Address::from_seed(&[1u8; 32]), Address::from_seed(&[2u8; 32]));

    // The held session subscribes to A, which is empty.
    let mut held = Router::open(&routes).await.unwrap();
    assert!(held.drain(&[a.tag_hex()], |_| Ok(0)).await.unwrap().is_empty());

    // Someone else publishes to both. A's arrives on the held socket as a live push.
    let mut other = Router::open(&routes).await.unwrap();
    other.publish(&a, &pacific_wire::blob_b64(b"for a")).await.unwrap();
    other.publish(&b, &pacific_wire::blob_b64(b"for b")).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;

    let got = held.drain(&[b.tag_hex()], |_| Ok(0)).await.unwrap();
    let tags: Vec<&str> = got.iter().map(|m| m.tag.as_str()).collect();
    assert_eq!(tags, vec![b.tag_hex().as_str()], "only B's, and A's stays for A's own drain");

    // A's is still there for A, from A's cursor.
    let for_a = held.drain(&[a.tag_hex()], |_| Ok(0)).await.unwrap();
    assert!(for_a.iter().any(|m| m.tag == a.tag_hex()), "{for_a:?}");
}

#[tokio::test]
async fn a_drain_that_reaches_no_transport_fails() {
    // A relay that takes the socket and then hangs up.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            tokio::spawn(async move {
                if let Ok(ws) = tokio_tungstenite::accept_async(tcp).await {
                    drop(ws);
                }
            });
        }
    });
    let mut sess = Router::open(&Routes::parse(&format!("ws://{addr}"))).await.unwrap();
    let tag = Address::from_seed(&[3u8; 32]).tag_hex();
    let r = sess.drain(&[tag], |_| Ok(0)).await;
    assert!(r.is_err(), "a drain that reached no one reported {r:?}");
}
