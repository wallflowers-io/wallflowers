//! Shared multi-user test harness — the robust way to exercise N real users.
//!
//! Each `User` is a fully isolated device: its own temp state dir, its own minted
//! identity, its own SQLite + MLS keys. A single real relay (`relay::serve`) sits
//! between them. The harness drives the SAME `Node` facade the CLI and the iOS app
//! use — real pairing, real MLS commits, real Deltas over the wire — and folds the
//! same projection. Nothing is mocked; a passing scenario is a proof the live
//! system converges.
//!
//! ## The env-switching discipline
//! `Node` resolves its state dir from the process-global `PACIFIC_STATE_DIR`, so only
//! ONE user can be "active" in-process at a time and only ONE harness scenario may
//! run at once. The harness holds a global lock for its whole lifetime (tests in a
//! binary are serialized) and every op re-points the env at its user's sandbox
//! before opening that user's `Node`. Each op fully opens+acts+drops its `Node`, so
//! no handle ever outlives its activation — there is no window for a cross-user
//! state bleed. This faithfully models real device boundaries: users act one at a
//! time, then sync over the relay.

#![allow(dead_code)] // shared across test binaries; not every test uses every helper.

/// THE FOLD CACHE'S MEASURE (O-69, FC-10), as the Door installs it (app/door/src/counting.rs):
/// the system allocator, counting each thread's bytes allocated net of those freed. Every
/// binary with this harness counts, so a model in the env turns the cache on under it.
pub mod counting {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    pub struct Counting;

    thread_local! {
        static NET: Cell<isize> = const { Cell::new(0) };
    }

    fn count(n: isize) {
        let _ = NET.try_with(|c| c.set(c.get() + n));
    }

    /// This thread's bytes allocated, net of those it freed.
    pub fn allocated() -> isize {
        NET.try_with(Cell::get).unwrap_or(0)
    }

    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, l: Layout) -> *mut u8 {
            let p = System.alloc(l);
            if !p.is_null() {
                count(l.size() as isize);
            }
            p
        }

        unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
            let p = System.alloc_zeroed(l);
            if !p.is_null() {
                count(l.size() as isize);
            }
            p
        }

        unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
            System.dealloc(p, l);
            count(-(l.size() as isize));
        }

        unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
            let q = System.realloc(p, l, new);
            if !q.is_null() {
                count(new as isize - l.size() as isize);
            }
            q
        }
    }
}

#[global_allocator]
static COUNTING: counting::Counting = counting::Counting;

use std::sync::Mutex;

use pacific_core::contact::SelfProfile;
use pacific_core::coordinator::{ArgVal, Args, Ballot, ForumMessage, MsgRef, Rule};
use pacific_core::group::{ContactCard, GroupShape, GroupView, OP_SET_PRESENCE, OP_SET_PROFILE};
use pacific_core::node::RatifyView;
use pacific_core::router::Routes;
use pacific_core::Node;

/// The replica-invariant CONTENT of a folded view: every field except the
/// sender-relative `receipt` code (each device fills that for its own messages
/// only, so it never agrees across devices). Convergence assertions compare this.
fn content(v: &[ForumMessage]) -> Vec<ForumMessage> {
    v.iter()
        .map(|m| ForumMessage {
            receipt: 0,
            ..m.clone()
        })
        .collect()
}

/// PACIFIC_STATE_DIR is process-global. Every `Harness` holds this for its lifetime,
/// serializing scenarios within a test binary. Poison is recovered (a panicking
/// test shouldn't wedge the rest).
static ENV_LOCK: Mutex<()> = Mutex::new(());

async fn spawn_relay() -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { relay::serve(l).await });
    format!("ws://{addr}")
}

/// Pin the device in the current state dir to the harness relay, BEFORE its Node exists: its
/// routes, and its default Arc. The self record is minted with the identity and stamped with
/// the Arc it finds; stamped with production's (`paths::DEFAULT_ARC_URL`) on a device whose Arc
/// is local, it routes to production (`routes_for`; NC-85's class).
fn pin(relay: &str) {
    Routes::parse(relay).save().unwrap();
    pacific_core::node::set_default_arc(relay).unwrap();
}

/// One isolated user/device.
pub struct User {
    pub name: String,
    /// The identity pubkey — doubles as the DM peer address AND the author key the
    /// commutative fold keys membership on (same value the core stamps on a Delta).
    ///
    /// NOT unique across the harness once a person has more than one device: every
    /// device of one person restores the SAME identity, which is the whole point of
    /// [`Harness::people`] and the reason `id` cannot be used to tell devices apart.
    pub id: [u8; 32],
    home: tempfile::TempDir,
    /// Index into `Harness::people` — who is holding this device.
    person: usize,
    /// The device's label within its person ("phone", "laptop"). Unique per person,
    /// not globally.
    label: String,
}

/// One PERSON, and the devices they are signed in on.
///
/// The distinction this type exists to make: a `User` is a device — a leaf, its own
/// state dir, its own MLS signing key — and a `Person` is who is holding it. Before
/// this the two were the same struct, so "the same account signed in twice" had
/// nowhere to be written down.
pub struct Person {
    pub name: String,
    /// Indices into `Harness::users`, in the order the devices were added. The first
    /// is the one that MINTED the identity; the rest restored it from the 24 words.
    devices: Vec<usize>,
    /// The 24 words the FIRST device emitted. Kept because that is the only way a
    /// later device can be this same person — [`Harness::add_device`] restores from
    /// exactly these. `None` for an identity that arrived without emitting them.
    recovery: Option<String>,
}

/// N isolated devices, held by M people, + one relay. Drop releases the global lock.
pub struct Harness {
    _guard: std::sync::MutexGuard<'static, ()>,
    pub relay: String,
    users: Vec<User>,
    people: Vec<Person>,
}

impl Harness {
    /// Spawn a relay and mint one isolated, identified user per name — one person,
    /// one device, which is what every scenario written before multi-device assumes.
    /// Equivalent to [`Harness::people`] with a single unnamed device each.
    pub async fn new(names: &[&str]) -> Self {
        let spec: Vec<(&str, &[&str])> = names.iter().map(|n| (*n, &["a"][..])).collect();
        Self::people(&spec).await
    }

    /// Spawn a relay and mint M people, each holding N devices.
    ///
    /// ```ignore
    /// Harness::people(&[
    ///     ("ada", &["phone", "laptop"]),   // the same account, signed in twice
    ///     ("bo",  &["sim"]),
    /// ]).await
    /// ```
    ///
    /// THE DEVICES OF ONE PERSON SHARE AN IDENTITY, and they get it the way a real
    /// second device does: the first device mints, and every later one calls
    /// `Node::restore_identity` with the 24 words the mint emitted. So `id` is
    /// byte-identical across a person's devices — the same key, in two places, which
    /// is the thing being modelled and not a shortcut around it. Each device still
    /// has its own state dir, its own store and its own MLS signing key.
    ///
    /// What this does NOT do is put two of a person's devices in one MLS group.
    /// That is a separate act with its own call — somebody already in the group
    /// adds the device's contact bundle, exactly as they would add a person — and
    /// since `mls::PerLeafIdentity` (A1a) it works; `m21_one_person_two_devices.rs`
    /// is where it is proved, and `add_device` is how a scenario does it later.
    pub async fn people(spec: &[(&str, &[&str])]) -> Self {
        let guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        pacific_core::fold_cache::set_measure(counting::allocated);
        let relay = spawn_relay().await;
        let mut users: Vec<User> = Vec::new();
        let mut people: Vec<Person> = Vec::new();

        for (person_name, labels) in spec {
            assert!(
                !labels.is_empty(),
                "person '{person_name}' was given no devices — a person with no device \
                 cannot act, and a silent empty roster is how that goes unnoticed"
            );
            let mut devices = Vec::with_capacity(labels.len());
            // The words the first device emits; every later device of this person is
            // restored from them, which is what makes it the SAME account.
            let mut recovery: Option<String> = None;

            for label in labels.iter() {
                let home = tempfile::tempdir().unwrap();
                std::env::set_var("PACIFIC_STATE_DIR", home.path());
                // Transport is the CORE's state now, not an argument on every call: pin this
                // sandbox to the harness relay once, and every later `Node::open()` here loads it.
                pin(&relay);
                let node = match &recovery {
                    None => Node::init_identity(person_name).unwrap(),
                    Some(words) => Node::restore_identity(person_name, words).unwrap(),
                };
                let id = node.id.identity_pk();
                if recovery.is_none() {
                    recovery = Some(node.recovery_key().expect(
                        "a freshly minted identity emits a recovery key — without one \
                         this person cannot have a second device",
                    ));
                }
                drop(node);
                devices.push(users.len());
                users.push(User {
                    name: person_name.to_string(),
                    id,
                    home,
                    person: people.len(),
                    label: label.to_string(),
                });
            }
            people.push(Person {
                name: person_name.to_string(),
                devices,
                recovery,
            });
        }

        Harness {
            _guard: guard,
            relay,
            users,
            people,
        }
    }

    /// Introduce a fresh isolated identity mid-scenario — models a party that
    /// installs Pacific AFTER the conversation started (e.g. the company invited to
    /// represent itself). Same isolation as `new`: own tempdir + PACIFIC_STATE_DIR +
    /// minted identity. Returns the new user's index. It can `pair`/DM like any
    /// other user — a real second identity, never a pre-seeded fake.
    pub fn add_user(&mut self, name: &str) -> usize {
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("PACIFIC_STATE_DIR", home.path());
        pin(&self.relay);
        let node = Node::init_identity(name).unwrap();
        let id = node.id.identity_pk();
        let recovery = node.recovery_key();
        drop(node);
        self.users.push(User {
            name: name.to_string(),
            id,
            home,
            person: self.people.len(),
            label: "a".to_string(),
        });
        self.people.push(Person {
            name: name.to_string(),
            devices: vec![self.users.len() - 1],
            recovery,
        });
        self.users.len() - 1
    }

    /// A user brought back on a device that never held them — the identity door
    /// only: `Node::restore_from_wrap`. Same isolation as `add_user` (own
    /// tempdir, own `PACIFIC_STATE_DIR`) and NO minted identity: the one in the
    /// wrap IS the identity, so `id(u)` reads the exporter's key, which is the
    /// point. Returns the new index.
    ///
    /// IT BRINGS NO GROUPS, and that is the design rather than a gap in the
    /// helper. The second door used to be `Node::import_history`, which restored
    /// `mls_mem::Snapshot` — every group's ratchet tree, epoch secrets and
    /// key-package private halves — from a blob at the Arc. Re-admission §1
    /// called that a complete key escrow and the register retired it on 14
    /// September 2026; it has now been removed from the code as well as the
    /// documents. A restored user here is therefore a member of nothing; the path
    /// that goes on to resume every object is [`Harness::sign_in_from_wrap`].
    pub fn add_user_from_wrap(
        &mut self,
        name: &str,
        prf: &[u8; 32],
        wrap: &[u8],
        host: &str,
    ) -> usize {
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("PACIFIC_STATE_DIR", home.path());
        pin(&self.relay);
        let node = Node::restore_from_wrap(prf, wrap, host, name).unwrap();
        let id = node.id.identity_pk();
        let recovery = node.recovery_key();
        drop(node);
        self.users.push(User {
            name: name.to_string(),
            id,
            home,
            person: self.people.len(),
            label: "restored".to_string(),
        });
        self.people.push(Person {
            name: name.to_string(),
            devices: vec![self.users.len() - 1],
            recovery,
        });
        self.users.len() - 1
    }

    /// A person signs in on a device that holds nothing, from the wrap alone: the
    /// Door's path (mdr/door.md §4), `Node::sign_in_from_wrap`. This harness's
    /// relay goes into the new state dir first, because `resume` walks the chain
    /// there. The device joins `person` if the harness holds them, or is a new
    /// person (an account made where no Node ran). Nothing is added on a refusal.
    #[allow(clippy::too_many_arguments)]
    pub async fn sign_in_from_wrap(
        &mut self,
        person: &str,
        label: &str,
        prf: &[u8; 32],
        wrap: &[u8],
        host: &str,
        expect_key: &str,
        head: Option<pacific_core::resumption::HeadInput>,
    ) -> Result<(usize, pacific_core::resumption::Resumed), pacific_core::CoreError> {
        let pi = self.people.iter().position(|p| p.name == person);
        if let Some(pi) = pi {
            assert!(
                !self.people[pi].devices.iter().any(|&d| self.users[d].label == label),
                "person '{person}' already holds a device labelled '{label}'"
            );
        }
        let home = tempfile::tempdir().unwrap();
        std::env::set_var("PACIFIC_STATE_DIR", home.path());
        pin(&self.relay);
        let (node, resumed) =
            Node::sign_in_from_wrap(prf, wrap, host, person, expect_key, head).await?;
        let id = node.id.identity_pk();
        let recovery = node.recovery_key();
        drop(node);
        let u = self.users.len();
        let pi = pi.unwrap_or_else(|| {
            self.people.push(Person { name: person.to_string(), devices: vec![], recovery });
            self.people.len() - 1
        });
        self.users.push(User { name: person.to_string(), id, home, person: pi, label: label.to_string() });
        self.people[pi].devices.push(u);
        Ok((u, resumed))
    }

    /// A PERSON ACQUIRES ANOTHER DEVICE, mid-scenario — the second half of the
    /// multi-device story that [`Harness::people`] can only tell up front.
    ///
    /// It is the same act `people` performs for every device after the first: a
    /// fresh tempdir, a fresh `PACIFIC_STATE_DIR`, and `Node::restore_identity`
    /// with the 24 words the person's FIRST device minted. So the new device is
    /// the same account — byte-identical `id`, same fingerprint words — with its
    /// own store, its own MLS leaf key and its own intro tag. Returns its index.
    ///
    /// It is NOT in any group yet. A device becomes a leaf the way a member does:
    /// somebody who is already in the group adds its contact bundle. That is
    /// deliberate — "acquiring a phone" and "putting it in a group" are separate
    /// events and a scenario usually wants to observe the gap between them.
    pub fn add_device(&mut self, person: &str, label: &str) -> usize {
        let pi = self
            .people
            .iter()
            .position(|p| p.name == person)
            .unwrap_or_else(|| panic!("no person '{person}' in this harness"));
        assert!(
            !self.people[pi]
                .devices
                .iter()
                .any(|&d| self.users[d].label == label),
            "person '{person}' already holds a device labelled '{label}' — labels are \
             the handle every other verb takes, so a duplicate would silently resolve \
             to the wrong device"
        );
        let words = self.people[pi].recovery.clone().unwrap_or_else(|| {
            panic!(
                "person '{person}' has no recovery key, so they cannot have a second \
                 device — this identity did not arrive through a mint"
            )
        });

        let home = tempfile::tempdir().unwrap();
        std::env::set_var("PACIFIC_STATE_DIR", home.path());
        pin(&self.relay);
        let node = Node::restore_identity(person, &words).unwrap();
        let id = node.id.identity_pk();
        drop(node);
        assert_eq!(
            id, self.users[self.people[pi].devices[0]].id,
            "a restored device of '{person}' must carry the SAME identity key — a \
             different one is a second person wearing the same name"
        );

        let u = self.users.len();
        self.users.push(User {
            name: person.to_string(),
            id,
            home,
            person: pi,
            label: label.to_string(),
        });
        self.people[pi].devices.push(u);
        u
    }

    pub fn len(&self) -> usize {
        self.users.len()
    }
    pub fn id(&self, u: usize) -> [u8; 32] {
        self.users[u].id
    }
    pub fn name(&self, u: usize) -> &str {
        &self.users[u].name
    }

    // ---- people and their devices ----

    /// The device index for `person`'s device labelled `label` — the handle every
    /// other verb on this harness takes. Panics with both names on a miss, because
    /// a typo'd device label silently resolving to device 0 would make a
    /// multi-device scenario quietly pass as a single-device one.
    pub fn device(&self, person: &str, label: &str) -> usize {
        let p = self
            .people
            .iter()
            .find(|p| p.name == person)
            .unwrap_or_else(|| panic!("no person '{person}' in this harness"));
        *p.devices
            .iter()
            .find(|&&d| self.users[d].label == label)
            .unwrap_or_else(|| {
                let have: Vec<&str> = p
                    .devices
                    .iter()
                    .map(|&d| self.users[d].label.as_str())
                    .collect();
                panic!("person '{person}' has no device '{label}' — they hold {have:?}")
            })
    }

    /// Every device `person` is signed in on, in the order they were added.
    pub fn devices(&self, person: &str) -> Vec<usize> {
        self.people
            .iter()
            .find(|p| p.name == person)
            .unwrap_or_else(|| panic!("no person '{person}' in this harness"))
            .devices
            .clone()
    }

    /// Who is holding device `u`.
    pub fn person_of(&self, u: usize) -> &str {
        &self.people[self.users[u].person].name
    }

    /// Device `u` written the way a failure should name it: `person/label`.
    pub fn device_name(&self, u: usize) -> String {
        format!("{}/{}", self.users[u].name, self.users[u].label)
    }

    /// The roster of `obj` as PEOPLE, not leaves — each identity resolved to whoever
    /// holds it, deduplicated, in first-seen order.
    ///
    /// This is the assertion multi-device exists to satisfy: four leaves belonging to
    /// two people must read as two rows. An identity no person in this harness holds
    /// is reported as `unknown:<8 hex>` rather than dropped — a roster entry nobody
    /// can account for is the interesting case, so it must not vanish into a filter.
    pub fn roster_people(&self, u: usize, obj: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for id in self.roster(u, obj) {
            let who = self
                .people
                .iter()
                .find(|p| p.devices.iter().any(|&d| self.users[d].id == id))
                .map(|p| p.name.clone())
                .unwrap_or_else(|| format!("unknown:{}", hex::encode(&id[..4])));
            if !out.contains(&who) {
                out.push(who);
            }
        }
        out
    }

    /// Point the process env at user `u`'s sandbox, then open their `Node`. Every
    /// op calls this and drops the `Node` before returning, so an activation never
    /// outlives one op — no cross-user bleed while the harness holds the lock.
    /// A Node bound to user `u`'s isolated home. `pub` so scenario tests can drive
    /// core verbs directly, not only through this harness's convenience wrappers.
    pub fn node(&self, u: usize) -> Node {
        std::env::set_var("PACIFIC_STATE_DIR", self.users[u].home.path());
        Node::open().unwrap()
    }

    /// Cut a device off the relay, and put it back. `apply` drains to head before it
    /// seals, so a device that can REACH the relay converges instead of forking: to
    /// write "two devices in one pocket, one asleep" you have to make asleep real.
    /// `set_routes` persists into the device's state dir, so it outlives `node(u)`.
    /// Authoring still succeeds while partitioned — `Router::open` fails, the publish
    /// block is skipped, and the delta waits in the outbox.
    pub fn partition(&self, u: usize) {
        let mut n = self.node(u);
        n.set_routes(Routes::parse("ws://127.0.0.1:1")).unwrap();
    }

    /// Put `u` back on the relay the rest of the harness uses.
    pub fn rejoin(&self, u: usize) {
        let mut n = self.node(u);
        n.set_routes(Routes::parse(&self.relay)).unwrap();
    }

    // ---- pairing (double-opt-in DM connection) ----

    /// Form the full 2-party connection between users `a` and `b` (both opted in),
    /// exactly as two phones would: `b` scans `a`'s bundle over the relay, then
    /// each side accepts. After this, DM ops in either direction work.
    pub async fn pair(&self, a: usize, b: usize) {
        let a_bundle = self.node(a).build_contact_bundle().unwrap();
        {
            let n = self.node(b);
            let peer = n.pair_scan(&a_bundle).await.unwrap();
            assert_eq!(peer, self.users[a].id, "pair_scan yielded a's identity");
            n.pair_accept(&peer).await.unwrap();
        }
        {
            let n = self.node(a);
            n.sync_once().await.unwrap(); // drain b's Welcome
            n.pair_accept(&self.users[b].id).await.unwrap();
        }
    }

    // ---- group objects (N-member forum) ----

    /// `owner` mints a forum object and owner-adds each member in turn (each a real
    /// staged MLS commit through the relay's commit slot); every member syncs to
    /// join, then everyone settles to the final epoch. Returns the object id hex —
    /// the SAME hex every member addresses (the MLS group id is stable across
    /// epochs). This is the real N-member group path, not the in-process mechanic.
    pub async fn form_forum(&self, owner: usize, members: &[usize]) -> String {
        self.form_object(owner, "forum", members).await
    }
    /// Create a NAMED forum as a solo group-of-1 (the name rides the GroupContext).
    pub fn form_named_forum(&self, owner: usize, name: &str) -> String {
        self.node(owner).object_new("forum", name).unwrap()
    }
    /// The display name `u` reads for `obj` (from its MLS GroupContext extension).
    pub fn object_name(&self, u: usize, obj: &str) -> String {
        self.node(u).object_name(obj).unwrap()
    }
    /// Owner-rename `obj` (a GroupContextExtensions commit through the epoch's
    /// commit slot), then settle so every member folds it.
    pub async fn rename_object(&self, owner: usize, obj: &str, name: &str) {
        self.try_rename(owner, obj, name).await.unwrap();
        self.settle().await;
    }
    /// The fallible twin of [`rename_object`] — for asserting the owner-only gate.
    pub async fn try_rename(
        &self,
        u: usize,
        obj: &str,
        name: &str,
    ) -> Result<(), pacific_core::CoreError> {
        self.node(u).object_rename(obj, name).await
    }

    /// The general N-member object path for any kind ("forum", "project", …):
    /// `owner` mints it, owner-adds each member, everyone settles. Returns the
    /// object id hex (stable across epochs — the address every member uses).
    pub async fn form_object(&self, owner: usize, kind: &str, members: &[usize]) -> String {
        let obj = self.node(owner).object_new(kind, "").unwrap();
        for &m in members {
            self.owner_add(owner, m, &obj).await;
        }
        self.settle().await;
        obj
    }

    /// FORM AN OBJECT THE WAY THE PRODUCT DOES: `Node::mint`, so the kind's op 0
    /// is authored and its CONSTITUENT PARTS are brought into being (m34) — a Post
    /// gets its comments Forum. `form_object` above is the primitive, `object_new`,
    /// which is right for a scenario that only wants a roster and wrong for one
    /// about what a kind IS.
    pub async fn mint_object(&self, owner: usize, kind: pacific_core::object::ObjectKind, name: &str, members: &[usize]) -> String {
        let draft = pacific_core::mint::MintDraft { name: name.to_string(), ..Default::default() };
        let obj = self.node(owner).mint(kind, &draft).await.unwrap();
        for &m in members {
            self.owner_add(owner, m, &obj).await;
        }
        self.settle().await;
        obj
    }

    /// Owner-add one more member to an ALREADY-formed object, then settle. The
    /// newcomer backfills the whole prior transcript from the group log on join.
    pub async fn add_to_forum(&self, owner: usize, member: usize, obj: &str) {
        self.owner_add(owner, member, obj).await;
        self.settle().await;
    }

    /// One owner-add (real staged commit through the relay's commit slot) + the
    /// newcomer's join-sync. No settle — callers batch that.
    async fn owner_add(&self, owner: usize, member: usize, obj: &str) {
        let bundle = self.node(member).build_contact_bundle().unwrap();
        self.node(owner)
            .group_add_member(obj, &bundle)
            .await
            .unwrap();
        self.node(member).sync_once().await.unwrap(); // drain the Welcome → join
    }

    // ---- group objects (GroupType identity records / the directory) ----

    /// `owner` mints a group-of-1 identity record (kind "group") — a private
    /// directory entry, owned by `owner`, membership = just `owner` (the MLS
    /// ratchet tree). Returns the object id hex.
    pub fn mint_group(&self, owner: usize) -> String {
        self.node(owner).object_new("group", "").unwrap()
    }

    /// Author the Group's profile (display name + shape ∈ individual|team|organisation).
    pub async fn group_set_profile(&self, owner: usize, obj: &str, name: &str, shape: &str) {
        let mut a = Args::new();
        a.insert("displayName".into(), ArgVal::Text(name.into()));
        a.insert("shape".into(), ArgVal::Text(shape.into()));
        self.node(owner)
            .apply(obj, OP_SET_PROFILE, a)
            .await
            .unwrap();
    }

    /// Mark the Group OFF-platform with an out-of-band invite hint (email/phone) —
    /// a known contact/entity we can't yet reach over MLS.
    pub async fn group_presence_off(&self, owner: usize, obj: &str, hint: &str) {
        let mut a = Args::new();
        a.insert("kind".into(), ArgVal::Text("offPlatform".into()));
        a.insert("inviteHint".into(), ArgVal::Text(hint.into()));
        self.node(owner)
            .apply(obj, OP_SET_PRESENCE, a)
            .await
            .unwrap();
    }

    /// Mark the Group ON-platform at a real IdentityKey (`space1<hex>`) — reachable
    /// over MLS. The reducer rejects a non-32-byte space, so pass a real one.
    pub async fn group_presence_on(&self, owner: usize, obj: &str, space: &str) {
        let mut a = Args::new();
        a.insert("kind".into(), ArgVal::Text("onPlatform".into()));
        a.insert("identityKey".into(), ArgVal::Text(space.into()));
        self.node(owner)
            .apply(obj, OP_SET_PRESENCE, a)
            .await
            .unwrap();
    }

    /// The folded Group read-model as user `u` sees it (profile, presence, card,
    /// roles projected onto the live roster, can_sync).
    pub fn group_view(&self, u: usize, obj: &str) -> GroupView {
        self.node(u).group_view(obj).unwrap()
    }

    /// The live MLS roster (identity pubkeys) of an object as user `u` sees it —
    /// the `group_members` projection of the ratchet tree (a rebuildable cache of
    /// the MLS membership, never the Delta log).
    pub fn roster(&self, u: usize, obj: &str) -> Vec<[u8; 32]> {
        self.node(u).object_members(obj).unwrap()
    }

    /// How many LEAVES device `u` sees in `obj`'s ratchet tree — the tree's own
    /// width, straight off the MLS group, NOT the deduplicated member projection.
    ///
    /// `roster_people` says how many PEOPLE are in a group and this says how many
    /// leaves; since `mls::PerLeafIdentity` those are different numbers, and every
    /// multi-device assertion is really about the gap between them.
    pub fn leaves(&self, u: usize, obj: &str) -> usize {
        self.with_mls_group(u, obj, |g| g.roster().members().len())
    }

    /// The pool leaves in `obj`'s tree as `u` holds it: every leaf whose key some
    /// person's spine names in a `Pool` entry (resumption.md §5). Each device's own
    /// spine rows are read, so a pool leaf provisioned anywhere in the harness counts.
    pub fn pool_leaves(&self, u: usize, obj: &str) -> usize {
        let pools = self.pool_keys();
        self.with_mls_group(u, obj, |g| {
            g.roster()
                .members()
                .iter()
                .filter(|m| {
                    <[u8; 32]>::try_from(m.signing_identity.signature_key.as_bytes())
                        .is_ok_and(|k| pools.contains(&k))
                })
                .count()
        })
    }

    /// Every pool leaf's key any person's spine names, as the harness's devices hold
    /// their spines.
    pub fn pool_keys(&self) -> std::collections::HashSet<[u8; 32]> {
        let mut pools = std::collections::HashSet::new();
        for d in 0..self.len() {
            let entries = self.with_sync(d, |n| n.spine_entries().unwrap_or_default());
            for (_, e) in entries {
                if let pacific_core::spine::Body::Pool(p) = e.body {
                    pools.extend(p.pools.iter().map(|l| l.pool));
                }
            }
        }
        pools
    }

    /// `mls::roster_identities` less the pool leaves: one entry per DEVICE leaf, no
    /// dedup — what the multi-device tests mean by the per-leaf Vec.
    pub fn device_identities(&self, u: usize, obj: &str) -> Vec<[u8; 32]> {
        let pools = self.pool_keys();
        self.with_mls_group(u, obj, |g| {
            g.roster()
                .members()
                .iter()
                .filter(|m| {
                    <[u8; 32]>::try_from(m.signing_identity.signature_key.as_bytes())
                        .map_or(true, |k| !pools.contains(&k))
                })
                .filter_map(|m| m.signing_identity.credential.as_basic().and_then(|b| <[u8; 32]>::try_from(b.identifier.as_slice()).ok()))
                .collect()
        })
    }

    /// The leaves that are DEVICES: the tree less its pool leaves. What the
    /// multi-device tests always meant by "leaves", from before every object carried
    /// one idle pool leaf per person.
    pub fn device_leaves(&self, u: usize, obj: &str) -> usize {
        self.leaves(u, obj) - self.pool_leaves(u, obj)
    }

    /// The MLS EPOCH device `u` holds for `obj`.
    ///
    /// No accessor for this exists on `Node` or on the harness, and it is the only
    /// way to say "neither device forked the epoch" — two devices on the same
    /// epoch number with the same transcript are in one group, two devices on
    /// diverging epochs are two groups wearing one name. Reached the way `node.rs`
    /// reaches a group: rebuild the client over this device's SQLite stores and
    /// `load_group`.
    pub fn epoch(&self, u: usize, obj: &str) -> u64 {
        self.with_mls_group(u, obj, |g| g.current_epoch())
    }

    /// Open `obj`'s live MLS group on device `u` and hand it to `f`. The shared
    /// half of [`epoch`] and [`leaves`]; a test wanting something else off the
    /// ratchet tree should use it rather than rebuilding the client again.
    ///
    /// [`epoch`]: Self::epoch
    /// [`leaves`]: Self::leaves
    pub fn with_mls_group<T, F>(&self, u: usize, obj: &str, f: F) -> T
    where
        F: FnOnce(&pacific_core::mls::Group) -> T,
    {
        let node = self.node(u); // re-points PACIFIC_STATE_DIR at this device
        let (sk, pk) = node
            .dir
            .mls_signing_keypair()
            .expect("this device has no MLS signing key");
        let sid = pacific_core::mls::signing_identity(&node.id.identity_pk(), &pk);
        let client = pacific_core::mls::build_client_sqlite(
            &pacific_core::paths::db_path(),
            sid,
            pacific_core::mls::SecretKey::new(sk),
        )
        .expect("could not open this device's MLS stores");
        let gid = hex::decode(obj).expect("an object id is hex");
        let group = pacific_core::mls::load_group(&client, &gid).unwrap_or_else(|e| {
            panic!(
                "{} holds no MLS group state for {obj} — it is not a leaf of that \
                 group (or has not joined yet): {e}",
                self.device_name(u)
            )
        });
        f(&group)
    }

    // ---- messaging (DM) ----

    pub async fn dm_post(&self, from: usize, to: usize, text: &str) {
        self.node(from)
            .author_post(&self.users[to].id, text)
            .await
            .unwrap();
    }
    pub async fn dm_reply(&self, from: usize, to: usize, text: &str, parent: MsgRef) {
        self.node(from)
            .author_post_reply(&self.users[to].id, text, Some(parent), None)
            .await
            .unwrap();
    }
    pub async fn dm_react(
        &self,
        from: usize,
        to: usize,
        target: MsgRef,
        emoji: &str,
        active: bool,
    ) {
        self.node(from)
            .react_dm(&self.users[to].id, target, emoji, active)
            .await
            .unwrap();
    }
    /// How many usable prekeys `u` holds for `peer` (offers `peer` stocked into their
    /// shared Contact channel).
    pub fn contact_prekey_count(&self, u: usize, peer: usize) -> usize {
        self.node(u)
            .contact_prekeys(&self.users[peer].id)
            .unwrap()
            .count_from(&self.users[peer].id, 0)
    }
    /// How many of `peer`'s offers `u` has seen TOMBSTONED (consumed or revoked).
    /// The durable evidence that an add spent a prekey — unlike the available
    /// count, which the replenishment pass restores on the next sync.
    pub fn contact_prekeys_spent(&self, u: usize, peer: usize) -> usize {
        self.node(u)
            .contact_prekeys(&self.users[peer].id)
            .unwrap()
            .spent
            .len()
    }
    /// The name `u` resolves for `peer` via the Contact→identity-Group edge.
    pub fn peer_identity_name(&self, u: usize, peer: usize) -> String {
        self.node(u)
            .peer_identity_name(&self.users[peer].id)
            .unwrap()
    }
    /// `owner` adds an existing `contact` to object `obj` by consuming a stocked
    /// prekey — no fresh code exchange. Settles so the contact joins.
    pub async fn add_contact_to(&self, owner: usize, contact: usize, obj: &str) {
        self.node(owner)
            .add_contact_to_object(obj, &self.users[contact].id)
            .await
            .unwrap();
        self.settle().await;
    }
    pub fn dm_view(&self, u: usize, peer: usize) -> Vec<ForumMessage> {
        self.node(u).dm_detailed(&self.users[peer].id).unwrap()
    }
    /// `u` read-receipts every message it holds from `peer` (the ✓✓-blue path).
    /// Returns how many read receipts were authored.
    pub async fn dm_mark_read(&self, u: usize, peer: usize) -> usize {
        self.node(u)
            .dm_mark_read(&self.users[peer].id)
            .await
            .unwrap()
    }

    // ---- messaging (group object) ----

    pub async fn obj_post(&self, from: usize, obj: &str, text: &str) {
        self.node(from).object_post(obj, text, None).await.unwrap();
    }
    pub async fn obj_reply(&self, from: usize, obj: &str, text: &str, parent: MsgRef) {
        self.node(from)
            .object_post_reply(obj, text, Some(parent), None)
            .await
            .unwrap();
    }
    pub async fn obj_react(
        &self,
        from: usize,
        obj: &str,
        target: MsgRef,
        emoji: &str,
        active: bool,
    ) {
        self.node(from)
            .object_react(obj, target, emoji, active)
            .await
            .unwrap();
    }
    pub fn obj_view(&self, u: usize, obj: &str) -> Vec<ForumMessage> {
        self.node(u).object_detailed(obj).unwrap()
    }

    // ---- ratify (base governance) ----
    pub async fn obj_propose(&self, from: usize, obj: &str, payload: &str, rule: Rule) -> MsgRef {
        self.node(from)
            .object_propose(obj, payload, rule)
            .await
            .unwrap()
    }
    pub async fn obj_vote(&self, from: usize, obj: &str, target: MsgRef, ballot: Ballot) {
        self.node(from)
            .object_vote(obj, target, ballot)
            .await
            .unwrap();
    }
    pub async fn obj_close(&self, owner: usize, obj: &str, target: MsgRef) {
        self.node(owner).object_close(obj, target).await.unwrap();
    }
    pub fn obj_ratify(&self, u: usize, obj: &str) -> Vec<RatifyView> {
        self.node(u).object_ratify(obj).unwrap()
    }

    // ---- escape hatch for bespoke ops ----

    /// Run an arbitrary async op on user `u`'s `Node` while their env is active —
    /// for the few tests that call specialised Node methods (scripted demo seeds,
    /// project state) not wrapped above. The `Node` is opened fresh and handed in;
    /// do NOT stash it or touch another user inside `f` (the env would move under
    /// it). `self.relay` and `self.id(v)` are the values you'll usually capture.
    pub async fn with<T, F, Fut>(&self, u: usize, f: F) -> T
    where
        F: FnOnce(Node) -> Fut,
        Fut: std::future::Future<Output = T>,
    {
        f(self.node(u)).await
    }

    /// Synchronous variant of [`with`] for non-async reads (project_state, etc.).
    pub fn with_sync<T, F>(&self, u: usize, f: F) -> T
    where
        F: FnOnce(Node) -> T,
    {
        f(self.node(u))
    }

    /// User `u`'s space id ("space1<hex>") — for assertions that compare against a
    /// folded transcript's short author token.
    pub fn identity_key(&self, u: usize) -> String {
        format!("space1{}", hex::encode(self.users[u].id))
    }

    // ---- sync ----

    pub async fn sync(&self, u: usize) {
        self.node(u).sync_once().await.unwrap();
    }
    pub async fn sync_all(&self) {
        for u in 0..self.users.len() {
            self.sync(u).await;
        }
    }
    /// Sync everyone until the relay goes quiet: rounds of `sync_all` until one
    /// completes with no device reporting any work, so multi-hop propagation (a
    /// commit that reaches a member who then re-broadcasts) is followed to its end
    /// rather than guessed at.
    ///
    /// WHY NOT A ROUND COUNT. This used to run `max(len, 2)` rounds unconditionally,
    /// which is a guess in both directions: too few for a long chain, and N wasted
    /// round-trips for a scenario that settled on the first. The guess was
    /// survivable only because devices act in turn — under genuine concurrency a
    /// fixed count stops being conservative and starts being flaky, and a flaky
    /// convergence test teaches people to re-run rather than to read.
    ///
    /// THE SIGNAL, and the trap in it. `sync_once` returns the group's whole folded
    /// TRANSCRIPT, not a list of what this call applied — so it is never empty once
    /// anything has been said, and "empty means idle" is wrong. What is sound is the
    /// transcript not CHANGING: a round in which every device folds byte-identically
    /// to its previous round is a round in which nothing was delivered.
    ///
    /// That needs two rounds minimum (the first has nothing to compare against),
    /// which is the same floor the old fixed count had — the difference is the
    /// ceiling, which now moves with the scenario instead of being guessed.
    ///
    /// It waits for CONTENT to converge, which is what the assertions check. It does
    /// not wait for every last receipt to land: delivery acks change a message's
    /// `receipt` code, the transcript lines carry `(author, gen, text)` and not that
    /// code, so receipt traffic cannot hold settle open — and must not, since
    /// `content()` deliberately excludes the same field for the same reason.
    pub async fn settle(&self) {
        self.settle_within(12).await;
    }

    /// [`settle`] over a SUBSET of devices — everyone else is OFFLINE for the
    /// duration and syncs nothing.
    ///
    /// This is the only way to write "the laptop was shut" or "paola was in
    /// Xalapa": `settle` syncs every device in the harness, so a scenario that
    /// needs a device to miss some epochs cannot use it. The quiesce rule is
    /// identical — rounds until every LISTED device folds byte-identically to its
    /// previous round — and so is the failure: exhausting the ceiling panics.
    ///
    /// [`settle`]: Self::settle
    pub async fn settle_among(&self, who: &[usize]) {
        self.settle_among_within(who, 12).await;
    }

    /// [`settle`] with an explicit round ceiling. Exhausting it is a FAILURE, not a
    /// return: a scenario whose traffic never stops is the bug this would otherwise
    /// hide behind a green assertion further down.
    pub async fn settle_within(&self, max_rounds: usize) {
        let everyone: Vec<usize> = (0..self.users.len()).collect();
        self.settle_among_within(&everyone, max_rounds).await;
    }

    /// The one loop behind all four settle verbs: `who` syncs, nobody else does,
    /// `max_rounds` is the ceiling. Split out so a device can be offline without
    /// the quiesce rule being restated (and drifting) at the call site.
    pub async fn settle_among_within(&self, who: &[usize], max_rounds: usize) {
        assert!(
            !who.is_empty(),
            "settle_among was given no devices — nothing would sync and it would \
             return quiet on the second round, which is a pass that proves nothing"
        );
        let mut last: Vec<Option<Vec<String>>> = (0..self.users.len()).map(|_| None).collect();
        let mut moved: Vec<String> = Vec::new();

        for round in 1..=max_rounds {
            let mut quiet = true;
            for &u in who {
                let now = self.node(u).sync_once().await.unwrap();
                if last[u].as_ref() != Some(&now) {
                    quiet = false;
                    if round > 1 {
                        moved.push(format!(
                            "round {round}: {} moved to {} line(s)",
                            self.device_name(u),
                            now.len()
                        ));
                    }
                }
                last[u] = Some(now);
            }
            if quiet {
                return;
            }
        }
        let names: Vec<String> = who.iter().map(|&u| self.device_name(u)).collect();
        panic!(
            "settle did not quiesce in {max_rounds} rounds among {names:?} — devices \
             are still folding new content each round:\n  {}",
            moved.join("\n  ")
        );
    }

    // ---- the market (things, postures, and their fan-out) ----

    /// Mint a Thing for `u` — the same two-step the swipe deck's ACCEPT performs: a group of
    /// one, then its profile. Returns the object id.
    pub async fn mint_thing(&self, u: usize, name: &str) -> String {
        let n = self.node(u);
        let id = n.object_new("thing", name).unwrap();
        let mut args = pacific_core::coordinator::Args::new();
        args.insert(
            "name".into(),
            pacific_core::coordinator::ArgVal::Text(name.into()),
        );
        args.insert(
            "descriptor".into(),
            pacific_core::coordinator::ArgVal::Text(String::new()),
        );
        n.apply(&id, pacific_core::thing::OP_SET_PROFILE, args)
            .await
            .unwrap();
        id
    }

    /// Put a thing on the market. `price` is only legal on an active posture, exactly as the
    /// reducer requires.
    pub async fn set_posture(
        &self,
        u: usize,
        id: &str,
        posture: pacific_core::thing::Posture,
        price: Option<&str>,
        reach: pacific_core::thing::Reach,
    ) {
        use pacific_core::coordinator::ArgVal;
        let mut args = pacific_core::coordinator::Args::new();
        args.insert("posture".into(), ArgVal::Text(posture.as_str().into()));
        if let Some(p) = price {
            args.insert("price".into(), ArgVal::Text(p.into()));
        }
        args.insert("reach".into(), ArgVal::Text(reach.as_str().into()));
        self.node(u)
            .apply(id, pacific_core::thing::OP_SET_POSTURE, args)
            .await
            .unwrap();
    }

    /// Take a thing off the market — sold, lent, thought better of.
    pub async fn clear_posture(&self, u: usize, id: &str) {
        self.node(u)
            .apply(
                id,
                pacific_core::thing::OP_CLEAR_POSTURE,
                pacific_core::coordinator::Args::new(),
            )
            .await
            .unwrap();
    }

    /// Run `u`'s market pass: fan out their own postures, then relay what they hold onward.
    /// Returns the number of deltas authored — 0 when nothing has changed.
    pub async fn publish_market(&self, u: usize) -> usize {
        self.node(u).publish_market(false).await.unwrap()
    }

    /// Every live listing `u` has discovered from others, deduped across channels.
    pub fn discovered(&self, u: usize) -> Vec<pacific_core::contact::Listing> {
        self.node(u).inbound_listings().unwrap()
    }

    /// The titles `u` has discovered, sorted — the terse form most assertions want.
    pub fn discovered_titles(&self, u: usize) -> Vec<String> {
        let mut v: Vec<String> = self.discovered(u).into_iter().map(|l| l.title).collect();
        v.sort();
        v
    }

    // ---- profile (self-published, fanned out to every connection) ----

    /// User `u` edits their profile — the real `set_my_profile` path, so it persists
    /// locally AND fans a signed `contact.publishProfile` delta into every connection
    /// they hold. Returns how many connections it published into.
    pub async fn set_profile(&self, u: usize, display_name: &str, card: &ContactCard) -> usize {
        self.node(u)
            .set_my_profile(display_name, GroupShape::Individual, card)
            .await
            .unwrap()
    }

    /// [`set_profile`] against an explicit relay — pass a dead URL to model editing
    /// your profile with no network, and prove the edit is still durable.
    pub async fn set_profile_via(
        &self,
        u: usize,
        display_name: &str,
        card: &ContactCard,
        _relay: &str,
    ) -> usize {
        self.node(u)
            .set_my_profile(display_name, GroupShape::Individual, card)
            .await
            .unwrap()
    }

    /// The profile user `u` currently holds for `peer` — folded from the signed
    /// deltas `peer` published on their shared Contact channel. None until `peer`
    /// has published one.
    pub fn peer_profile(&self, u: usize, peer: usize) -> Option<SelfProfile> {
        self.node(u).peer_profile(&self.users[peer].id).unwrap()
    }

    /// `u`'s own stored profile: (display name, shape, card).
    pub fn my_profile(&self, u: usize) -> (String, GroupShape, ContactCard) {
        self.node(u).my_profile().unwrap()
    }

    /// The display name `u` has cached for `peer` in the connections list — the
    /// projection every name-rendering surface reads.
    pub fn connection_name(&self, u: usize, peer: usize) -> String {
        self.node(u)
            .connections()
            .unwrap()
            .into_iter()
            .find(|(pk, _, _)| pk == &self.users[peer].id)
            .map(|(_, name, _)| name)
            .unwrap_or_default()
    }

    // ---- assertions & lookups ----

    /// Assert every listed user sees a byte-identical folded view of forum `obj`,
    /// and return that agreed view. The core convergence proof.
    ///
    /// The `receipt` code is DELIBERATELY excluded from the equality: it is the
    /// sender's own ✓✓ state (delivered/read by the other members), so it is
    /// perspective-relative — each device fills it for its OWN messages only, and
    /// two devices are never expected to agree on it. Everything else (author, gen,
    /// text, ts, reply linkage, reactions) is the replica-invariant content.
    pub fn assert_forum_converges(&self, obj: &str, us: &[usize]) -> Vec<ForumMessage> {
        let mut agreed: Option<Vec<ForumMessage>> = None;
        for &u in us {
            let v = self.obj_view(u, obj);
            match &agreed {
                None => agreed = Some(v),
                Some(base) => assert_eq!(
                    content(&v),
                    content(base),
                    "user {u} ('{}') diverged on forum {obj}",
                    self.users[u].name
                ),
            }
        }
        agreed.expect("at least one user")
    }

    /// Assert both sides of a DM see the same folded view (content only — `receipt`
    /// is sender-relative; see `assert_forum_converges`).
    pub fn assert_dm_converges(&self, a: usize, b: usize) -> Vec<ForumMessage> {
        let va = self.dm_view(a, b);
        let vb = self.dm_view(b, a);
        assert_eq!(content(&va), content(&vb), "DM {a}<->{b} diverged");
        va
    }

    /// The `MsgRef` of the first forum message whose text matches, as user `u` sees
    /// it — for targeting a reaction/reply.
    pub fn find_forum(&self, u: usize, obj: &str, text: &str) -> MsgRef {
        let v = self.obj_view(u, obj);
        let m = v
            .iter()
            .find(|m| m.text == text)
            .unwrap_or_else(|| panic!("user {u} has no forum message {text:?} in {obj}"));
        (m.author, m.gen)
    }
    /// The `MsgRef` of the first DM message whose text matches, as user `u` sees it.
    pub fn find_dm(&self, u: usize, peer: usize, text: &str) -> MsgRef {
        let v = self.dm_view(u, peer);
        let m = v
            .iter()
            .find(|m| m.text == text)
            .unwrap_or_else(|| panic!("user {u} has no DM message {text:?} with {peer}"));
        (m.author, m.gen)
    }
}
