//! Does the relay take a blob this big? Go-live's step 1 (K-44): one publish of `kib`
//! KiB (default 300) of random bytes to a fresh random address, answered with the
//! relay's ack or its refusal. The relay measures the base64 frame, and its default cap
//! is 256 KiB, so the default size passes only where RELAY_MAX_BLOB_BYTES was raised
//! (NC-27). Nobody holds the address, so nobody can read or drain what it stores.
//!
//!   cargo run -p pacific-core --example relay_probe -- wss://arc.wallflowers.io/v1/relay [kib]
//!
//! Exit 0 accepted, 1 refused, 2 no connection.

use pacific_core::transport::RelaySession;
use pacific_wire::address::Address;
use rand_core::RngCore;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let url = args.next().unwrap_or_else(|| {
        eprintln!("usage: relay_probe <relay url> [kib]");
        std::process::exit(2)
    });
    let kib: usize = args.next().map(|s| s.parse().expect("kib is a number")).unwrap_or(300);
    let mut seed = [0u8; 32];
    rand_core::OsRng.fill_bytes(&mut seed);
    let mut blob = vec![0u8; kib * 1024];
    rand_core::OsRng.fill_bytes(&mut blob);
    let b64 = pacific_wire::blob_b64(&blob);
    let wire = b64.len();
    let mut relay = RelaySession::connect(&url).await.unwrap_or_else(|e| {
        eprintln!("relay_probe: no connection to {url}: {e}");
        std::process::exit(2)
    });
    match relay.publish(&Address::from_seed(&seed), b64).await {
        Ok(seq) => println!("accepted: {kib} KiB ({wire} bytes on the wire), seq {seq}"),
        Err(e) => {
            println!("refused: {kib} KiB ({wire} bytes on the wire): {e}");
            std::process::exit(1)
        }
    }
}
