//! The Arc-served console CONTRACT — the wire shapes the (handed-off) frontend consumes.
//!
//! In the mesh model the ONLY admin console is the one served from within an Arc: it reads
//! state folded from THIS Arc's own pacific-core directory (fold, don't mirror — no side DB)
//! and issues role-gated commands to the Arcs it is tethered to. arc-node exposes it:
//!
//!   GET  /console/state    -> ConsoleState        (the whole read model, 1-hop mesh + members)
//!   POST /console/command  -> ConsoleCommand       -> CommandAck  (role-gated arc.* op on a tether)
//!   POST /tether           -> TetherRequest        -> TetherView  (form an Arc<->Arc tether)
//!   POST /console/role     -> RoleRequest          -> CommandAck  (group.setMemberRole on a tether)
//!   GET  /bundle           -> text/plain           (this Arc's contact bundle, for a peer to scan)
//!
//! This module is the CONTRACT only — the handlers live in the backend track. The mirror the
//! UI team compiles against is `src/console/src/contract.ts`; the `contract_json_is_stable`
//! test below pins the exact bytes so the two never drift.
//!
//! Versioning: `CONSOLE_API_VERSION` bumps on a BREAKING change. Additive fields (new optional
//! telemetry, new command verbs) are NOT breaking — every optional field is `skip_serializing_if`
//! and every enum is closed on the backend but forward-tolerant on the reader.
#![allow(dead_code)] // the contract types are consumed by the backend track + the test below.

use pacific_core::{CoreError, Node};
use serde::{Deserialize, Serialize};

/// Contract version. Bump ONLY on a breaking change (renamed/removed field, changed casing).
pub const CONSOLE_API_VERSION: &str = "console/v1";

/// The group `kind` an Arc↔Arc tether carries — the mesh link between two Arcs.
/// EVERY 2-MEMBER LINK IS A CONNECTION (24 Sep 2026). `arc-tether` and
/// `member-tether` were Group-typed twins of one, minted only because a role had
/// to ride a Group-folded log; standing is a facet now, so what the other party
/// is to us is the ROLE, not the kind.
pub const CONNECTION_KIND: &str = "connection";
pub const ARC_TETHER_KIND: &str = CONNECTION_KIND;

/// The group `kind` a {user, Arc} MEMBERSHIP tether carries.
///
/// Deliberately NOT `"connection"`: a connection's log is folded as ForumType (chat posts), and
/// a role must ride a log folded as GroupType so `group.setMemberRole` reads back correctly —
/// see `pacific_core::group::GROUP_TYPED_KINDS`. Membership is therefore its own 2-member
/// tether, mirroring `arc-tether`, and the role rides it to the member.
pub const MEMBER_TETHER_KIND: &str = CONNECTION_KIND;

// ── read model: GET /console/state ────────────────────────────────────────────

/// The whole console read model, folded from this Arc's directory at request time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleState {
    /// Always `CONSOLE_API_VERSION` — lets the UI detect a contract bump.
    pub version: String,
    /// Unix epoch millis when this snapshot was folded (server clock).
    pub generated_at_ms: u64,
    /// This Arc (self) — identity + its own operational telemetry.
    pub arc: ArcNode,
    /// The Arc↔Arc tethers this Arc holds — the mesh from its vantage (1-hop).
    pub peers: Vec<TetherView>,
    /// The user tethers this Arc owns — its membership (who signed up here).
    pub members: Vec<MemberView>,
}

/// An Arc identity plus its last-known telemetry. Used for both `self` and each peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArcNode {
    /// `space1` + hex(Ed25519 pubkey) — the stable peer id.
    pub identity_key: String,
    pub name: String,
    /// Colon-grouped SAS (first 8 pubkey bytes) for human verification.
    pub fingerprint: String,
    /// Folded from `arc.telemetry` deltas (peers) or local probe (self). `None` if never seen.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<Telemetry>,
}

/// One Arc↔Arc tether from this Arc's vantage — a 2-member GroupObject.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TetherView {
    /// Hex MLS group id — the tether's stable handle (target of `/console/command`).
    pub group_id: String,
    pub peer: ArcNode,
    /// The role I hold in this tether — what I can command of the peer.
    pub role_i_hold: Role,
    /// The role the peer holds — what they can command of me.
    pub role_peer_holds: Role,
    pub link: LinkHealth,
}

/// A user tethered to this Arc (a {user, Arc} connection this Arc owns).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberView {
    pub identity_key: String,
    pub name: String,
    /// Hex group id of the {user, Arc} tether.
    pub connection_id: String,
    pub role: Role,
}

/// Liveness of a tether, derived from the freshness of the peer's last folded delta.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkHealth {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen_ms: Option<u64>,
    /// `last_seen_ms` within the freshness window.
    pub healthy: bool,
}

/// Mirrors pacific-core `GroupRole` — all five variants. Lowercase on the wire
/// (`"guest"`/`"viewer"`/`"member"`/`"admin"`/`"owner"`).
///
/// Declared LEAST → MOST privileged so the derived `Ord` IS the gate check: a route states a
/// minimum (`role >= Role::Member`) and the comparison does the rest. Adding a variant in the
/// middle would silently re-rank every gate, so the order is load-bearing, not cosmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Relay only — can message, cannot reach the planes that spend the Arc's own resources.
    Guest,
    /// + read the mesh directory (`GET /v1/arcs`).
    Viewer,
    /// + connectors (`/v1/systems/*`), land (`/v1/land/*`) and any new gated plane. The signup default.
    Member,
    /// + assign roles to other members.
    Admin,
    /// The Arc's operator.
    Owner,
    /// ANOTHER ARC, federated with this one. Not a standing within a body but a
    /// standing on the LINK — and it is what tells a peer connection from a
    /// member connection now that both are the same kind.
    Peer,
}

impl Role {
    /// The role the Arc grants a member at signup, before any explicit assignment.
    pub const DEFAULT: Role = Role::Member;

    /// Fold pacific-core's `GroupRole` (the value that actually rode the tether) into the
    /// wire role. Total — every core variant has exactly one wire form.
    pub fn from_core(r: pacific_core::group::GroupRole) -> Self {
        use pacific_core::group::GroupRole as G;
        match r {
            G::Owner => Role::Owner,
            G::Admin => Role::Admin,
            G::Member => Role::Member,
            G::Viewer => Role::Viewer,
            G::Guest => Role::Guest,
            G::Peer => Role::Peer,
            // A Site's Add, not a plane's: an admitter keeps a member's standing here.
            G::Admitter => Role::Member,
            // A Transaction's principals (W-98 Trade), never a tether's: a member's standing here.
            G::Settler | G::Seller | G::Buyer => Role::Member,
        }
    }

    /// The string `group.setMemberRole` expects in its `role` arg (`GroupRole::parse`).
    pub fn as_core_str(&self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Admin => "admin",
            Role::Member => "member",
            Role::Viewer => "viewer",
            Role::Guest => "guest",
            Role::Peer => "peer",
        }
    }
}

/// Self-reported operational telemetry. Additive: new fields are optional, never breaking.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Telemetry {
    pub model_live: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_flight: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_util_pct: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_mem_used_mb: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_mem_total_mb: Option<f32>,
}

// ── command: POST /console/command ────────────────────────────────────────────

/// A role-gated command to a tethered Arc. The Arc authors the corresponding `arc.*` op on the
/// tether GroupObject (requires the Admin role on that tether); the target folds and executes it.
/// Wire form: `{"tether":"<group_id>","verb":"setServing","args":{"serving":true}}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleCommand {
    /// `group_id` of the target tether.
    pub tether: String,
    #[serde(flatten)]
    pub command: Command,
}

/// The command vocabulary. Closed here (the backend owns the verbs); the UI mirror is a
/// discriminated union on `verb`. Adding a verb is additive for readers. STARTER SET — the
/// serving/model/partition verbs map the old central control-plane actions onto the tether;
/// the UI team's consumer wish list will extend this.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "verb", content = "args", rename_all = "camelCase")]
pub enum Command {
    /// Bring the peer's model serving up/down.
    SetServing { serving: bool },
    /// Switch the peer's served model.
    SetModel { model: String },
    /// Allocate a compute partition of `units` on the peer.
    Partition { units: u32 },
}

/// Acknowledges that a command was AUTHORED + sent on the tether. Execution is async (the peer
/// folds the op on its next sync), so `ok` means "accepted + sequenced", not "already applied".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandAck {
    pub ok: bool,
    pub tether: String,
    pub verb: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

// ── lifecycle: POST /tether, POST /console/role ───────────────────────────────

/// Form an Arc↔Arc tether: this Arc fetches `<peer_url>/bundle` and pair-scans it into a
/// 2-member GroupObject, sealing the Welcome to the peer's intro mailbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TetherRequest {
    pub peer_url: String,
}

/// Assign a role to a member of a tether (`group.setMemberRole`; owner-authored).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleRequest {
    /// `group_id` of the tether. Optional for a USER role: a member has exactly one
    /// member-tether with this Arc, so the server derives it from `member` and cannot be
    /// tricked into authoring onto a tether the member isn't actually on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tether: Option<String>,
    /// `identity_key` of the member whose role changes.
    pub member: String,
    pub role: Role,
}

// ── projection: fold this Arc's directory into ConsoleState ───────────────────

/// Fold THIS Arc's directory into the console read model (`GET /console/state`). Pure read —
/// no relay, no MLS load; everything derives from the local directory projection (fold, don't
/// mirror). `arc-tether` groups become `peers`; `connection` groups become `members`.
pub fn project(node: &Node) -> Result<ConsoleState, CoreError> {
    let me = pacific_core::identity::parse_identity_key(&node.identity_key())?;
    let me_hex = hex::encode(me);
    let space_of = |pk: &[u8; 32]| format!("{}{}", pacific_core::identity::IDENTITY_KEY_PREFIX, hex::encode(pk));
    let fp_of =
        |pk: &[u8; 32]| pk[..8].iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":");

    let mut peers = Vec::new();
    let mut members = Vec::new();
    for (gid, kind) in node.all_groups()? {
        // The peer is the roster member who isn't us. A solo group (created but not yet
        // folded by the other side, or one-sided) has no peer — skip it.
        let Some(peer_pk) = node.group_roster(&gid)?.into_iter().find(|pk| *pk != me) else {
            continue;
        };
        if kind != CONNECTION_KIND {
            continue;
        }
        // ONE KIND, AND THE ROLE SAYS WHICH IT IS. `arc-tether` and
        // `member-tether` were separate kinds whose only difference was what the
        // other party is to us — which is a standing, not an object type. A peer
        // Arc carries `Role::Peer` on the connection; everyone else is a member,
        // and an un-roled member (signup delta not yet folded) reads as the
        // signup default rather than as denial.
        let standing = node.member_role(&gid, &peer_pk)?.map(Role::from_core);
        if standing == Some(Role::Peer) {
            let owner = node.group_owner_hex(&gid)?;
            let i_own = owner.as_deref() == Some(me_hex.as_str());
            let peer_owns = owner.as_deref() == Some(hex::encode(peer_pk).as_str());
            peers.push(TetherView {
                group_id: gid,
                peer: ArcNode {
                    identity_key: space_of(&peer_pk),
                    name: node.peer_name(&peer_pk)?,
                    fingerprint: fp_of(&peer_pk),
                    telemetry: None, // arrives via the arc.telemetry op (next increment)
                },
                role_i_hold: if i_own { Role::Owner } else { Role::Member },
                role_peer_holds: if peer_owns { Role::Owner } else { Role::Member },
                // No last-seen until telemetry deltas flow — honest, not fabricated.
                link: LinkHealth { last_seen_ms: None, healthy: false },
            });
        } else {
            members.push(MemberView {
                identity_key: space_of(&peer_pk),
                name: node.peer_name(&peer_pk)?,
                connection_id: gid,
                role: standing.unwrap_or(Role::DEFAULT),
            })
        }
    }

    Ok(ConsoleState {
        version: CONSOLE_API_VERSION.to_string(),
        generated_at_ms: crate::now_millis(),
        arc: ArcNode {
            identity_key: node.identity_key(),
            name: node.display_name()?,
            fingerprint: node.fingerprint(),
            telemetry: None, // self-probe / local arc.telemetry — next increment
        },
        peers,
        members,
    })
}

// ── role assignment (authored ONTO the tether) ────────────────────────────────

/// The `member-tether` this Arc holds with `member_pk` — the object a role is authored onto.
/// `None` when they are not a member here (so a role can never be aimed at a stranger).
pub fn member_tether_of(node: &Node, member_pk: &[u8; 32]) -> Result<Option<String>, CoreError> {
    let me = pacific_core::identity::parse_identity_key(&node.identity_key())?;
    for (gid, kind) in node.all_groups()? {
        if kind != MEMBER_TETHER_KIND {
            continue;
        }
        if node.group_roster(&gid)?.into_iter().any(|pk| pk != me && pk == *member_pk) {
            return Ok(Some(gid));
        }
    }
    Ok(None)
}

/// The `(op_id, args)` for `group.setMemberRole` — the ONE way a role is assigned.
///
/// Authoring this delta IS the transmission: it is sequenced onto the tether's log and sealed to
/// the member over MLS, so the Arc and the device converge on the same role from the same source.
/// The reducer refuses a role aimed at a non-roster member (`PreconditionFailed`), so this cannot
/// be used to admit anyone — it only layers standing onto an existing member.
pub fn set_member_role_op(member_space_id: &str, role: Role) -> (u32, pacific_core::coordinator::Args) {
    use pacific_core::coordinator::ArgVal;
    let mut args = pacific_core::coordinator::Args::new();
    // `member`, not `space`: the arg names what it holds now. `roles::reduce_roles`
    // takes either the `space1<hex>` form this passes or bare hex.
    args.insert("member".into(), ArgVal::Text(member_space_id.to_string()));
    args.insert("role".into(), ArgVal::Text(role.as_core_str().to_string()));
    (pacific_core::roles::OP_SET_ROLE, args)
}

// ── membership verification (the gate the gateway calls) ──────────────────────

/// The freshness window for a membership credential — bounds replay of a captured signature.
const MEMBER_CRED_WINDOW_MS: i64 = 5 * 60 * 1000;

/// Verify a membership credential (ICD §10). Membership IS the {user, Arc} MLS tether — the
/// same pair-scan key exchange as user↔user — so a member proves it by signing with the SAME
/// identity key that established the tether. The credential is
/// `<identity_key>:<ts_ms>:<sig_hex>`, where `sig` = Ed25519 over
/// `pacific-arc-member:v1\n<arc_identity_key>\n<identity_key>\n<ts_ms>`. A caller is a member iff the
/// signature verifies against `identity_key`'s key, it is fresh, AND `identity_key` is on the roster of
/// a `member-tether` on this Arc. An optional scheme prefix (`Bearer …`) is tolerated.
///
/// Returns the member's ROLE (`Some`) so the gateway can gate by capability rather than merely
/// by identity, or `None` for any malformed, stale, unsigned, or non-member credential — never
/// an error. `None` is indistinguishable to the caller from "not a member", by design.
pub fn verify_member(node: &Node, credential: &str) -> Result<Option<Role>, CoreError> {
    // Every rejection below returns the SAME `Ok(None)` — the gateway maps all of them to one
    // 401, and a device sees only "not a member of this Arc". That opacity is right for the wire
    // (never tell an attacker which check they failed) but blinding for operating the Arc: stale
    // clock, wrong signature, and a genuine non-member are three different problems with three
    // different fixes. So each reason is logged HERE, server-side, where it is safe to be exact.
    // The signature is never logged — only the member's identity_key (public) and the timestamp.
    let token = credential.trim().rsplit(' ').next().unwrap_or("").trim();
    let mut parts = token.splitn(3, ':');
    let (Some(identity_key), Some(ts_s), Some(sig_hex)) = (parts.next(), parts.next(), parts.next())
    else {
        eprintln!("[verify] reject: malformed credential (not <identity_key>:<ts>:<sig>)");
        return Ok(None);
    };
    let Ok(ts) = ts_s.parse::<i64>() else {
        eprintln!("[verify] reject: timestamp {ts_s:?} is not an integer");
        return Ok(None);
    };
    let Ok(sig_vec) = hex::decode(sig_hex) else {
        eprintln!("[verify] reject: signature is not hex (member {identity_key})");
        return Ok(None);
    };
    let Ok(sig) = <[u8; 64]>::try_from(sig_vec.as_slice()) else {
        eprintln!("[verify] reject: signature is {} bytes, want 64 (member {identity_key})", sig_vec.len());
        return Ok(None);
    };
    let Ok(member_pk) = pacific_core::identity::parse_identity_key(identity_key) else {
        eprintln!("[verify] reject: identity_key {identity_key:?} does not parse");
        return Ok(None);
    };

    // Fresh?
    let skew = crate::now_millis() as i64 - ts;
    if skew.abs() > MEMBER_CRED_WINDOW_MS {
        eprintln!("[verify] reject: stale/skewed by {skew}ms (window ±{MEMBER_CRED_WINDOW_MS}ms, member {identity_key}) — check device clock");
        return Ok(None);
    }
    // Signed by that identity, bound to THIS Arc?
    // BARE HEX on both sides of the payload — the credential token is
    // colon-delimited (`<id>:<ts>:<sig>`) and the `ed25519:` rendering carries a
    // colon of its own, so machine formats take the hex and the rendering stays
    // for display (12 Aug rename). Must match pacific-ffi::arc_member_credential.
    let arc_identity_key = hex::encode(node.id.identity_pk());
    let payload = format!("pacific-arc-member:v1\n{arc_identity_key}\n{identity_key}\n{ts}");
    if pacific_core::identity::verify_sig(&member_pk, payload.as_bytes(), &sig).is_err() {
        eprintln!("[verify] reject: bad signature (member {identity_key}, arc {arc_identity_key}) — key mismatch or payload drift");
        return Ok(None);
    }
    // A member here? Membership is the MLS roster of a `member-tether` with this Arc — the
    // signature proves the KEY, the roster proves the MEMBERSHIP, and only then does the folded
    // role say what they may DO. A role alone never admits: it is read from a tether we already
    // confirmed they are on (`setMemberRole` layers onto a real roster member).
    let me = pacific_core::identity::parse_identity_key(&arc_identity_key)?;
    let mut tethers = 0usize;
    // These reads are annotated so a failure names WHICH one threw — the handler logs the
    // resulting error, and "all_groups" vs "group_roster(<gid>)" is the difference between a
    // corrupt directory and one unreadable tether.
    let groups = node.all_groups().map_err(|e| {
        CoreError::Identity(format!("verify_member: all_groups failed: {e}"))
    })?;
    for (gid, kind) in groups {
        if kind != MEMBER_TETHER_KIND {
            continue;
        }
        tethers += 1;
        let roster = node.group_roster(&gid).map_err(|e| {
            CoreError::Identity(format!("verify_member: group_roster({gid}) failed: {e}"))
        })?;
        if roster.into_iter().any(|pk| pk != me && pk == member_pk) {
            // On the roster. An un-roled member (their setMemberRole delta not folded yet)
            // gets the signup default rather than a denial — honest, and never MORE than
            // what an explicit role would grant.
            let role = node.member_role(&gid, &member_pk)?.map(Role::from_core).unwrap_or(Role::DEFAULT);
            eprintln!("[verify] ALLOW: member {identity_key} on roster (role {role:?})");
            return Ok(Some(role));
        }
    }
    // Signature was VALID but the key is on no member-tether roster: the credential is
    // authentic, the membership is not. This is the case that looks identical to a bad
    // signature from outside, and the one most worth naming — it means signup never
    // completed its tether, or it landed on a different Arc.
    eprintln!("[verify] reject: valid signature but member {identity_key} is on none of this Arc's {tethers} member-tether(s) — signup incomplete or wrong Arc");
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the exact JSON the frontend contract (`contract.ts`) is written against. If this
    /// changes, the TS mirror and `CONSOLE_API_VERSION` must change with it.
    #[test]
    fn contract_json_is_stable() {
        let state = ConsoleState {
            version: CONSOLE_API_VERSION.into(),
            generated_at_ms: 1_700_000_000_000,
            arc: ArcNode {
                identity_key: "space1aa3c409c7cfc379abf969f0fb1ed0dbcffae734fc41c8c076851b1f94b4e0d18".into(),
                name: "Alameda".into(),
                fingerprint: "aa:3c:40:9c:7c:fc:37:9a".into(),
                telemetry: Some(Telemetry {
                    model_live: true,
                    in_flight: Some(2),
                    gpu_util_pct: Some(41.0),
                    gpu_mem_used_mb: Some(12000.0),
                    gpu_mem_total_mb: Some(80000.0),
                }),
            },
            peers: vec![TetherView {
                group_id: "ab12cd34".into(),
                peer: ArcNode {
                    identity_key: "space1bb00000000000000000000000000000000000000000000000000000000000".into(),
                    name: "Oakland".into(),
                    fingerprint: "bb:00:00:00:00:00:00:00".into(),
                    telemetry: None,
                },
                role_i_hold: Role::Admin,
                role_peer_holds: Role::Member,
                link: LinkHealth { last_seen_ms: Some(1_700_000_000_000), healthy: true },
            }],
            members: vec![MemberView {
                identity_key: "space1cc00000000000000000000000000000000000000000000000000000000000".into(),
                name: "Ada".into(),
                connection_id: "cd34ab12".into(),
                role: Role::Member,
            }],
        };
        let state_json = serde_json::to_string_pretty(&state).unwrap();
        assert!(state_json.contains("\"version\": \"console/v1\""));
        assert!(state_json.contains("\"role_i_hold\": \"admin\""));
        assert!(state_json.contains("\"role_peer_holds\": \"member\""));
        assert!(state_json.contains("\"model_live\": true"));
        // peer telemetry is None -> the key is omitted, never null.
        assert!(!state_json.contains("\"telemetry\": null"));

        // The command envelope: flattened tether + adjacently-tagged verb/args, and it round-trips.
        let cmd = ConsoleCommand { tether: "ab12cd34".into(), command: Command::SetServing { serving: true } };
        let cmd_json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(cmd_json, r#"{"tether":"ab12cd34","verb":"setServing","args":{"serving":true}}"#);
        let back: ConsoleCommand = serde_json::from_str(&cmd_json).unwrap();
        assert_eq!(back.command, Command::SetServing { serving: true });
        assert_eq!(back.tether, "ab12cd34");

        // Emit the canonical samples for the cross-language check when asked.
        if let Ok(dir) = std::env::var("CONTRACT_OUT") {
            std::fs::write(format!("{dir}/state.json"), &state_json).unwrap();
            std::fs::write(format!("{dir}/command.json"), &cmd_json).unwrap();
        }
    }
}
