//! THE OWNER'S DEVICE, for proving the hosting stack against real processes.
//!
//! This is not a fixture and not a simulator: it is an ordinary `Node`, on its own state
//! dir, doing exactly what a phone does — mint a group, mint its Host (kind 33, a part of
//! the group), add the Arc to the Host's roster from the Arc's own contact bundle, and
//! publish a face. Everything it writes is a Delta, through `mint` and `apply`, over the
//! relay, and the Arc folds it as any member would.
//!
//! It exists because the two supported platforms are the iOS app and the web app, and
//! neither can be scripted from a shell — so a local proof of the Arc's serving path
//! needs one device that can. It is an `examples/` target: dev-only, never in the
//! shipped library.
//!
//! Usage (see arc/hosting/face-smoke.sh, which drives it):
//!
//!   PACIFIC_STATE_DIR=/tmp/face-smoke/owner \
//!   cargo run -p pacific-core --example publish_a_face -- \
//!       --relay ws://127.0.0.1:8787 --bundle /tmp/face-smoke/arc-bundle.json \
//!       --arc <arc identity hex> --slug allotments --name "Mill Road Allotments"

use pacific_core::Node;

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn need(name: &str) -> String {
    arg(name).unwrap_or_else(|| {
        eprintln!("publish_a_face: {name} is required");
        std::process::exit(2);
    })
}

/// THE TWO THINGS A SHELL CANNOT DO FOR THE ADDRESS AUTHORITY, which owns a slug from
/// the claim at signup (ruled 22 Sep 2026). `face-smoke.sh` does the HTTP with curl;
/// what it needs from the device is the device's own key, and that never leaves here.
///
///   --sign   --audience <host> --nonce <nonce>   one challenge signature, hex
///   --wrap   --host <arc host>                   the account's wrap, hex, for registering
///
/// THE WRAP IS SEALED UNDER A FIXED PRF, and that is the one thing here that is not a
/// phone: a PRF comes out of a passkey ceremony and a shell has no authenticator. The
/// seal is real and the seed is this device's; only the unlock is a constant, and a
/// wrap made this way opens for anyone who reads this file. It is for the smoke's
/// throwaway accounts and nothing else.
fn answer_for_the_key(node: &Node) {
    if std::env::args().any(|a| a == "--sign") {
        let sig = node.sign_challenge(&need("--audience"), &need("--nonce")).expect("sign");
        println!("{sig}");
    } else {
        const SMOKE_PRF: [u8; 32] = *b"face-smoke: not a passkey's PRF.";
        let wrap = node.export_wrap(&SMOKE_PRF, &need("--host")).expect("wrap");
        println!("{}", hex::encode(wrap));
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    // An existing device answering for its key: no relay, no group, nothing written.
    if std::env::args().any(|a| a == "--sign" || a == "--wrap") {
        let node = Node::open().expect("open — run the publish first, so there is a device");
        answer_for_the_key(&node);
        return;
    }
    let relay = arg("--relay").unwrap_or_else(|| "ws://127.0.0.1:8787".into());
    let bundle_path = need("--bundle");
    let arc_hex = need("--arc").trim().trim_start_matches("ed25519:").to_string();
    let slug = need("--slug");
    let name = arg("--name").unwrap_or_else(|| "A Group".into());

    // The device's transport set, as a phone's would be — one file, one decision.
    let state = pacific_core::paths::state_dir();
    std::fs::create_dir_all(&state).expect("state dir");
    std::fs::write(state.join("relay_url"), &relay).expect("relay_url");

    let node = if state.join("id_ed25519").exists() {
        // An existing device keeps its Arc: its self record carries the old stamp, and a new
        // Arc would route that record to the stamp.
        Node::open().expect("open")
    } else {
        // A new device's Arc is its relay, before the identity mints the self record, or what
        // it mints is stamped with production's and the Arc that joins the Host dials that
        // stamp (NC-85).
        pacific_core::node::set_default_arc(&relay).expect("the device's Arc");
        Node::init_identity(&name).expect("init identity")
    };
    println!("owner   : {}", node.identity_key());

    // THE GROUP, then its HOST. Both are real objects with real MLS groups.
    let site = node.object_new("group", "").expect("mint group");
    let mut profile = pacific_core::coordinator::Args::new();
    profile.insert("displayName".into(), pacific_core::coordinator::ArgVal::Text(name.clone()));
    profile.insert("shape".into(), pacific_core::coordinator::ArgVal::Text("community".into()));
    node.apply(&site, pacific_core::group::OP_SET_PROFILE, profile).await.expect("profile");
    let host = node.host_new(&site, &name).await.expect("host_new");
    println!("group   : {site}\nhost    : {host}");

    // THE ARC JOINS THE HOST — its own bundle, exactly as GET /bundle served it.
    let bundle = std::fs::read_to_string(&bundle_path).expect("bundle file");
    let bundle = bundle.trim();
    // The route answers either the bare bundle or a JSON object carrying it.
    let bundle = match serde_json::from_str::<serde_json::Value>(bundle) {
        Ok(v) => v
            .get("bundle")
            .and_then(|b| b.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| bundle.to_string()),
        Err(_) => bundle.to_string(),
    };
    node.group_add_member(&host, &bundle).await.expect("add the Arc to the Host");
    node.sync_once().await.expect("sync");

    // THE FACE. A bundle the renderer can draw, one picture, one live item.
    let face = serde_json::json!({
        "v": 1,
        "profile": { "displayName": name, "shape": "community",
                     "card": { "note": "Thirty plots behind the station. Open day every Sunday.", "urls": [] } },
        "face": { "v": 1, "listed": true, "look": { "paper": "#F6F1E7", "ink": "#1A1A1A", "accent": "#ED140D" },
                  // A feed widget, so the Host's live items have somewhere to be drawn —
                  // without one an item folds fine and renders nowhere, which is the face
                  // document doing what it says, not a fault.
                  "widgets": { "root": { "children": [
                      { "type": "wf-feed", "source": "events", "title": "What's on", "size": "l" },
                      { "type": "wf-text", "text": "Plots come up in spring. Knock and ask." }
                  ] } },
                  "door": { "doorbell": "door", "label": "Ask about a plot", "questions": [] } },
        "location": { "shape": "area", "precision": 5, "label": "Romsey, Cambridge", "cells": [] }
    })
    .to_string();
    // A 1×1 png — the smoke is about the path, not the picture.
    const PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
    node.host_publish(&host, &slug, &arc_hex, &face, &[("mark".into(), "image/png".into(), PNG_B64.into())])
        .await
        .expect("publish");
    // A week from NOW, not a fixed date: the face draws only events still to come, and
    // this was `1790000000000` — 21 Sep 2026 — until the day after, when every run's page
    // quietly lost its event and the smoke went red with nothing wrong in the stack.
    let next_week = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
        + 7 * 86_400_000;
    node.host_put_item(
        &host,
        "event:dig-day",
        &serde_json::json!({ "title": "Dig day", "startMs": next_week, "venue": "The shed" }).to_string(),
        false,
    )
    .await
    .expect("put item");
    node.sync_once().await.expect("sync");

    println!("published: /{slug}  (the Arc serves it once its own sync has folded the Host)");
}
