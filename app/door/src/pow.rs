//! pow — the proof of work a sign-up or sign-in without a kiosk claim pays before
//! it starts (D-55). About a second of a phone's CPU, spent in a Worker during
//! consent (`app/web/door/pow-worker.js`); a flood pays it per attempt.
//!
//! The contract, stated too in pow-worker.js:
//!   challenge  v1.<seed>.<issued>.<bits>.<mac>
//!              seed    16 random bytes, base64url without padding
//!              issued  unix seconds, decimal
//!              bits    leading zero bits the work must reach, decimal
//!              mac     HMAC-SHA256(key, "v1.<seed>.<issued>.<bits>.<for>"), base64url
//!                      without padding; `for` is the start it was issued for, signup
//!                      or signin, bound but not written
//!   nonce      the count 0, 1, 2, … as ASCII decimal, no leading zeros
//!   work       SHA-256(seed bytes ‖ nonce digits) has `bits` leading zero bits
//!
//! The key is made at start and never leaves the process, so the Door holds nothing
//! per challenge until one is spent, and a restart voids every challenge outstanding.

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;

/// 2^20 tries expected: ~1 s on a mid-range phone, taken as 3–5x slower than the
/// SHA-256/s pow-worker.js makes on an M5 Max (4.7M in node, 3.4M in JavaScriptCore).
pub const BITS: u32 = 20;
/// Seconds a challenge may be spent after it is issued.
const CHALLENGE_LIFE: u64 = 120;

/// The start a challenge is issued for: under its MAC, so a sign-in's cannot be
/// presented for a sign-up.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Start {
    SignUp,
    SignIn,
}

impl Start {
    fn word(self) -> &'static str {
        match self {
            Start::SignUp => "signup",
            Start::SignIn => "signin",
        }
    }

    fn other(self) -> Start {
        match self {
            Start::SignUp => Start::SignIn,
            Start::SignIn => Start::SignUp,
        }
    }
}

pub struct Pow {
    key: [u8; 32],
    /// Seeds spent, with when their challenge was issued: a seed is forgotten only
    /// once its challenge would be refused as expired anyway.
    spent: Mutex<HashMap<[u8; 16], u64>>,
}

/// HMAC-SHA256 (RFC 2104) from sha2 alone. The key is shorter than the block, so
/// it is zero-padded, never hashed.
fn hmac(key: &[u8; 32], msg: &[u8]) -> [u8; 32] {
    let (mut ipad, mut opad) = ([0x36u8; 64], [0x5cu8; 64]);
    for (i, k) in key.iter().enumerate() {
        ipad[i] ^= k;
        opad[i] ^= k;
    }
    let inner = Sha256::new().chain_update(ipad).chain_update(msg).finalize();
    Sha256::new().chain_update(opad).chain_update(inner).finalize().into()
}

/// Equal, in time that does not say where they differ.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |d, (x, y)| d | (x ^ y)) == 0
}

fn zeros(d: &[u8]) -> u32 {
    let mut n = 0;
    for b in d {
        n += b.leading_zeros();
        if *b != 0 {
            break;
        }
    }
    n
}

impl Pow {
    pub fn new() -> Self {
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        Self { key, spent: Mutex::new(HashMap::new()) }
    }

    pub fn issue(&self, now: u64, start: Start) -> String {
        self.issue_at(now, BITS, start)
    }

    fn issue_at(&self, now: u64, bits: u32, start: Start) -> String {
        let mut seed = [0u8; 16];
        OsRng.fill_bytes(&mut seed);
        self.sign(&seed, now, bits, start)
    }

    fn mac(&self, body: &str, start: Start) -> [u8; 32] {
        hmac(&self.key, format!("{body}.{}", start.word()).as_bytes())
    }

    fn sign(&self, seed: &[u8; 16], issued: u64, bits: u32, start: Start) -> String {
        let body = format!("v1.{}.{issued}.{bits}", B64.encode(seed));
        let mac = B64.encode(self.mac(&body, start));
        format!("{body}.{mac}")
    }

    /// A challenge this Door issued for `start`, live, worked by `nonce`, and spent here once.
    pub fn check(&self, challenge: &str, nonce: &str, now: u64, start: Start) -> Result<(), String> {
        let malformed = || "the challenge is malformed".to_string();
        let (body, mac) = challenge.rsplit_once('.').ok_or_else(malformed)?;
        let mac = B64.decode(mac).map_err(|_| malformed())?;
        // The MAC over the text as issued, before any of it is read.
        if !same(&mac, &self.mac(body, start)) {
            if same(&mac, &self.mac(body, start.other())) {
                return Err("the challenge was issued for another start".into());
            }
            return Err("the challenge is not this Door's".into());
        }
        let parts: Vec<&str> = body.split('.').collect();
        let ["v1", seed, issued, bits] = parts[..] else { return Err(malformed()) };
        let seed: [u8; 16] = B64.decode(seed).ok().and_then(|v| v.try_into().ok()).ok_or_else(malformed)?;
        let issued: u64 = issued.parse().map_err(|_| malformed())?;
        let bits: u32 = bits.parse().map_err(|_| malformed())?;
        if issued > now {
            return Err("the challenge is dated in the future".into());
        }
        if now - issued > CHALLENGE_LIFE {
            return Err("the challenge has expired".into());
        }
        // Canonical, so the digits hashed are the digits the worker counted.
        if !nonce.parse::<u64>().is_ok_and(|n| n.to_string() == nonce) {
            return Err("the nonce is not a decimal count".into());
        }
        if zeros(&Sha256::new().chain_update(seed).chain_update(nonce).finalize()) < bits {
            return Err("the nonce does not do the work".into());
        }
        if self.spent.lock().unwrap().insert(seed, issued).is_some() {
            return Err("the challenge was already spent".into());
        }
        Ok(())
    }

    /// Forget spent seeds whose challenges have expired.
    pub fn sweep(&self, now: u64) {
        self.spent.lock().unwrap().retain(|_, issued| now.saturating_sub(*issued) <= CHALLENGE_LIFE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_790_000_000;

    /// The vector pow-worker.test.mjs pins too: this key signs it, and the worker
    /// answers NONCE.
    const KEY: &[u8; 32] = b"wallflowers/door/pow/test-vector";
    const VECTOR: &str = "v1.AAECAwQFBgcICQoLDA0ODw.1790000000.16.y-rbL9BGUNtPum8ZPfzsZ0MOqzmk-lXx9Gh-0CkCCm0";
    const NONCE: &str = "34416";

    /// The first count from 0 whose work against `challenge` does, or does not, reach its bits.
    fn count(challenge: &str, reaches: bool) -> String {
        let p: Vec<&str> = challenge.split('.').collect();
        let (seed, bits) = (B64.decode(p[1]).unwrap(), p[3].parse::<u32>().unwrap());
        (0u64..)
            .map(|n| n.to_string())
            .find(|n| (zeros(&Sha256::new().chain_update(&seed).chain_update(n).finalize()) >= bits) == reaches)
            .unwrap()
    }

    /// The worker's answer.
    fn solve(challenge: &str) -> String {
        count(challenge, true)
    }

    #[test]
    fn hmac_is_rfc_4231_case_2() {
        // "Jefe" zero-padded to 32 bytes is "Jefe": HMAC pads a short key to the block.
        let mut key = [0u8; 32];
        key[..4].copy_from_slice(b"Jefe");
        assert_eq!(
            hex::encode(hmac(&key, b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn the_vector_the_worker_pins() {
        let pow = Pow { key: *KEY, spent: Mutex::new(HashMap::new()) };
        let seed: [u8; 16] = std::array::from_fn(|i| i as u8);
        assert_eq!(pow.sign(&seed, NOW, 16, Start::SignUp), VECTOR);
        assert_eq!(solve(VECTOR), NONCE, "the Door counts to the nonce the worker finds");
        assert_eq!(pow.check(VECTOR, NONCE, NOW + 1, Start::SignUp), Ok(()));
    }

    #[test]
    fn a_solved_challenge_passes_once() {
        let pow = Pow::new();
        let c = pow.issue_at(NOW, 8, Start::SignUp);
        let n = solve(&c);
        assert_eq!(pow.check(&c, &n, NOW + CHALLENGE_LIFE, Start::SignUp), Ok(()));
        assert_eq!(pow.check(&c, &n, NOW + CHALLENGE_LIFE, Start::SignUp), Err("the challenge was already spent".into()));
    }

    #[test]
    fn a_forged_expired_future_or_unworked_challenge_is_refused_by_name() {
        let pow = Pow::new();
        let c = pow.issue_at(NOW, 8, Start::SignUp);
        let n = solve(&c);
        let refused = |c: &str, n: &str, now| pow.check(c, n, now, Start::SignUp).unwrap_err();

        let other = Pow::new().issue_at(NOW, 8, Start::SignUp);
        assert_eq!(refused(&other, &solve(&other), NOW), "the challenge is not this Door's");
        let easier = c.replacen(".8.", ".0.", 1);
        assert_eq!(refused(&easier, "0", NOW), "the challenge is not this Door's");
        assert_eq!(refused("v1.x", &n, NOW), "the challenge is malformed");
        assert_eq!(refused("", &n, NOW), "the challenge is malformed");

        assert_eq!(refused(&c, &n, NOW + CHALLENGE_LIFE + 1), "the challenge has expired");
        assert_eq!(refused(&c, &n, NOW - 1), "the challenge is dated in the future");

        assert_eq!(refused(&c, &count(&c, false), NOW), "the nonce does not do the work");
        for bad in ["", "-1", "+1", "01", "1.0", " 1", "18446744073709551616"] {
            assert_eq!(refused(&c, bad, NOW), "the nonce is not a decimal count", "{bad:?}");
        }

        // None of those spent it.
        assert_eq!(pow.check(&c, &n, NOW, Start::SignUp), Ok(()));
    }

    #[test]
    fn a_challenge_is_spent_only_on_the_start_it_was_issued_for() {
        let pow = Pow::new();
        let c = pow.issue_at(NOW, 8, Start::SignIn);
        let n = solve(&c);
        assert_eq!(pow.check(&c, &n, NOW, Start::SignUp), Err("the challenge was issued for another start".into()));
        assert_eq!(pow.check(&c, &n, NOW, Start::SignIn), Ok(()), "refused, it was not spent");
    }

    #[test]
    fn a_spent_seed_is_forgotten_only_once_it_would_expire() {
        let pow = Pow::new();
        let c = pow.issue_at(NOW, 8, Start::SignUp);
        let n = solve(&c);
        pow.check(&c, &n, NOW, Start::SignUp).unwrap();
        pow.sweep(NOW + CHALLENGE_LIFE);
        assert_eq!(pow.check(&c, &n, NOW + CHALLENGE_LIFE, Start::SignUp), Err("the challenge was already spent".into()));
        pow.sweep(NOW + CHALLENGE_LIFE + 1);
        assert!(pow.spent.lock().unwrap().is_empty());
        assert_eq!(pow.check(&c, &n, NOW + CHALLENGE_LIFE + 1, Start::SignUp), Err("the challenge has expired".into()));
    }
}
