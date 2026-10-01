//! token — Sign in with WallFlowers, for a public site (RD.4; mdr/door.md §5).
//!
//! An authorisation-code callback (Ralph, 26 Sep: "It needs to be a proper
//! callback"): the Door's window signs the person in and redirects to the client's
//! REGISTERED callback with a code and the state; the site's page trades the code
//! and its PKCE verifier for the session token, which it spends with a key it
//! cannot export (DPoP, RFC 9449; SEC-39). The code is single-use, lives 60
//! seconds, and is bound to the client, the callback, the session and the S256
//! challenge. The token never rides a URL. It is bound to one session, one Site
//! and one key.
//!
//! REGISTRATION IS A STAND-IN. D-32 puts a Site's registration of its origins in
//! ICD-0 as a Site op; until that lands this reads `DOOR_CLIENTS`, a local file,
//! and every answer that rests on it says so. The file is read again when it changes
//! (`Grants::reload`, polled every `RELOAD_EVERY`; R3.1 (B)): held to the rules the start
//! holds it to, and refused whole, the clients before it kept, where it fails one.

use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The Site of a client with no fixed Site (R3.1 (C), the hackathon's `hackathon-local`): its
/// person chooses one of theirs at the Door's Site step, where they are the owner or an admin.
/// Local work only: every callback and origin is http://localhost:<port> or
/// http://127.0.0.1:<port>, and it has no home and no slug (`check_wild`).
pub const WILD: &str = "*";
/// How long a wild client's session waits for its person to choose the Site.
pub const PICK_LIFE: Duration = Duration::from_secs(5 * 60);
const CODE_LIFE: Duration = Duration::from_secs(60);
/// How long a new account's visitor may take over the words before continuing to the
/// site: the code is minted when they continue, not before (NC-53; TBD-13).
pub const WORDS_LIFE: Duration = Duration::from_secs(15 * 60);
/// How far a proof's `iat` may sit from this Door's clock.
const PROOF_SKEW: u64 = 60;
/// Live proof ids one address may hold at /v2/token, where nothing else bounds it.
const PROOFS_PER_ADDR: usize = 60;
/// How often the Door looks at `DOOR_CLIENTS` for a change (R3.1 (B)).
pub const RELOAD_EVERY: Duration = Duration::from_secs(2);

/// What a client's sessions reach: its Site and what they mint there. `account` is
/// parsed only to be refused: no client reaches the whole account (D-33, O-58; W-76's
/// scope withdrawn, Software Security: fail closed).
#[derive(Deserialize, Clone, Copy, Default, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Reach {
    #[default]
    Site,
    Account,
}

/// A client, as its Site would register it.
#[derive(Deserialize, Clone)]
pub struct Client {
    /// What the window shows the person before they sign in (consent, §5 step 3).
    #[serde(default)]
    pub name: String,
    /// The callback URIs, each matched as a whole string and redirected to as
    /// registered, never as requested (CS-39).
    pub callbacks: Vec<String>,
    /// The origins the site's page calls from (CORS).
    pub origins: Vec<String>,
    /// The Site the client's sessions are scoped to: an object id, or `"*"` (`WILD`): the one
    /// its person chooses after the passkey.
    pub site: String,
    #[serde(default)]
    pub scope: Reach,
    /// Where a visitor who joined the Site by claim goes next (J-A): an exact URL on one of
    /// this client's own origins, held there at load (`check_homes`) as callbacks are held
    /// to the registration. None, and a join stays in the webapp.
    #[serde(default)]
    pub home: Option<String>,
    /// The Site's address, wallflowers.io/<slug>: the Face whose name, look and mark the
    /// sign-in window wears for this client (face.rs). None, and the window is the Door's.
    #[serde(default)]
    pub slug: Option<String>,
}

/// No registration carries the whole account's scope: the passkey's ceremony and the
/// account are the Door's own host's alone (D-33, O-58). One that does stops the Door.
pub fn check_reach(clients: &HashMap<String, Client>) -> Result<(), String> {
    let mut account: Vec<&String> = clients.iter().filter(|(_, c)| c.scope == Reach::Account).map(|(id, _)| id).collect();
    account.sort();
    match account.first() {
        Some(id) => Err(format!("{id:?} is registered with the whole account's scope, which no client may carry (O-58)")),
        None => Ok(()),
    }
}

/// A registration's `home` is on one of its own origins, or the Door does not start: a home
/// off them would send a joined visitor wherever the file said. The first off, by client,
/// is named with the client.
pub fn check_homes(clients: &HashMap<String, Client>) -> Result<(), String> {
    let origin = |u: &str| reqwest::Url::parse(u).ok().map(|u| u.origin()).filter(|o| o.is_tuple()).map(|o| o.ascii_serialization());
    let mut ids: Vec<&String> = clients.keys().collect();
    ids.sort();
    for id in ids {
        let c = &clients[id];
        let Some(home) = c.home.as_deref() else { continue };
        let at = origin(home);
        if at.is_none() || !c.origins.iter().any(|o| origin(o) == at) {
            return Err(format!("{id:?}'s home {home} is on none of its registered origins"));
        }
    }
    Ok(())
}

/// A client with no fixed Site is for local work: every callback and origin
/// http://localhost:<port> or http://127.0.0.1:<port>, exactly as written, and no home and no
/// slug. The first that is not, by client, is named with the client.
pub fn check_wild(clients: &HashMap<String, Client>) -> Result<(), String> {
    let loopback = |u: &str, callback: bool| {
        reqwest::Url::parse(u).is_ok_and(|p| {
            p.scheme() == "http"
                && matches!(p.host_str(), Some("localhost" | "127.0.0.1"))
                && p.port().is_some()
                && if callback { p.as_str() == u } else { p.origin().ascii_serialization() == u }
        })
    };
    let mut ids: Vec<&String> = clients.keys().filter(|id| clients[*id].site == WILD).collect();
    ids.sort();
    for id in ids {
        let c = &clients[id];
        if let Some(home) = &c.home {
            return Err(format!("{id:?} has no fixed Site, so no home: {home}"));
        }
        if let Some(slug) = &c.slug {
            return Err(format!("{id:?} has no fixed Site, so no slug: {slug}"));
        }
        let off = c.callbacks.iter().map(|u| (u, true)).chain(c.origins.iter().map(|u| (u, false))).find(|(u, cb)| !loopback(u, *cb));
        if let Some((u, _)) = off {
            return Err(format!("{id:?} has no fixed Site: {u} is not http://localhost:<port> or http://127.0.0.1:<port>"));
        }
    }
    Ok(())
}

/// An S256 PKCE challenge: 43 base64url characters that decode to 32 bytes.
pub fn s256(challenge: &str) -> bool {
    challenge.len() == 43 && B64.decode(challenge).map(|v| v.len()) == Ok(32)
}

/// A registration, as one read of `DOOR_CLIENTS` found it: parsed, and held to every rule.
pub fn parse(text: &str) -> Result<HashMap<String, Client>, String> {
    let clients: HashMap<String, Client> = serde_json::from_str(text).map_err(|e| e.to_string())?;
    check_reach(&clients)?;
    check_homes(&clients)?;
    check_wild(&clients)?;
    Ok(clients)
}

/// What a read of the file is told apart by: modified, length, inode. None: not there.
type Stamp = Option<(Option<SystemTime>, u64, u64)>;

fn stamp(path: &str) -> Stamp {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| (m.modified().ok(), m.len(), m.ino()))
}

/// `DOOR_CLIENTS`, and the stamp of the file last read.
struct Source {
    path: String,
    seen: Mutex<Stamp>,
}

pub struct Grants {
    /// Swapped whole by a reload: a request reads one registration throughout.
    clients: RwLock<Arc<HashMap<String, Client>>>,
    source: Option<Source>,
    codes: Mutex<HashMap<String, Code>>,
    tokens: Mutex<HashMap<String, Token>>,
    /// Proof ids seen, so a proof is spent once, with who presented them.
    jtis: Mutex<HashMap<String, (Instant, String)>>,
    /// Sign-ups whose visitor is still keeping the words, by the handle the window holds.
    pending: Mutex<HashMap<String, Pending>>,
    /// Wild clients' sessions waiting for their person to choose the Site, by the handle the
    /// Site step's cookie holds.
    picks: Mutex<HashMap<String, Pick>>,
    pub token_life: Duration,
}

/// A sign-up taken to continue: what its code is minted from.
pub struct Continued {
    pub client: String,
    pub callback: String,
    challenge: String,
    pub state: String,
    pub session: String,
}

/// A sign-up for a site, its account made, waiting for the visitor to keep the words.
struct Pending {
    client: String,
    callback: String,
    challenge: String,
    state: String,
    session: String,
    made: Instant,
}

struct Code {
    client: String,
    callback: String,
    challenge: String,
    session: String,
    made: Instant,
    /// A wild client's: the Site its person chose.
    site: Option<String>,
}

/// A wild client's session, signed in, before its person has chosen the Site.
#[derive(Clone)]
pub struct Pick {
    pub client: String,
    pub callback: String,
    challenge: String,
    pub state: String,
    pub session: String,
    made: Instant,
}

#[derive(Clone)]
pub struct Token {
    pub session: String,
    client: String,
    /// The Site the token reads.
    pub site: String,
    jkt: String,
    expires: Instant,
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn random_hex(n: usize) -> String {
    let mut b = vec![0u8; n];
    rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut b);
    hex::encode(b)
}

impl Grants {
    /// The stand-in registration: `DOOR_CLIENTS` names a JSON file of
    /// `{client_id: {callbacks, origins, site}}`. None, and no public site signs in.
    /// One that fails at start stops the Door; after it, `reload` refuses and keeps.
    pub fn load(path: Option<String>, token_life: Duration) -> Self {
        let source = path.map(|p| Source { seen: Mutex::new(stamp(&p)), path: p });
        let clients = source
            .as_ref()
            .map(|s| {
                let text = std::fs::read_to_string(&s.path).unwrap_or_else(|e| panic!("DOOR_CLIENTS {}: {e}", s.path));
                parse(&text).unwrap_or_else(|e| panic!("DOOR_CLIENTS {}: {e}", s.path))
            })
            .unwrap_or_default();
        Self {
            clients: RwLock::new(Arc::new(clients)),
            source,
            codes: Mutex::new(HashMap::new()),
            tokens: Mutex::new(HashMap::new()),
            jtis: Mutex::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
            picks: Mutex::new(HashMap::new()),
            token_life,
        }
    }

    /// The registration as it stands: one read, whatever a reload does meanwhile.
    fn registered(&self) -> Arc<HashMap<String, Client>> {
        self.clients.read().unwrap().clone()
    }

    /// Read `DOOR_CLIENTS` again if it changed since it was last read: None if it did not, or
    /// there is none; else how many clients it registers now, or why it was refused, the
    /// clients before it kept. Never a panic: the Door is running.
    pub fn reload(&self) -> Option<Result<usize, String>> {
        let src = self.source.as_ref()?;
        let now = stamp(&src.path);
        if std::mem::replace(&mut *src.seen.lock().unwrap(), now) == now {
            return None;
        }
        let next = std::fs::read_to_string(&src.path).map_err(|e| e.to_string()).and_then(|t| parse(&t));
        Some(next.map(|c| {
            let n = c.len();
            self.swap(c);
            n
        }))
    }

    /// Where `DOOR_CLIENTS` is, if the Door reads one.
    pub fn source(&self) -> Option<&str> {
        self.source.as_ref().map(|s| s.path.as_str())
    }

    /// The registration becomes `next`. A grant of a client no longer registered, or now
    /// registered for another Site, goes: its codes, its sign-ups waiting, its tokens. Their
    /// sessions are then held by nothing, and the sweeper ends them.
    fn swap(&self, next: HashMap<String, Client>) {
        let before = std::mem::replace(&mut *self.clients.write().unwrap(), Arc::new(next));
        let now = self.registered();
        let kept = |client: &str| matches!((before.get(client), now.get(client)), (Some(a), Some(b)) if a.site == b.site);
        self.codes.lock().unwrap().retain(|_, c| kept(&c.client));
        self.pending.lock().unwrap().retain(|_, p| kept(&p.client));
        self.picks.lock().unwrap().retain(|_, p| kept(&p.client));
        self.tokens.lock().unwrap().retain(|_, t| kept(&t.client));
    }

    /// The home of the client registered for `site` (J-A), if one names a home; with two, the
    /// first by client id, so the answer is the same on every start.
    pub fn home_of_site(&self, site: &str) -> Option<String> {
        let clients = self.registered();
        let mut ids: Vec<&String> = clients.keys().collect();
        ids.sort();
        ids.into_iter().map(|id| &clients[id]).find(|c| c.site == site && c.site != WILD && c.home.is_some()).and_then(|c| c.home.clone())
    }

    /// Whether some client registered `origin`: the CORS check for token calls, made on each
    /// request against the registration as it stands.
    pub fn allows_origin(&self, origin: &str) -> bool {
        self.registered().values().any(|c| c.origins.iter().any(|o| o == origin))
    }

    /// A registered client, by its id: what the window wears for it is public.
    pub fn client(&self, client: &str) -> Option<Client> {
        self.registered().get(client).cloned()
    }

    /// The address a registered client names for `site`, where one does.
    pub fn slug_of_site(&self, site: &str) -> Option<String> {
        self.registered().values().filter(|c| c.site == site && c.site != WILD).find_map(|c| c.slug.clone())
    }

    /// A registered client and the callback it registered that matches `asked` as a
    /// whole string, returned as REGISTERED (DT-9). The request only looks it up.
    pub fn callback(&self, client: &str, asked: &str) -> Result<(Client, String), String> {
        let c = self.client(client).ok_or_else(|| format!("no client {client:?} is registered"))?;
        let cb = c
            .callbacks
            .iter()
            .find(|cb| cb.as_str() == asked)
            .cloned()
            .ok_or_else(|| format!("{asked:?} is not a callback client {client:?} registered"))?;
        Ok((c, cb))
    }

    /// A code for a session that has just opened for `client`, to go to `callback`.
    pub fn code(&self, client: &str, callback: &str, challenge: &str, session: &str) -> Result<String, String> {
        self.mint(client, callback, challenge, session, None)
    }

    fn mint(&self, client: &str, callback: &str, challenge: &str, session: &str, site: Option<String>) -> Result<String, String> {
        if !s256(challenge) {
            return Err("code_challenge is not an S256 challenge".into());
        }
        let code = random_hex(32);
        self.codes.lock().unwrap().insert(
            code.clone(),
            Code {
                client: client.into(),
                callback: callback.into(),
                challenge: challenge.into(),
                session: session.into(),
                made: Instant::now(),
                site,
            },
        );
        Ok(code)
    }

    /// Hold a wild client's session for its person to choose the Site: answers the handle the
    /// Site step's cookie carries. No code until they choose.
    pub fn pick(&self, client: &str, callback: &str, challenge: &str, state: &str, session: &str) -> Result<String, String> {
        if !s256(challenge) {
            return Err("code_challenge is not an S256 challenge".into());
        }
        let handle = random_hex(32);
        let p = Pick { client: client.into(), callback: callback.into(), challenge: challenge.into(), state: state.into(), session: session.into(), made: Instant::now() };
        self.picks.lock().unwrap().insert(handle.clone(), p);
        Ok(handle)
    }

    /// The choice waiting under `handle`, within `PICK_LIFE`; taken, once, where `take`.
    pub fn picking(&self, handle: &str, take: bool) -> Option<Pick> {
        let mut picks = self.picks.lock().unwrap();
        let p = if take { picks.remove(handle) } else { picks.get(handle).cloned() };
        p.filter(|p| p.made.elapsed() <= PICK_LIFE)
    }

    /// The code for a choice taken: the chosen Site's, as a fixed client's is its own.
    pub fn code_at(&self, p: &Pick, site: &str) -> Result<String, String> {
        self.mint(&p.client, &p.callback, &p.challenge, &p.session, Some(site.into()))
    }

    /// Hold a site's sign-up while its visitor keeps the words: answers the handle the
    /// window continues with. The code waits until then (NC-53).
    pub fn pend(&self, client: &str, callback: &str, challenge: &str, state: &str, session: &str) -> Result<String, String> {
        if !s256(challenge) {
            return Err("code_challenge is not an S256 challenge".into());
        }
        let handle = random_hex(32);
        self.pending.lock().unwrap().insert(
            handle.clone(),
            Pending { client: client.into(), callback: callback.into(), challenge: challenge.into(), state: state.into(), session: session.into(), made: Instant::now() },
        );
        Ok(handle)
    }

    /// The visitor has kept the words: once, within `WORDS_LIFE`, the sign-up is taken.
    /// Answers its session, and a closure-free mint: the caller checks the session is
    /// still there before `mint_for` makes the code (NC-53).
    pub fn take_pending(&self, handle: &str, now: Instant) -> Result<Continued, String> {
        let p = self.pending.lock().unwrap().remove(handle).ok_or("no such sign-up, or it has continued")?;
        if now.saturating_duration_since(p.made) > WORDS_LIFE {
            return Err("the sign-up waited too long; sign in to continue".into());
        }
        Ok(Continued { client: p.client, callback: p.callback, challenge: p.challenge, state: p.state, session: p.session })
    }

    /// The code for a taken sign-up.
    pub fn mint_for(&self, c: &Continued) -> Result<String, String> {
        self.code(&c.client, &c.callback, &c.challenge, &c.session)
    }

    /// A taken sign-up of a wild client's: its Site step.
    pub fn pick_for(&self, c: &Continued) -> Result<String, String> {
        self.pick(&c.client, &c.callback, &c.challenge, &c.state, &c.session)
    }

    /// A session held by a sign-up whose visitor is still keeping the words: exempt from
    /// the idle end until `WORDS_LIFE` (NC-53, Software Testing's E15).
    pub fn pending(&self, session: &str) -> bool {
        self.pending.lock().unwrap().values().any(|p| p.session == session && p.made.elapsed() <= WORDS_LIFE)
            || self.picks.lock().unwrap().values().any(|p| p.session == session && p.made.elapsed() <= PICK_LIFE)
    }

    /// Spend a code: once, within its minute, by its client, with its verifier, and
    /// bind the token to the proof's key. Returns the token and its session.
    pub fn redeem(
        &self,
        code: &str,
        verifier: &str,
        client: &str,
        callback: &str,
        jkt: String,
    ) -> Result<(String, Token), String> {
        let c = self.codes.lock().unwrap().remove(code).ok_or("no such code, or it was spent")?;
        if c.made.elapsed() > CODE_LIFE {
            return Err("the code has expired".into());
        }
        if c.client != client || c.callback != callback {
            return Err("the code was issued to another client or callback".into());
        }
        // RFC 7636 §4.1: 43 to 128 unreserved characters.
        let unreserved = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~');
        if !(43..=128).contains(&verifier.len()) || !verifier.bytes().all(unreserved) {
            return Err("the code_verifier is not 43 to 128 unreserved characters".into());
        }
        if B64.encode(Sha256::digest(verifier.as_bytes())) != c.challenge {
            return Err("the code_verifier does not match the challenge".into());
        }
        let site = match self.client(client).map(|c| c.site).ok_or("the client is gone")? {
            wild if wild == WILD => c.site.ok_or("the code names no Site")?,
            fixed => fixed,
        };
        let token = random_hex(32);
        let t = Token { session: c.session, client: client.into(), site, jkt, expires: Instant::now() + self.token_life };
        self.tokens.lock().unwrap().insert(token.clone(), t.clone());
        Ok((token, t))
    }

    /// The token a request carries, if its proof holds: live, and proven by the
    /// key it was bound to, for this method and URL, once.
    pub fn spend(&self, token: &str, proof: &str, htm: &str, htu: &str) -> Result<Token, String> {
        let t = self.tokens.lock().unwrap().get(token).cloned().ok_or("no such token, or it was revoked")?;
        if Instant::now() > t.expires {
            self.tokens.lock().unwrap().remove(token);
            return Err("the token has expired".into());
        }
        let jkt = self.proof(proof, htm, htu, Some(token), "")?;
        if jkt != t.jkt {
            return Err("the proof is not by the key this token is bound to".into());
        }
        Ok(t)
    }

    pub fn revoke(&self, token: &str) -> Option<Token> {
        self.tokens.lock().unwrap().remove(token)
    }

    /// Codes never redeemed name sessions nobody holds: those sessions, to end.
    pub fn sweep(&self) -> Vec<String> {
        self.tokens.lock().unwrap().retain(|_, t| Instant::now() <= t.expires);
        self.jtis.lock().unwrap().retain(|_, (at, _)| at.elapsed() < Duration::from_secs(2 * PROOF_SKEW));
        let mut ended: Vec<String> = {
            let mut pending = self.pending.lock().unwrap();
            let stale: Vec<String> = pending.iter().filter(|(_, p)| p.made.elapsed() > WORDS_LIFE).map(|(k, _)| k.clone()).collect();
            stale.into_iter().filter_map(|k| pending.remove(&k)).map(|p| p.session).collect()
        };
        self.picks.lock().unwrap().retain(|_, p| {
            let live = p.made.elapsed() <= PICK_LIFE;
            if !live {
                ended.push(p.session.clone());
            }
            live
        });
        let mut codes = self.codes.lock().unwrap();
        let stale: Vec<String> = codes.iter().filter(|(_, c)| c.made.elapsed() > CODE_LIFE).map(|(k, _)| k.clone()).collect();
        ended.extend(stale.into_iter().filter_map(|k| codes.remove(&k)).map(|c| c.session));
        ended
    }

    /// A session no token names any more.
    pub fn holds(&self, session: &str) -> bool {
        self.tokens.lock().unwrap().values().any(|t| t.session == session)
            || self.codes.lock().unwrap().values().any(|c| c.session == session)
            || self.pending.lock().unwrap().values().any(|p| p.session == session)
            || self.picks.lock().unwrap().values().any(|p| p.session == session)
    }

    /// Check a DPoP proof (RFC 9449 §4.3) and answer its key's thumbprint.
    /// `who` is the keyed hash of the caller's address where no token vouches for
    /// it (/v2/token): one address may hold at most `PROOFS_PER_ADDR` live proof ids.
    pub fn proof(&self, proof: &str, htm: &str, htu: &str, token: Option<&str>, who: &str) -> Result<String, String> {
        let parts: Vec<&str> = proof.split('.').collect();
        let [h, p, s] = parts[..] else { return Err("the DPoP proof is not a JWT".into()) };
        let header: serde_json::Value = serde_json::from_slice(&B64.decode(h).map_err(|_| "proof header")?)
            .map_err(|_| "proof header")?;
        let claims: serde_json::Value = serde_json::from_slice(&B64.decode(p).map_err(|_| "proof claims")?)
            .map_err(|_| "proof claims")?;
        if header["typ"] != "dpop+jwt" || header["alg"] != "ES256" {
            return Err("the proof is not a dpop+jwt signed ES256".into());
        }
        let jwk = &header["jwk"];
        let (x, y) = (jwk["x"].as_str().unwrap_or(""), jwk["y"].as_str().unwrap_or(""));
        if jwk["kty"] != "EC" || jwk["crv"] != "P-256" {
            return Err("the proof's key is not P-256".into());
        }
        let (xb, yb) = (B64.decode(x).map_err(|_| "jwk x")?, B64.decode(y).map_err(|_| "jwk y")?);
        if xb.len() != 32 || yb.len() != 32 {
            return Err("the proof's key is not P-256".into());
        }
        let mut point = vec![4u8];
        point.extend_from_slice(&xb);
        point.extend_from_slice(&yb);
        use p256::ecdsa::signature::Verifier;
        let key = p256::ecdsa::VerifyingKey::from_sec1_bytes(&point).map_err(|_| "the proof's key is not on P-256")?;
        let sig = p256::ecdsa::Signature::from_slice(&B64.decode(s).map_err(|_| "proof signature")?)
            .map_err(|_| "proof signature")?;
        key.verify(format!("{h}.{p}").as_bytes(), &sig).map_err(|_| "the proof's signature does not verify")?;
        if claims["htm"] != htm || claims["htu"] != htu {
            return Err(format!("the proof is for {} {}, not {htm} {htu}", claims["htm"], claims["htu"]));
        }
        let iat = claims["iat"].as_u64().ok_or("the proof has no iat")?;
        if iat.abs_diff(now_secs()) > PROOF_SKEW {
            return Err("the proof is stale".into());
        }
        if let Some(t) = token {
            if claims["ath"] != B64.encode(Sha256::digest(t.as_bytes())).as_str() {
                return Err("the proof is not for this token".into());
            }
        }
        let jti = claims["jti"].as_str().filter(|j| !j.is_empty() && j.len() <= 128).ok_or("the proof has no jti")?;
        let mut jtis = self.jtis.lock().unwrap();
        if jtis.contains_key(jti) {
            return Err("the proof was already used".into());
        }
        if !who.is_empty() && jtis.values().filter(|(_, w)| w == who).count() >= PROOFS_PER_ADDR {
            return Err("too many token requests from here; wait a minute".into());
        }
        jtis.insert(jti.to_string(), (Instant::now(), who.to_string()));
        drop(jtis);
        // RFC 7638: the thumbprint over the required members, in order.
        let canonical = format!(r#"{{"crv":"P-256","kty":"EC","x":"{x}","y":"{y}"}}"#);
        Ok(B64.encode(Sha256::digest(canonical.as_bytes())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    fn registered(json: serde_json::Value) -> HashMap<String, Client> {
        serde_json::from_value(json).unwrap()
    }

    /// A mistyped client or callback is refused in words a site's builder can act on, and
    /// nothing internal (TEST, 1 Oct: "; client registration is a local stand-in (D-32)").
    #[test]
    fn a_callback_refusal_names_what_was_asked_and_nothing_internal() {
        let path = std::env::temp_dir().join(format!("door-clients-words-{}.json", std::process::id()));
        let entry = serde_json::json!({ "name": "", "callbacks": ["https://client.example.test/cb"], "origins": ["https://client.example.test"], "site": "ab" });
        std::fs::write(&path, serde_json::json!({ "dt-client": entry }).to_string()).unwrap();
        let g = Grants::load(Some(path.to_string_lossy().into_owned()), std::time::Duration::from_secs(60));
        let _ = std::fs::remove_file(&path);
        assert_eq!(g.callback("nobody", "https://client.example.test/cb").err().as_deref(), Some(r#"no client "nobody" is registered"#));
        assert_eq!(
            g.callback("dt-client", "https://elsewhere.example.test/cb").err().as_deref(),
            Some(r#""https://elsewhere.example.test/cb" is not a callback client "dt-client" registered"#)
        );
    }

    /// No registration reaches the whole account, whatever its origin: any that claims
    /// it stops the Door at start, naming the entry (O-58).
    #[test]
    fn no_registration_reaches_the_whole_account() {
        let entry = |origin: &str, scope: &str| {
            serde_json::json!({ "name": "WallFlowers", "callbacks": ["https://www.wallflowers.io/signin"], "origins": [origin], "site": "", "scope": scope })
        };
        let site = serde_json::json!({ "callbacks": ["https://egregores-echoes.com/signin/callback"], "origins": ["https://egregores-echoes.com"], "site": "ab" });
        let ok = registered(serde_json::json!({ "egregores-echoes.com": site.clone() }));
        assert_eq!(check_reach(&ok), Ok(()));
        assert_eq!(ok["egregores-echoes.com"].scope, Reach::Site, "no scope is the Site's");

        let www = registered(serde_json::json!({ "www.wallflowers.io": entry("https://www.wallflowers.io", "account"), "egregores-echoes.com": site }));
        assert!(check_reach(&www).unwrap_err().contains("\"www.wallflowers.io\""), "the website's own is refused, named");
        let other = registered(serde_json::json!({ "evil": entry("https://evil.example", "account") }));
        assert!(check_reach(&other).unwrap_err().contains("\"evil\""), "the entry is named");
        assert!(serde_json::from_value::<HashMap<String, Client>>(serde_json::json!({ "x": entry("https://evil.example", "acount") })).is_err(),
                "a scope it does not know is no registration");
    }

    /// J-A: a home on the client's own origins loads; one off them, or not a URL, is refused,
    /// naming the client and the home.
    #[test]
    fn a_home_is_on_its_clients_own_origins() {
        let with = |home: &str| {
            registered(serde_json::json!({ "egregores-echoes.com": {
                "callbacks": ["https://egregores-echoes.com/signin/callback"], "origins": ["https://egregores-echoes.com"],
                "site": "ab", "home": home } }))
        };
        assert_eq!(check_homes(&with("https://egregores-echoes.com/community")), Ok(()));
        for off in ["https://evil.example/community", "http://egregores-echoes.com/community", "https://egregores-echoes.com:8443/community", "javascript:alert(1)", "/community"] {
            let e = check_homes(&with(off)).unwrap_err();
            assert!(e.contains("\"egregores-echoes.com\"") && e.contains(off), "{off}: {e}");
        }
        assert_eq!(check_homes(&registered(serde_json::json!({ "x": { "callbacks": [], "origins": [], "site": "ab" } }))), Ok(()), "no home, nothing to hold");
    }

    /// The registration the deploy ships loads as the Door loads it, and holds Egregore's
    /// home (EGREGORE, 29 Sep): a bad value here would stop door-01 at start.
    #[test]
    fn the_shipped_stand_in_loads_and_names_egregores_home() {
        let clients: HashMap<String, Client> = serde_json::from_str(include_str!("../clients.stand-in.json")).unwrap();
        assert_eq!(check_reach(&clients), Ok(()));
        assert_eq!(check_homes(&clients), Ok(()));
        assert_eq!(clients["egregores-echoes.com"].home.as_deref(), Some("https://egregores-echoes.com/community"));
        assert_eq!(check_wild(&clients), Ok(()));
        let wild = &clients["hackathon-local"];
        assert_eq!((wild.site.as_str(), wild.callbacks.len(), wild.origins.len()), (WILD, 50, 30), "R3.1 (C): /callback on ports 3000 to 3009, and the page itself (\"/\", the starter's callback) on those and 5173 to 5175, 8000 and 8080, on both loopback names; an origin for each port");
        for port in [3000, 3009, 5173, 5174, 5175, 8000, 8080] {
            for host in ["localhost", "127.0.0.1"] {
                assert!(wild.callbacks.contains(&format!("http://{host}:{port}/")), "the starter's page on {host}:{port}");
                assert!(wild.origins.contains(&format!("http://{host}:{port}")), "{host}:{port}'s origin");
            }
        }
    }

    fn challenge() -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7u8; 32])
    }

    /// NC-53: a visitor who takes five minutes over the words still continues to the
    /// site. The session is held meanwhile, and the code is minted when they continue.
    #[test]
    fn a_sign_up_continues_after_five_minutes_over_the_words() {
        let g = Grants::load(None, Duration::from_secs(3600));
        let h = g.pend("site.example", "https://site.example/cb", &challenge(), "st", "sess").unwrap();
        assert!(g.holds("sess") && g.pending("sess"), "the session is held, and exempt from idle, while the words are kept");
        let later = Instant::now() + Duration::from_secs(5 * 60);
        let c = g.take_pending(&h, later).expect("five minutes over the words, and it continues");
        assert_eq!((c.callback.as_str(), c.state.as_str(), c.session.as_str()), ("https://site.example/cb", "st", "sess"));
        g.mint_for(&c).expect("the code, minted now");
        assert!(g.holds("sess") && !g.pending("sess"), "held now by its code, and idle again");
        assert!(g.take_pending(&h, later).is_err(), "a sign-up continues once");
        let h2 = g.pend("site.example", "https://site.example/cb", &challenge(), "st", "sess2").unwrap();
        assert!(
            g.take_pending(&h2, Instant::now() + WORDS_LIFE + Duration::from_secs(1)).is_err(),
            "not past WORDS_LIFE"
        );
    }
}
