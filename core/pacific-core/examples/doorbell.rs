//! doorbell — drive the doorbell-hosting verbs from a shell (smoke + live e2e).
//!
//! One invocation = one device op, exactly the harness discipline: the device is
//! `$PACIFIC_STATE_DIR`, the transport is whatever `init`/`routes` pinned into it.
//! This is the G4/G5 driver for docs/doorbell-hosting-icd.html — a founder phone
//! and a scanner phone in a terminal, against a real relay and a real hosting Arc.
//!
//!   doorbell init <name> <routes>     mint identity here + pin routes (comma list)
//!   doorbell routes <routes>          re-pin routes
//!   doorbell id                       print identity pk (hex)
//!   doorbell open-place <name>        mint + open + fit doorbell → prints the CARD
//!   doorbell card <place>             reprint a place's card
//!   doorbell anchor-group <place>     mint a group + anchor it at place → group id
//!   doorbell knock <card>             parse the card and knock (as this device)
//!   doorbell answer                   answer every open door we are behind
//!   doorbell sync [n]                 run n sync passes (default 1)
//!   doorbell roster <object>          print the MLS roster (identity pks, hex)
//!   doorbell state <place>            print folded name/access/doorbell-present

use pacific_core::place::Access;
use pacific_core::router::Routes;
use pacific_core::Node;

fn die(msg: &str) -> ! {
    eprintln!("doorbell: {msg}");
    std::process::exit(1)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("");
    let arg = |i: usize| -> &str {
        args.get(i)
            .map(String::as_str)
            .unwrap_or_else(|| die("missing argument — see the header for usage"))
    };

    match cmd {
        "init" => {
            // Its Arc is its relay, before the identity mints the self record, or what it
            // mints is stamped with production's (NC-85).
            let routes = Routes::parse(arg(2));
            if let Some(relay) = routes.urls().find(|u| u.starts_with("ws")) {
                pacific_core::node::set_default_arc(relay).unwrap_or_else(|e| die(&e.to_string()));
            }
            let mut n = Node::init_identity(arg(1)).unwrap_or_else(|e| die(&e.to_string()));
            n.set_routes(routes)
                .unwrap_or_else(|e| die(&e.to_string()));
            println!("{}", hex::encode(n.id.identity_pk()));
        }
        "routes" => {
            let mut n = Node::open().unwrap_or_else(|e| die(&e.to_string()));
            n.set_routes(Routes::parse(arg(1)))
                .unwrap_or_else(|e| die(&e.to_string()));
        }
        "id" => {
            let n = Node::open().unwrap_or_else(|e| die(&e.to_string()));
            println!("{}", hex::encode(n.id.identity_pk()));
        }
        "open-place" => {
            let n = Node::open().unwrap_or_else(|e| die(&e.to_string()));
            let id = n
                .place_mint(arg(1), "", None)
                .await
                .unwrap_or_else(|e| die(&e.to_string()));
            n.place_set_access(&id, Access::Public)
                .await
                .unwrap_or_else(|e| die(&e.to_string()));
            n.place_fit_doorbell(&id)
                .await
                .unwrap_or_else(|e| die(&e.to_string()));
            eprintln!("place {id}");
            println!(
                "{}",
                n.place_join_card(&id).unwrap_or_else(|e| die(&e.to_string()))
            );
        }
        "card" => {
            let n = Node::open().unwrap_or_else(|e| die(&e.to_string()));
            println!(
                "{}",
                n.place_join_card(arg(1))
                    .unwrap_or_else(|e| die(&e.to_string()))
            );
        }
        "anchor-group" => {
            let n = Node::open().unwrap_or_else(|e| die(&e.to_string()));
            let group = n
                .object_new("group", "")
                .unwrap_or_else(|e| die(&e.to_string()));
            n.group_anchor_at_place(&group, arg(1), now_ms())
                .await
                .unwrap_or_else(|e| die(&e.to_string()));
            println!("{group}");
        }
        "knock" => {
            let (place, bell) =
                Node::parse_place_join_card(arg(1)).unwrap_or_else(|e| die(&e.to_string()));
            let n = Node::open().unwrap_or_else(|e| die(&e.to_string()));
            n.place_knock(&place, &bell)
                .await
                .unwrap_or_else(|e| die(&e.to_string()));
            println!("knocked {place}");
        }
        "answer" => {
            let n = Node::open().unwrap_or_else(|e| die(&e.to_string()));
            let admitted = n
                .answer_all_doors()
                .await
                .unwrap_or_else(|e| die(&e.to_string()));
            println!("admitted-to {}", admitted.join(" "));
        }
        "sync" => {
            let rounds: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
            for _ in 0..rounds {
                let n = Node::open().unwrap_or_else(|e| die(&e.to_string()));
                let lines = n.sync_once().await.unwrap_or_else(|e| die(&e.to_string()));
                if !lines.is_empty() {
                    eprintln!("{}", lines.join(" · "));
                }
            }
        }
        "roster" => {
            let n = Node::open().unwrap_or_else(|e| die(&e.to_string()));
            for m in n
                .object_members(arg(1))
                .unwrap_or_else(|e| die(&e.to_string()))
            {
                println!("{}", hex::encode(m));
            }
        }
        "state" => {
            let n = Node::open().unwrap_or_else(|e| die(&e.to_string()));
            let st = n
                .place_state(arg(1))
                .unwrap_or_else(|e| die(&e.to_string()));
            println!(
                "name={:?} access={:?} doorbell={}",
                st.name,
                st.access,
                st.doorbell.is_some()
            );
        }
        _ => die("unknown command — see the header for usage"),
    }
}
