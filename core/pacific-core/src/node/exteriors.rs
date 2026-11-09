//! THE EXTERIOR ON A NODE (#76, ICD 2.6.0): writing it, carrying it, knocking at its door and
//! answering. The facet, the seal and the leg's fold are `crate::exterior` and `crate::door`;
//! this is where they meet the relay, the connections and the MLS doors.
//!
//! - an owner writes the exterior and the door (`set_exterior`), and the dial
//!   (`set_visibility`);
//! - every member carries each object's exterior to its connections, and every receiver carries
//!   a `network` one on (`publish_exteriors`, `relay_exteriors`): gated by what the target
//!   channel's own fold already holds, so a settled mesh authors nothing;
//! - a non-member asks (`request_join`), sealed to the door, and records the ask on their own
//!   record; or withdraws it;
//! - a member's device opens the door: an `auto` door admits on sight (`answer_exterior_doors`),
//!   a `review` door waits for the owner or an admitter (`door_requests`, `decide_request`).

use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::door::{self, Knock};
use crate::exterior::{self as ext, Carried, DoorMode, ExteriorState};
use crate::object_store::GroupObjectStore;
use crate::visibility::Visibility;

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
}

/// An exterior that reached me, and whether I have asked to join.
#[derive(Clone, Debug)]
pub struct HeldExterior {
    pub carried: Carried,
    pub requested: bool,
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

    /// The object's exterior and door, written by its owner. `door` None closes the door; a door
    /// already open keeps its key, so requests sealed to it still open.
    pub async fn set_exterior(
        &self,
        object_hex: &str,
        name: &str,
        description: &str,
        photo: Option<&pacific_media::MediaRef>,
        door: Option<DoorMode>,
    ) -> Result<(), CoreError> {
        let (_, state, _) = self
            .exterior_state(object_hex)?
            .ok_or_else(|| CoreError::Coordinator("this kind has no exterior".into()))?;
        let public = match door {
            None => {
                if state.door.is_some() {
                    self.apply(object_hex, ext::OP_SET_DOOR, ext::set_door_args(None, DoorMode::Review)).await?;
                }
                None
            }
            Some(mode) => {
                let key = state.door.as_ref().map(|d| d.key).unwrap_or_else(|| door::mint().0);
                if state.door.as_ref().map(|d| (d.key, d.mode)) != Some((key, mode)) {
                    self.apply(object_hex, ext::OP_SET_DOOR, ext::set_door_args(Some(&key), mode)).await?;
                }
                Some(door::public_of(&key))
            }
        };
        self.apply(object_hex, ext::OP_SET_EXTERIOR, ext::set_exterior_args(name, description, photo, public.as_ref())).await?;
        Ok(())
    }

    /// How far an object's existence, and its exterior, may travel.
    pub async fn set_visibility(&self, object_hex: &str, v: Visibility) -> Result<String, CoreError> {
        self.apply(object_hex, crate::visibility::OP_SET_VISIBILITY, crate::visibility::set_visibility_args(v)).await
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

    /// Every member carries the exteriors of its objects to its connections who are not inside
    /// them. Returns how many copies were sent.
    pub async fn publish_exteriors(&self) -> Result<usize, CoreError> {
        let me = self.id.identity_pk();
        let connections = self.connection_channels()?;
        if connections.is_empty() {
            return Ok(0);
        }
        let mut sent = 0;
        for (gid, kind) in self.dir.list_objects()? {
            let Some(kind) = crate::fold::lens_for(&kind).filter(|k| ext::exterior_kind(k.name()).is_some()) else { continue };
            let Some(owner) = self.dir.group_owner(&gid)?.and_then(|o| <[u8; 32]>::try_from(o).ok()) else { continue };
            let Some(ext_row) = self.owner_row(&gid, kind, &owner, ext::OP_SET_EXTERIOR)? else { continue };
            let vis_row = self.owner_row(&gid, kind, &owner, crate::visibility::OP_SET_VISIBILITY)?;
            let object = hex::encode(&gid);
            let mut args = ext::publish_exterior_args(&object, kind.name(), &owner, &ext_row, vis_row.as_deref(), 1);
            args.insert("gen".into(), coordinator::ArgVal::Int(0));
            let mine = match ext::carried(&args, &me) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(object = %object, error = ?e, "an exterior that would not carry; not sent");
                    continue;
                }
            };
            args.remove("gen");
            let inside = self.dir.group_members(&gid)?;
            for channel in &connections {
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
        Ok(sent)
    }

    /// A receiver carries each `network` exterior on, one hop further, and each withdrawal down
    /// the paths its copy took: never back to whoever brought it, never to its owner.
    pub async fn relay_exteriors(&self) -> Result<usize, CoreError> {
        let me = self.id.identity_pk();
        let connections = self.connection_channels()?;
        let mut sent = 0;
        for c in GroupObjectStore::new(&self.dir).merged_exteriors()?.into_values() {
            if c.visibility != Visibility::Network && !c.withdrawn {
                continue;
            }
            let mut args = ext::publish_exterior_args(&c.object, c.kind.name(), &c.owner, &c.exterior_row, c.visibility_row.as_deref(), c.hops + 1);
            args.insert("gen".into(), coordinator::ArgVal::Int(0));
            let Ok(next) = ext::carried(&args, &me) else { continue };
            args.remove("gen");
            for channel in &connections {
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
        let id = door::id_of(&blob);
        let mut sess = Router::open(&self.routes).await?;
        let out = sess.publish(&Address::from_seed(&door::address_seed(door_pub)), &pacific_wire::blob_b64(&blob)).await;
        sess.close().await;
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
        let bundles = (0..crate::exterior::icd::DOOR_BUNDLES).map(|_| self.build_contact_bundle()).collect::<Result<Vec<_>, _>>()?;
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

    /// The requests waiting at one held object's door: opened with its key, signed by their
    /// askers, every bundle the asker's; less those decided, withdrawn, or already in. The door
    /// is read from the start each time: the relay is where a request lives, so a new device
    /// sees what the old one saw.
    pub async fn door_requests(&self, object_hex: &str) -> Result<Vec<DoorRequest>, CoreError> {
        let Some((kind, state, _)) = self.exterior_state(object_hex)? else { return Ok(Vec::new()) };
        let Some(d) = state.door.as_ref() else { return Ok(Vec::new()) };
        let gid = self.object_group_id(object_hex)?;
        let address = Address::from_seed(&door::address_seed(&door::public_of(&d.key)));
        let mut sess = Router::open(&self.routes).await?;
        let msgs = sess.drain(&[address.tag_hex()], |_| Ok(0)).await;
        sess.close().await;
        let inside = self.dir.group_members(&gid)?;
        let mut asked: BTreeMap<[u8; 32], DoorRequest> = BTreeMap::new();
        let mut withdrawn: BTreeSet<([u8; 32], [u8; 32])> = BTreeSet::new();
        for m in msgs? {
            let opened = pacific_wire::blob_unb64(&m.blob)
                .map_err(|e| CoreError::Seal(format!("base64: {e}")))
                .and_then(|blob| Ok((door::id_of(&blob), door::decode(&door::open_with(&d.key, &blob)?)?)));
            match opened {
                Ok((id, Knock::Request(r))) if r.object == gid => match self.askers_bundles(&r) {
                    Ok(name) => {
                        asked.insert(id, DoorRequest { object: object_hex.into(), kind: kind.name().into(), request: hex::encode(id), asker: r.asker, asker_name: name, note: r.note, at: r.at, bundles: r.bundles });
                    }
                    Err(e) => tracing::debug!(object = %object_hex, error = %e, "a request whose bundles are not its asker's; set aside"),
                },
                Ok((_, Knock::Withdraw(w))) if w.object == gid => {
                    withdrawn.insert((w.request, w.asker));
                }
                Ok(_) => tracing::debug!(object = %object_hex, "a knock naming another object; set aside"),
                Err(e) => tracing::debug!(object = %object_hex, error = %e, "an unusable knock; set aside"),
            }
        }
        // One request per asker: their latest.
        let mut latest: BTreeMap<[u8; 32], DoorRequest> = BTreeMap::new();
        for (id, r) in asked {
            if state.decided.contains_key(&id) || inside.contains(&r.asker) || withdrawn.contains(&(id, r.asker)) {
                continue;
            }
            if latest.get(&r.asker).is_none_or(|held| r.at > held.at) {
                latest.insert(r.asker, r);
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

    /// Every request waiting for ME: at the `review` doors of objects I own or admit to.
    pub async fn pending_requests(&self) -> Result<Vec<DoorRequest>, CoreError> {
        let me = self.id.identity_pk();
        let mut out = Vec::new();
        for (gid, _) in self.dir.list_objects()? {
            let object = hex::encode(&gid);
            let Ok(Some((_, state, _))) = self.exterior_state(&object) else { continue };
            if !state.door.as_ref().is_some_and(|d| d.mode == DoorMode::Review) {
                continue;
            }
            let mine = self.dir.group_owner(&gid)?.is_some_and(|o| o == me) || self.is_admitter(&object, &me);
            if !mine {
                continue;
            }
            match self.door_requests(&object).await {
                Ok(mut v) => out.append(&mut v),
                Err(e) => tracing::debug!(object = %object, error = %e, "a door not read"),
            }
        }
        Ok(out)
    }

    async fn admit_request(&self, r: &DoorRequest) -> Result<(), CoreError> {
        let mut last = CoreError::Coordinator("a request with no bundle".into());
        for b in &r.bundles {
            match self.group_add_member(&r.object, b).await {
                Ok(_) => return Ok(()),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// The owner's or an admitter's answer to one request. Admitting adds the asker by one of
    /// their bundles, then records the decision; declining records it, and the asker is not told.
    pub async fn decide_request(&self, object_hex: &str, request_hex: &str, admit: bool) -> Result<(), CoreError> {
        let r = self
            .door_requests(object_hex)
            .await?
            .into_iter()
            .find(|r| r.request == request_hex)
            .ok_or_else(|| CoreError::Coordinator("no such request is waiting at this door".into()))?;
        if admit {
            self.admit_request(&r).await?;
        }
        let id: [u8; 32] = hex::decode(request_hex).ok().and_then(|v| v.try_into().ok()).ok_or_else(|| CoreError::Coordinator("a request id is 64 hex".into()))?;
        self.apply(object_hex, ext::OP_DECIDE_REQUEST, ext::decide_request_args(&id, &r.asker, admit)).await?;
        Ok(())
    }

    /// Answer every `auto` door I am behind: whoever asked is admitted. Runs on every member,
    /// as the Place's doorbell does, so a door is answered while anyone inside is about.
    pub async fn answer_exterior_doors(&self) -> Result<Vec<String>, CoreError> {
        let mut admitted = Vec::new();
        for (gid, _) in self.dir.list_objects()? {
            let object = hex::encode(&gid);
            let Ok(Some((_, state, _))) = self.exterior_state(&object) else { continue };
            if !state.door.as_ref().is_some_and(|d| d.mode == DoorMode::Auto) {
                continue;
            }
            for r in self.door_requests(&object).await.unwrap_or_default() {
                match self.admit_request(&r).await {
                    Ok(()) => admitted.push(object.clone()),
                    Err(e) => tracing::debug!(object = %object, error = %e, "an auto door could not admit"),
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
