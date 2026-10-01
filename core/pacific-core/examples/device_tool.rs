//! A minimal "device" for the local membership e2e: it does exactly what the app does, in the
//! two moments the HTTP flow needs — produce a contact bundle to sign up with, and mint an Arc
//! membership credential (ICD §10) to authenticate with. Crypto only; the HTTP is done by curl.
//!
//!   device_tool bundle <arc_ws_url>    -> prints {identity_key} and the contact bundle (one line)
//!   device_tool credential <arc_space> -> prints the membership credential for that Arc
//!
//! Identity is persisted at PACIFIC_STATE_DIR, so the two invocations share one identity. A new
//! device takes its Arc from `bundle`, before its identity (O-73).

use pacific_core::Node;

fn open_or_init(arc: Option<&String>) -> Node {
    match Node::open() {
        Ok(n) => n,
        Err(_) => {
            let arc = arc.expect("usage: bundle <arc_ws_url> — a new device needs its Arc");
            pacific_core::node::set_default_arc(arc).expect("the device's Arc");
            Node::init_identity("Device").expect("init identity")
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("bundle") => {
            let n = open_or_init(args.get(2));
            // Two lines: the space id (for reference) then the bundle curl will POST to /v1/signup.
            println!("SPACE {}", n.identity_key());
            println!("BUNDLE {}", n.build_contact_bundle().expect("bundle"));
        }
        Some("credential") => {
            let arc_space = args.get(2).expect("usage: credential <arc_space_id>");
            let n = open_or_init(None);
            let identity_key = n.identity_key();
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            // The EXACT payload verify_member checks — same as pacific-ffi::arc_member_credential.
            let payload = format!("pacific-arc-member:v1\n{arc_space}\n{identity_key}\n{ts}");
            let sig = n.id.sign(payload.as_bytes());
            println!("{identity_key}:{ts}:{}", hex::encode(sig));
        }
        other => {
            eprintln!("unknown command {other:?}; use `bundle` or `credential <arc_space>`");
            std::process::exit(2);
        }
    }
}
