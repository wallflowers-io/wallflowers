//! THE EXTERIOR ON A NODE (#76, ICD 2.6.0): writing it, carrying it, knocking at its door and
//! answering. The facet, the seal and the leg's fold are `crate::exterior` and `crate::door`;
//! this is where they meet the relay, the connections and the MLS doors.
//!
//! - an owner writes the dial, the door and the exterior in one Save (`set_outside`): only
//!   what changed, committed locally and sent once;
//! - every member carries each object's exterior to its connections, and every receiver carries
//!   a `network` one on (`carry_exteriors`): gated by what the target channel's own fold already
//!   holds, so a settled mesh authors nothing, and at most `CARRIES_PER_SYNC` a pass;
//! - a non-member asks (`request_join`), sealed to the door, and records the ask on their own
//!   record; or withdraws it;
//! - every member's sync drains each door it holds from its cursor into the knock store
//!   (`drain_doors`); an `auto` door admits from there, a `review` door waits for the owner or
//!   an admitter (`door_requests`, `decide_request`).

use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::door::{self, Knock};
use crate::exterior::{self as ext, Carried, DoorMode, ExteriorState};
use crate::object_store::GroupObjectStore;
use crate::visibility::Visibility;

/// Carries at most this many copies a sync; the rest go on the next pass (the ICD's
/// `contact.publishExterior.perSync`).
const CARRIES_PER_SYNC: usize = ext::icd::CARRIES_PER_SYNC;

/// A failed admission is tried again after a minute, doubling, at most a day apart; past this
/// many it is settled as `unadmittable` on this device.
const ADMIT_ATTEMPTS: u32 = 8;

/// One request waiting at a door, as an admitter's device reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DoorRequest {
    pub object: String,
    pub kind: String,
    /// The request's id, hex (`mls.door.id`).
    pub request: String,
    pub asker: [u8; 32],
    /// The name on the asker's own bundles.
    pub asker_name: String,
    pub note: String,
    pub at: u64,
    bundles: Vec<String>,
    door: [u8; 32],
    id: [u8; 32],
    attempts: u32,
    next_at: i64,
}

/// An exterior that reached me, and whether I have asked to join.
#[derive(Clone, Debug)]
pub struct HeldExterior {
    pub carried: Carried,
    pub requested: bool,
}

/// What an owner's Save says of the outside: the exterior and its door.
#[derive(Clone, Debug)]
pub struct ExteriorDraft {
    pub name: String,
    pub description: String,
    pub photo: Option<pacific_media::MediaRef>,
    /// `None` closes the door.
    pub door: Option<DoorMode>,
}

impl Node {
    /// A held object's kind, exterior facet and visibility; `None` for a kind with no exterior.
    pub fn exterior_state(&self, object_hex: &str) -> Result<Option<(ObjectKind, ExteriorState, Visibility)>, CoreError> {
        let gid = self.object_group_id(object_hex)?;
        let Some(kind) = self.dir.group_kind(&gid)?.as_deref().and_then(crate::fold::lens_for) else {
            return Ok(None);
        };
        if ext::exterior_kind(kind.name()).is_none() {
            return Ok(None);
        }
        Ok(GroupObjectStore::new(&self.dir).exterior_of(&gid, kind)?.map(|(e, v)| (kind, e, v)))
    }

    /// ONE SAVE OF THE OUTSIDE, by its owner: the dial, the door and the exterior. Only what
    /// differs from the fold is written, each a local commit, and the object is sent once, so a
    /// Save that changes nothing authors nothing and moves no copy. A door already open keeps
    /// its key, so the requests sealed to it still open. Returns how many ops were written.
    pub async fn set_outside(&self, object_hex: &str, visibility: Option<Visibility>, exterior: Option<&ExteriorDraft>) -> Result<usize, CoreError> {
        let (_, state, now) = self
            .exterior_state(object_hex)?
            .ok_or_else(|| CoreError::Coordinator("this kind has no exterior".into()))?;
        let mut wrote = 0;
        if let Some(v) = visibility.filter(|v| *v != now) {
            self.apply_local(object_hex, crate::visibility::OP_SET_VISIBILITY, crate::visibility::set_visibility_args(v)).await?;
            wrote += 1;
        }
        if let Some(d) = exterior {
            let key = d.door.map(|_| state.door.as_ref().map(|k| k.key).unwrap_or_else(|| door::mint().0));
            let door_now = state.door.as_ref().map(|k| (k.key, k.mode));
            let door_then = key.zip(d.door);
            if door_now != door_then {
                let args = ext::set_door_args(key.as_ref(), d.door.unwrap_or(DoorMode::Review));
                self.apply_local(object_hex, ext::OP_SET_DOOR, args).await?;
                wrote += 1;
            }
            let args = ext::set_exterior_args(&d.name, &d.description, d.photo.as_ref(), key.map(|k| door::public_of(&k)).as_ref());
            let then = ext::parse(&args).map_err(|e| CoreError::Coordinator(format!("the exterior: {e:?}")))?;
            if state.exterior.as_ref() != Some(&then) {
                self.apply_local(object_hex, ext::OP_SET_EXTERIOR, args).await?;
                wrote += 1;
            }
        }
        if wrote > 0 {
            self.publish_pending(object_hex).await?;
        }
        Ok(wrote)
    }

    /// The exterior and its door alone.
    pub async fn set_exterior(&self, object_hex: &str, draft: &ExteriorDraft) -> Result<usize, CoreError> {
        self.set_outside(object_hex, None, Some(draft)).await
    }

    /// The dial alone: how far an object's existence, and its exterior, may travel.
    pub async fn set_visibility(&self, object_hex: &str, v: Visibility) -> Result<usize, CoreError> {
        self.set_outside(object_hex, Some(v), None).await
    }

    /// The owner's latest signed row of `op_id` on an object, as `mls.applicationPayload`.
    fn owner_row(&self, gid: &[u8], kind: ObjectKind, owner: &[u8; 32], op_id: u32) -> Result<Option<Vec<u8>>, CoreError> {
        let mut best: Option<((u64, u64), Vec<u8>)> = None;
        for (author, envelope, sig) in self.dir.load_log_signed(gid)? {
            let (Some(sig), true) = (sig, author == *owner) else { continue };
            let Ok(d) = coordinator::decode_delta(&envelope) else { continue };
            let Some(seq) = d.seq else { continue };
            if d.type_id != kind.type_id() as u32 || d.op_id != op_id {
                continue;
            }
            if best.as_ref().is_none_or(|(at, _)| (d.epoch, seq) > *at) {
                best = Some(((d.epoch, seq), crate::delta_sig::seal_payload(&envelope, &author, &sig)));
            }
        }
        Ok(best.map(|(_, p)| p))
    }

    fn connection_channels(&self) -> Result<Vec<Vec<u8>>, CoreError> {
        Ok(self.dir.all_groups()?.into_iter().filter(|(_, k)| k == "connection").map(|(g, _)| g).collect())
    }

    /// Carry one copy into one channel, unless the channel already holds it as well. The gate is
    /// the fold's own rule, so what is sent is what the receiver keeps: a copy within reach
    /// that supersedes what is held, or a narrower visibility that withdraws a live copy.
    async fn carry_exterior(&self, channel: &[u8], args: &coordinator::Args, mine: &Carried) -> Result<bool, CoreError> {
        let held = GroupObjectStore::new(&self.dir)
            .folded::<crate::contact::ContactType>(channel)?
            .state()
            .exteriors
            .get(&mine.object)
            .cloned();
        let send = if mine.within_reach() {
            held.as_ref().is_none_or(|h| mine.supersedes(h))
        } else {
            held.as_ref().is_some_and(|h| !h.withdrawn && mine.visibility_pos > h.visibility_pos)
        };
        if !send {
            return Ok(false);
        }
        let mut args = args.clone();
        let gen = self.dir.author_delta_count(channel, &self.id.identity_pk())?;
        args.insert("gen".into(), coordinator::ArgVal::Int(gen as i64));
        self.contact_author_into(channel, crate::contact::OP_PUBLISH_EXTERIOR, args).await?;
        Ok(true)
    }

    /// THE CARRY, once a sync: every member carries each of its objects' exteriors to its
    /// connections outside them; every receiver carries a `network` one, and each withdrawal,
    /// one hop on — never back to whoever brought it, never to its owner. At most
    /// `CARRIES_PER_SYNC` copies a pass, its own objects first; the rest go on the next.
    pub async fn carry_exteriors(&self) -> Result<usize, CoreError> {
        let me = self.id.identity_pk();
        let connections = self.connection_channels()?;
        if connections.is_empty() {
            return Ok(0);
        }
        let mut sent = 0;
        // MINE: one hop, to every connection that is not inside.
        for (gid, kind) in self.dir.list_objects()? {
            let Some(kind) = crate::fold::lens_for(&kind).filter(|k| ext::exterior_kind(k.name()).is_some()) else { continue };
            let Some(owner) = self.dir.group_owner(&gid)?.and_then(|o| <[u8; 32]>::try_from(o).ok()) else { continue };
            let Some(ext_row) = self.owner_row(&gid, kind, &owner, ext::OP_SET_EXTERIOR)? else { continue };
            let vis_row = self.owner_row(&gid, kind, &owner, crate::visibility::OP_SET_VISIBILITY)?;
            let object = hex::encode(&gid);
            let args = ext::publish_exterior_args(&object, kind.name(), &owner, &ext_row, vis_row.as_deref(), 1);
            let mine = match ext::carried(&with_gen(&args), &me) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(object = %object, error = ?e, "an exterior that would not carry; not sent");
                    continue;
                }
            };
            let inside = self.dir.group_members(&gid)?;
            for channel in &connections {
                if sent >= CARRIES_PER_SYNC {
                    return Ok(sent);
                }
                // A connection already inside the object holds the interior.
                if self.dir.group_members(channel)?.iter().filter(|p| **p != me).all(|p| inside.contains(p)) {
                    continue;
                }
                match self.carry_exterior(channel, &args, &mine).await {
                    Ok(true) => sent += 1,
                    Ok(false) => {}
                    Err(e) => tracing::warn!(object = %object, error = %e, "exterior not carried to one connection; continuing"),
                }
            }
        }
        // CARRIED ON: a `network` copy, or a withdrawal, one hop further.
        for c in GroupObjectStore::new(&self.dir).merged_exteriors()?.into_values() {
            if c.visibility != Visibility::Network && !c.withdrawn {
                continue;
            }
            let args = ext::publish_exterior_args(&c.object, c.kind.name(), &c.owner, &c.exterior_row, c.visibility_row.as_deref(), c.hops + 1);
            let Ok(next) = ext::carried(&with_gen(&args), &me) else { continue };
            for channel in &connections {
                if sent >= CARRIES_PER_SYNC {
                    return Ok(sent);
                }
                let peers = self.dir.group_members(channel)?;
                if peers.contains(&c.via) || peers.contains(&c.owner) {
                    continue;
                }
                match self.carry_exterior(channel, &args, &next).await {
                    Ok(true) => sent += 1,
                    Ok(false) => {}
                    Err(e) => tracing::warn!(object = %c.object, error = %e, "exterior not relayed to one connection; continuing"),
                }
            }
        }
        Ok(sent)
    }

    /// The asks on my own record (`group.setRequest`), latest per object.
    pub fn asked(&self) -> Result<BTreeMap<String, crate::group::Asked>, CoreError> {
        let me = hex::decode(self.ensure_self_object()?).map_err(|e| CoreError::Directory(format!("self object id hex: {e}")))?;
        Ok(self.folded_group(&me)?.state().asked.clone())
    }

    /// THE EXTERIORS THAT REACHED ME, live ones only, each with whether I have asked to join.
    pub fn exteriors(&self) -> Result<Vec<HeldExterior>, CoreError> {
        let asks = self.asked()?;
        Ok(GroupObjectStore::new(&self.dir)
            .merged_exteriors()?
            .into_values()
            .filter(|c| !c.withdrawn)
            .map(|c| HeldExterior { requested: asks.get(&c.object).is_some_and(|a| !a.withdrawn), carried: c })
            .collect())
    }

    async fn knock(&self, door_pub: &[u8; 32], knock: &Knock) -> Result<[u8; 32], CoreError> {
        let blob = door::seal_to(door_pub, &door::encode(&self.id, knock));
        let published = pacific_wire::blob_b64(&blob);
        if published.len() as u64 > ext::DOOR_MAX_BYTES {
            return Err(CoreError::Coordinator(format!("a knock of {} bytes is over the door's {}", published.len(), ext::DOOR_MAX_BYTES)));
        }
        let id = door::id_of(&blob);
        let mut sess = self.home_session().await?;
        // At the door's tag: the relay holds it to the door's bound, and members drain it there.
        let out = sess.publish_door(&Address::from_seed(&door::address_seed(door_pub)), &published).await;
        self.keep_session(sess);
        out.map(|_| id)
    }

    /// ASK TO JOIN an object whose exterior reached me: sealed to its door, so only its members
    /// read it, and recorded on my own record, so the ask survives a new device. Returns the
    /// request's id, hex.
    pub async fn request_join(&self, object_hex: &str, note: &str) -> Result<String, CoreError> {
        let c = GroupObjectStore::new(&self.dir)
            .merged_exteriors()?
            .remove(object_hex)
            .filter(|c| !c.withdrawn)
            .ok_or_else(|| CoreError::Coordinator("no exterior of this object has reached you".into()))?;
        let door_pub = c.exterior.door.ok_or_else(|| CoreError::Coordinator("this object has no door".into()))?;
        let object = hex::decode(object_hex).map_err(|e| CoreError::Directory(format!("object id hex: {e}")))?;
        let bundles = (0..ext::icd::DOOR_BUNDLES).map(|_| self.build_contact_bundle()).collect::<Result<Vec<_>, _>>()?;
        let request = door::Request { object: object.clone(), asker: self.id.identity_pk(), bundles, note: note.to_string(), at: unix_now_ms() };
        if let Some(why) = request.refusal() {
            return Err(CoreError::Coordinator(why));
        }
        // The Welcome that answers must name the owner the exterior did (§8.5's card check).
        self.dir.set_expected_owner(&object, &c.owner)?;
        let id = self.knock(&door_pub, &Knock::Request(request)).await?;
        let args = crate::group::set_request_args(object_hex, c.kind.name(), &c.owner, &id, unix_now(), false);
        self.apply(&self.ensure_self_object()?, crate::group::OP_SET_REQUEST, args).await?;
        Ok(hex::encode(id))
    }

    /// Withdraw my ask: told to the door while its exterior still reaches me, and recorded on my
    /// own record either way.
    pub async fn withdraw_request(&self, object_hex: &str) -> Result<(), CoreError> {
        let ask = self
            .asked()?
            .remove(object_hex)
            .filter(|a| !a.withdrawn)
            .ok_or_else(|| CoreError::Coordinator("you have not asked to join this object".into()))?;
        let door_pub = GroupObjectStore::new(&self.dir).merged_exteriors()?.remove(object_hex).and_then(|c| c.exterior.door);
        if let Some(door_pub) = door_pub {
            let object = hex::decode(object_hex).map_err(|e| CoreError::Directory(format!("object id hex: {e}")))?;
            let w = door::Withdraw { object, asker: self.id.identity_pk(), request: ask.request, at: unix_now_ms() };
            self.knock(&door_pub, &Knock::Withdraw(w)).await?;
        }
        let args = crate::group::set_request_args(object_hex, &ask.kind, &ask.owner, &ask.request, unix_now(), true);
        self.apply(&self.ensure_self_object()?, crate::group::OP_SET_REQUEST, args).await?;
        Ok(())
    }

    /// The doors this device holds: (object, door's private key, mode).
    fn held_doors(&self) -> Result<Vec<(String, [u8; 32], DoorMode)>, CoreError> {
        let mut out = Vec::new();
        for (gid, _) in self.dir.list_objects()? {
            let object = hex::encode(&gid);
            if let Ok(Some((_, state, _))) = self.exterior_state(&object) {
                if let Some(d) = state.door {
                    out.push((object, d.key, d.mode));
                }
            }
        }
        Ok(out)
    }

    /// DRAIN EVERY DOOR I HOLD, from its cursor, into the knock store: what is new since the
    /// last pass, on one held session. Every sync, review doors and auto alike, so a knock is
    /// kept here before the relay can let it go. Returns how many knocks were new.
    pub async fn drain_doors(&self) -> Result<usize, CoreError> {
        let doors = self.held_doors()?;
        if doors.is_empty() {
            return Ok(0);
        }
        let seeds: Vec<[u8; 32]> = doors.iter().map(|(_, key, _)| door::address_seed(&door::public_of(key))).collect();
        let tags: Vec<String> = seeds.iter().map(|s| Address::from_seed(s).door_tag()).collect();
        let dir = &self.dir;
        let mut sess = self.home_session().await?;
        let drained = sess
            .drain_doors(&tags, |tag, source| {
                let i = tags.iter().position(|t| t == tag).expect("a tag asked for");
                dir.cursor(&seeds[i], source)
            })
            .await;
        self.keep_session(sess);
        let (drained, lost) = drained?;
        // A FLOOD, SEEN: what each door has lost to its cap since the total last seen, named
        // once when it rises; rotating the door is the owner's remedy.
        for (tag, total) in &lost {
            let Some(i) = tags.iter().position(|t| t == tag) else { continue };
            let rose = self.dir.evicted_rose(&seeds[i], *total)?;
            if rose > 0 {
                tracing::warn!(door = %tag, evicted = rose, total, "a door's cap has evicted knocks");
            }
        }
        let (now, ttl) = (unix_now(), ext::DOOR_TTL_DAYS as i64 * 86_400);
        let mut new = 0;
        for (seed, msgs) in seeds.iter().zip(drained) {
            for m in msgs {
                match pacific_wire::blob_unb64(&m.blob) {
                    Ok(blob) if m.blob.len() as u64 <= ext::DOOR_MAX_BYTES => {
                        self.dir.put_knock(seed, &door::id_of(&blob), &blob, now, ext::DOOR_CAP as usize, ttl)?;
                        new += 1;
                    }
                    Ok(_) => tracing::debug!(bytes = m.blob.len(), "a door blob over the bound; set aside"),
                    Err(e) => tracing::debug!(error = %e, "a door blob that is not base64; set aside"),
                }
                self.dir.advance_cursor(seed, m.source, m.seq)?;
            }
        }
        Ok(new)
    }

    /// The requests waiting at one held door, read from the knock store: opened with its key,
    /// signed by their askers, every bundle the asker's; the latest per asker. A knock that is
    /// decided, withdrawn, already inside, superseded or unusable is settled with why, and
    /// not read again.
    fn waiting_at(&self, object_hex: &str, kind: ObjectKind, state: &ExteriorState) -> Result<Vec<DoorRequest>, CoreError> {
        let Some(d) = state.door.as_ref() else { return Ok(Vec::new()) };
        let seed = door::address_seed(&door::public_of(&d.key));
        let gid = self.object_group_id(object_hex)?;
        let inside = self.dir.group_members(&gid)?;
        let mut asked: Vec<DoorRequest> = Vec::new();
        let mut withdrawn: BTreeSet<([u8; 32], [u8; 32])> = BTreeSet::new();
        for (id, blob, attempts, next_at) in self.dir.waiting_knocks(&seed)? {
            let opened = door::open_with(&d.key, &blob).and_then(|p| door::decode(&p));
            match opened {
                Ok(Knock::Request(r)) if r.object == gid => match self.askers_bundles(&r) {
                    Ok(name) => asked.push(DoorRequest {
                        object: object_hex.into(),
                        kind: kind.name().into(),
                        request: hex::encode(id),
                        asker: r.asker,
                        asker_name: name,
                        note: r.note,
                        at: r.at,
                        bundles: r.bundles,
                        door: seed,
                        id,
                        attempts,
                        next_at,
                    }),
                    Err(_) => self.dir.settle_knock(&seed, &id, "unusable")?,
                },
                Ok(Knock::Withdraw(w)) if w.object == gid => {
                    withdrawn.insert((w.request, w.asker));
                    self.dir.settle_knock(&seed, &id, "withdraw")?;
                }
                _ => self.dir.settle_knock(&seed, &id, "unusable")?,
            }
        }
        let mut latest: BTreeMap<[u8; 32], DoorRequest> = BTreeMap::new();
        for r in asked {
            let why = if state.decided.contains_key(&r.id) {
                Some("decided")
            } else if inside.contains(&r.asker) {
                Some("inside")
            } else if withdrawn.contains(&(r.id, r.asker)) {
                Some("withdrawn")
            } else {
                None
            };
            if let Some(why) = why {
                self.dir.settle_knock(&seed, &r.id, why)?;
                continue;
            }
            match latest.get(&r.asker) {
                Some(held) if held.at >= r.at => self.dir.settle_knock(&seed, &r.id, "superseded")?,
                Some(held) => {
                    self.dir.settle_knock(&seed, &held.id, "superseded")?;
                    latest.insert(r.asker, r);
                }
                None => {
                    latest.insert(r.asker, r);
                }
            }
        }
        Ok(latest.into_values().collect())
    }

    /// Every bundle verifies and is the asker's; the name they carry.
    fn askers_bundles(&self, r: &door::Request) -> Result<String, CoreError> {
        let mut name = String::new();
        for b in &r.bundles {
            let parsed = handshake::parse_and_verify(b)?;
            if parsed.identity_pk != r.asker {
                return Err(CoreError::Seal("a bundle that is not the asker's".into()));
            }
            name = parsed.display_name;
        }
        Ok(name)
    }

    /// The requests waiting at one held object's door, from what this device has drained.
    pub fn door_requests(&self, object_hex: &str) -> Result<Vec<DoorRequest>, CoreError> {
        let Some((kind, state, _)) = self.exterior_state(object_hex)? else { return Ok(Vec::new()) };
        self.waiting_at(object_hex, kind, &state)
    }

    /// Every request waiting for ME: at the `review` doors of objects I own or admit to.
    pub fn pending_requests(&self) -> Result<Vec<DoorRequest>, CoreError> {
        let me = self.id.identity_pk();
        let mut out = Vec::new();
        for (object, _, mode) in self.held_doors()? {
            let gid = self.object_group_id(&object)?;
            let mine = self.dir.group_owner(&gid)?.is_some_and(|o| o == me) || self.is_admitter(&object, &me);
            if mode == DoorMode::Review && mine {
                out.append(&mut self.door_requests(&object)?);
            }
        }
        Ok(out)
    }

    /// Add the asker by one of their bundles, unless they are already inside: another member may
    /// have let them in, and a second Add would graft a second leaf for one knock.
    async fn admit_request(&self, r: &DoorRequest) -> Result<(), CoreError> {
        let gid = self.object_group_id(&r.object)?;
        let mut last = CoreError::Coordinator("a request with no bundle".into());
        for b in &r.bundles {
            if self.dir.group_members(&gid)?.contains(&r.asker) {
                return Ok(());
            }
            match self.group_add_member(&r.object, b).await {
                Ok(_) => return Ok(()),
                Err(e) => last = e,
            }
        }
        if self.dir.group_members(&gid)?.contains(&r.asker) {
            return Ok(());
        }
        Err(last)
    }

    /// The owner's or an admitter's answer to one request. Admitting adds the asker by one of
    /// their bundles, then records the decision; declining records it, and the asker is not told.
    pub async fn decide_request(&self, object_hex: &str, request_hex: &str, admit: bool) -> Result<(), CoreError> {
        let r = self
            .door_requests(object_hex)?
            .into_iter()
            .find(|r| r.request == request_hex)
            .ok_or_else(|| CoreError::Coordinator("no such request is waiting at this door".into()))?;
        if admit {
            self.admit_request(&r).await?;
        }
        self.apply(object_hex, ext::OP_DECIDE_REQUEST, ext::decide_request_args(&r.id, &r.asker, admit)).await?;
        self.dir.settle_knock(&r.door, &r.id, "decided")?;
        Ok(())
    }

    /// Answer every `auto` door I am behind, from the knock store: whoever asked is admitted.
    /// Runs on every member, as the Place's doorbell does, so a door is answered while anyone
    /// inside is about. An admission that fails is tried again later, and in the end set aside.
    pub async fn answer_exterior_doors(&self) -> Result<Vec<String>, CoreError> {
        let now = unix_now();
        let mut admitted = Vec::new();
        for (object, _, mode) in self.held_doors()? {
            if mode != DoorMode::Auto {
                continue;
            }
            for r in self.door_requests(&object)? {
                if r.next_at > now {
                    continue;
                }
                match self.admit_request(&r).await {
                    Ok(()) => admitted.push(object.clone()),
                    Err(e) => {
                        // A minute, doubling, at most a day apart.
                        let next_at = now + (60i64 << r.attempts.min(11)).min(86_400);
                        let attempts = self.dir.knock_failed(&r.door, &r.id, next_at)?;
                        tracing::debug!(object = %object, attempts, error = %e, "an auto door could not admit");
                        if attempts >= ADMIT_ATTEMPTS {
                            self.dir.settle_knock(&r.door, &r.id, "unadmittable")?;
                        }
                    }
                }
            }
        }
        Ok(admitted)
    }

    /// An `auto` door is the owner's consent that any member may add whoever knocks there.
    pub(super) fn admits_at_an_auto_door(&self, object_hex: &str) -> bool {
        matches!(self.exterior_state(object_hex), Ok(Some((_, s, _))) if s.door.as_ref().is_some_and(|d| d.mode == DoorMode::Auto))
    }
}

/// The leg's args as a receiver folds them: a gen stands in for the one the write path stamps.
fn with_gen(args: &coordinator::Args) -> coordinator::Args {
    let mut a = args.clone();
    a.insert("gen".into(), coordinator::ArgVal::Int(0));
    a
}
