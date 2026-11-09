//! THE ICD'S NUMBERS, READ AT BUILD (the doctrine, parametric modelling: "numbers, counts,
//! operations, should be read at build/runtime directly from the ICD, because copies break").
//! Each number the code needs is read here from coordination/delta-graph.icd.json and emitted as
//! a const into OUT_DIR (`icd_consts.rs`), so the ICD is the one place it is written, and only
//! the number, not the whole model, reaches the binaries that link core (the webapp's wasm, iOS).

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let path = std::path::Path::new(&manifest).join("../coordination/delta-graph.icd.json");
    println!("cargo:rerun-if-changed={}", path.display());
    let icd: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("the ICD, beside pacific-core"))
        .expect("the ICD is JSON");
    let history_cap = icd["mls"]["intro"]["kinds"]["history"]["cap"]
        .as_u64()
        .expect("the ICD states mls.intro.kinds.history.cap");
    let cards_total = icd["mls"]["intro"]["kinds"]["history"]["cardsTotal"]
        .as_u64()
        .expect("the ICD states mls.intro.kinds.history.cardsTotal");
    // W-98 Rooms: the room's description, its op id and its cap.
    let describe = &icd["kinds"]["forum"]["ops"]["forum.editDescription"];
    let describe_op = describe["op"].as_u64().expect("the ICD declares forum.editDescription");
    let describe_max = describe["args"]["description"]["maxBytes"]
        .as_u64()
        .expect("the ICD states forum.editDescription's description.maxBytes");
    // The owner's one order: the slot a sequenced delta claims before it is sent.
    let slot = &icd["mls"]["sequencing"]["slot"];
    let slot_label = slot["label"].as_str().expect("the ICD states mls.sequencing.slot.label");
    let slot_rounds = slot["rounds"].as_u64().expect("the ICD states mls.sequencing.slot.rounds");
    // How a detached media object is sealed (media.detached, #56): the construction is seal.rs's,
    // so the ICD's cipher must be the one the code runs.
    let media = &icd["media"]["detached"];
    assert_eq!(media["cipher"].as_str(), Some("XChaCha20-Poly1305"), "media.detached.cipher is seal.rs's");
    let media_key = media["keyBytes"].as_u64().expect("the ICD states media.detached.keyBytes");
    let media_nonce = media["nonceBytes"].as_u64().expect("the ICD states media.detached.nonceBytes");
    let out = std::path::Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("icd_consts.rs");
    std::fs::write(
        out,
        format!(
            "/// `mls.intro.kinds.history.cap`, from the ICD at build.\npub const HISTORY_CAP: usize = {history_cap};\n\
             /// `mls.intro.kinds.history.cardsTotal`, from the ICD at build.\npub const HISTORY_CARDS_TOTAL: usize = {cards_total};\n\
             /// `kinds.forum.ops.\"forum.editDescription\".op`, from the ICD at build.\npub const FORUM_EDIT_DESCRIPTION: u32 = {describe_op};\n\
             /// `kinds.forum.ops.\"forum.editDescription\".args.description.maxBytes`, from the ICD at build.\npub const FORUM_DESCRIPTION_MAX: usize = {describe_max};\n\
             /// `mls.sequencing.slot.label`, from the ICD at build.\npub const SEQ_SLOT_LABEL: &str = {slot_label:?};\n\
             /// `mls.sequencing.slot.rounds`, from the ICD at build.\npub const SEQ_SLOT_ROUNDS: usize = {slot_rounds};\n\
             /// `media.detached.keyBytes`, from the ICD at build.\npub const MEDIA_KEY_BYTES: usize = {media_key};\n\
             /// `media.detached.nonceBytes`, from the ICD at build.\npub const MEDIA_NONCE_BYTES: usize = {media_nonce};\n"
        ),
    )
    .expect("icd_consts.rs");
    w98(&icd);
    links(&icd);
    vocabularies(&icd);
    exterior(&icd);
}

/// EVERY CLOSED VOCABULARY AND CEILING, into OUT_DIR/icd_vocabularies.rs: `(op, arg, words)`
/// and `(op, arg, maxLength)` for each op arg, on a kind or a facet, that states one — what a
/// UI offers for a choice and how long a value may be, so no surface keeps its own copy.
fn vocabularies(icd: &serde_json::Value) {
    let mut words = std::collections::BTreeMap::new();
    let mut ceilings = std::collections::BTreeMap::new();
    for section in ["kinds", "facets"] {
        for holder in icd[section].as_object().expect("the ICD's kinds and facets").values() {
            for (op, decl) in holder["ops"].as_object().into_iter().flatten() {
                for (arg, a) in decl["args"].as_object().into_iter().flatten() {
                    if let Some(v) = a["vocabulary"].as_object() {
                        let w: Vec<String> = v.keys().map(|k| format!("{k:?}")).collect();
                        words.insert((op.clone(), arg.clone()), w.join(", "));
                    }
                    if let Some(n) = a["maxLength"].as_u64() {
                        ceilings.insert((op.clone(), arg.clone()), n);
                    }
                }
            }
        }
    }
    let words: Vec<String> = words.iter().map(|((op, arg), w)| format!("    ({op:?}, {arg:?}, &[{w}]),")).collect();
    let ceilings: Vec<String> = ceilings.iter().map(|((op, arg), n)| format!("    ({op:?}, {arg:?}, {n}),")).collect();
    let out = std::path::Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("icd_vocabularies.rs");
    std::fs::write(
        out,
        format!(
            "/// Every op arg's closed vocabulary, `(op, arg, words)`, from the ICD at build.\n\
             pub const VOCABULARIES: &[(&str, &str, &[&str])] = &[\n{}\n];\n\
             /// Every op arg's `maxLength`, `(op, arg, n)`, from the ICD at build.\n\
             pub const MAX_LENGTHS: &[(&str, &str, u64)] = &[\n{}\n];\n",
            words.join("\n"),
            ceilings.join("\n")
        ),
    )
    .expect("icd_vocabularies.rs");
}

/// W-98's numbers and words (EVENTS-CORE), into OUT_DIR/icd_w98.rs: the per-kind visibility
/// defaults, each media arg's inline ceiling, the lineup's size, and the closed vocabularies the
/// event and post folds accept. Each is where the ICD states it; nothing here restates one.
fn w98(icd: &serde_json::Value) {
    let op = |kind: &str, op: &str| &icd["kinds"][kind]["ops"][op];
    let max_len = |kind: &str, o: &str, arg: &str| {
        op(kind, o)["args"][arg]["maxLength"].as_u64().unwrap_or_else(|| panic!("the ICD states {o}.{arg}'s maxLength"))
    };
    let facet_max = |o: &str, arg: &str| {
        icd["facets"]["media"]["ops"][o]["args"][arg]["maxLength"].as_u64().unwrap_or_else(|| panic!("the ICD states {o}.{arg}'s maxLength"))
    };
    let words = |v: &serde_json::Value, what: &str| -> String {
        let w: Vec<String> = v.as_object().unwrap_or_else(|| panic!("the ICD states {what}'s vocabulary")).keys().map(|k| format!("{k:?}")).collect();
        format!("&[{}]", w.join(", "))
    };
    let defaults: Vec<String> = icd["facets"]["visibility"]["defaults"]
        .as_object()
        .expect("the ICD states facets.visibility.defaults")
        .iter()
        .map(|(k, v)| format!("({k:?}, {:?})", v.as_str().expect("a visibility word")))
        .collect();
    let acts = &op("event", "event.setLineup")["args"]["acts"];
    let live = |kind: &str, o: &str| op(kind, o)["maxLive"].as_u64().unwrap_or_else(|| panic!("the ICD states {o}'s maxLive"));
    let out = std::path::Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("icd_w98.rs");
    std::fs::write(
        out,
        format!(
            "/// `facets.visibility.defaults`: a kind's visibility when none was written.\n\
             pub const VISIBILITY_DEFAULTS: &[(&str, &str)] = &[{}];\n\
             /// Inline ceilings, base64 characters: each arg's `maxLength`.\n\
             pub const EVENT_BANNER_MAX: usize = {};\npub const EVENT_PHOTO_MAX: usize = {};\npub const EVENT_CLIP_MAX: usize = {};\n\
             pub const POST_DOCUMENT_MAX: usize = {};\npub const POST_ASSET_MAX: usize = {};\n\
             pub const MEDIA_ICON_MAX: usize = {};\npub const MEDIA_BANNER_MAX: usize = {};\n\
             /// `event.setLineup` acts: `maxItems`.\npub const EVENT_ACTS_MAX: usize = {};\n\
             /// Live members of a keyed set: the op's `maxLive`.\npub const EVENT_PHOTOS_LIVE: usize = {};\npub const POST_ASSETS_LIVE: usize = {};\n\
             /// Text ceilings, characters: each arg's `maxLength`.\n\
             pub const EVENT_TZ_MAX: usize = {};\npub const EVENT_TICKET_URL_MAX: usize = {};\npub const EVENT_ONLINE_MAX: usize = {};\npub const EVENT_VIDEO_URL_MAX: usize = {};\n\
             pub const POST_EXCERPT_MAX: usize = {};\npub const POST_BANNER_ALT_MAX: usize = {};\npub const POST_ALT_MAX: usize = {};\npub const POST_NAME_MAX: usize = {};\n\
             /// Vocabularies, as the ICD names them.\n\
             pub const EVENT_STATUS: &[&str] = {};\npub const EVENT_DESCRIPTOR_FORMATS: &[&str] = {};\n\
             pub const EVENT_ACT_ROLES: &[&str] = {};\npub const POST_BODY_FORMATS: &[&str] = {};\n",
            defaults.join(", "),
            max_len("event", "event.setBanner", "banner"),
            max_len("event", "event.addPhoto", "photo"),
            max_len("event", "event.setClip", "clip"),
            max_len("post", "post.setDocument", "document"),
            max_len("post", "post.addAsset", "asset"),
            facet_max("base.setIcon", "icon"),
            facet_max("base.setBanner", "banner"),
            acts["maxItems"].as_u64().expect("the ICD states setLineup's acts maxItems"),
            live("event", "event.addPhoto"),
            live("post", "post.addAsset"),
            max_len("event", "event.setProfile", "tz"),
            max_len("event", "event.setProfile", "ticketUrl"),
            max_len("event", "event.setProfile", "online"),
            max_len("event", "event.setProfile", "videoUrl"),
            max_len("post", "post.setProfile", "excerpt"),
            max_len("post", "post.setMedia", "bannerAlt"),
            max_len("post", "post.addAsset", "alt"),
            max_len("post", "post.setDocument", "name"),
            words(&op("event", "event.setProfile")["args"]["status"]["vocabulary"], "status"),
            words(&op("event", "event.setProfile")["args"]["descriptorFormat"]["vocabulary"], "descriptorFormat"),
            words(&acts["items"]["role"]["vocabulary"], "an act's role"),
            words(&op("post", "post.setProfile")["args"]["bodyFormat"]["vocabulary"], "bodyFormat"),
        ),
    )
    .expect("icd_w98.rs");
}

/// W-98 LINKS: `contact.setLink`'s op id, into its own `icd_links.rs`.
fn links(icd: &serde_json::Value) {
    let set_link = icd["kinds"]["contact"]["ops"]["contact.setLink"]["op"]
        .as_u64()
        .expect("the ICD states kinds.contact.ops.contact.setLink.op");
    let out = std::path::Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("icd_links.rs");
    std::fs::write(
        out,
        format!("/// `kinds.contact.ops.\"contact.setLink\".op`, from the ICD at build.\npub const OP_SET_LINK: u32 = {set_link};\n"),
    )
    .expect("icd_links.rs");
}

/// THE EXTERIOR (#76, ICD 2.6.0): the facet's op ids, caps and words, the leg's op id, the
/// asker's request op, and `mls.door`, into `icd_exterior.rs`.
fn exterior(icd: &serde_json::Value) {
    let facet = &icd["facets"]["exterior"]["ops"];
    let op = |v: &serde_json::Value, what: &str| v["op"].as_u64().unwrap_or_else(|| panic!("the ICD states {what}'s op"));
    let max = |o: &str, arg: &str| {
        facet[o]["args"][arg]["maxLength"].as_u64().unwrap_or_else(|| panic!("the ICD states {o}.{arg}'s maxLength"))
    };
    let words = |v: &serde_json::Value, what: &str| -> String {
        let w: Vec<String> = v.as_object().unwrap_or_else(|| panic!("the ICD states {what}'s vocabulary")).keys().map(|k| format!("{k:?}")).collect();
        format!("&[{}]", w.join(", "))
    };
    let on: Vec<String> = icd["facets"]["exterior"]["on"]
        .as_array()
        .expect("the ICD states facets.exterior.on")
        .iter()
        .map(|k| format!("{:?}", k.as_str().expect("a kind")))
        .collect();
    let door = &icd["mls"]["door"];
    let text = |k: &str| door[k].as_str().unwrap_or_else(|| panic!("the ICD states mls.door.{k}")).to_string();
    let num = |k: &str| door[k].as_u64().unwrap_or_else(|| panic!("the ICD states mls.door.{k}"));
    let out = std::path::Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("icd_exterior.rs");
    std::fs::write(
        out,
        format!(
            "/// `facets.exterior.on`: the kinds that have an exterior.\npub const EXTERIOR_KINDS: &[&str] = &[{}];\n\
             /// `facets.exterior` op ids.\n\
             pub const OP_SET_EXTERIOR: u32 = {};\npub const OP_SET_DOOR: u32 = {};\npub const OP_DECIDE_REQUEST: u32 = {};\n\
             /// `contact.publishExterior`'s and `group.setRequest`'s op ids, and the leg's `perSync`.\n\
             pub const OP_PUBLISH_EXTERIOR: u32 = {};\npub const OP_SET_REQUEST: u32 = {};\npub const CARRIES_PER_SYNC: usize = {};\n\
             /// `base.setExterior` caps: each arg's `maxLength`.\n\
             pub const NAME_MAX: usize = {};\npub const DESCRIPTION_MAX: usize = {};\npub const PHOTO_MAX: usize = {};\n\
             /// Vocabularies, as the ICD names them.\n\
             pub const DOOR_MODES: &[&str] = {};\npub const DECISIONS: &[&str] = {};\n\
             /// `mls.door`.\n\
             pub const DOOR_ADDRESS_DOMAIN: &str = {:?};\npub const DOOR_SEAL_DOMAIN: &str = {:?};\npub const DOOR_SIG_DOMAIN: &str = {:?};\n\
             pub const DOOR_BUNDLES: usize = {};\npub const DOOR_NOTE_MAX: usize = {};\n",
            on.join(", "),
            op(&facet["base.setExterior"], "base.setExterior"),
            op(&facet["base.setDoor"], "base.setDoor"),
            op(&facet["base.decideRequest"], "base.decideRequest"),
            op(&icd["kinds"]["contact"]["ops"]["contact.publishExterior"], "contact.publishExterior"),
            op(&icd["kinds"]["group"]["ops"]["group.setRequest"], "group.setRequest"),
            icd["kinds"]["contact"]["ops"]["contact.publishExterior"]["perSync"].as_u64().expect("the ICD states contact.publishExterior's perSync"),
            max("base.setExterior", "name"),
            max("base.setExterior", "description"),
            max("base.setExterior", "photo"),
            words(&facet["base.setDoor"]["args"]["mode"]["vocabulary"], "base.setDoor's mode"),
            words(&facet["base.decideRequest"]["args"]["decision"]["vocabulary"], "base.decideRequest's decision"),
            text("addressDomain"),
            text("sealDomain"),
            text("sigDomain"),
            num("bundles"),
            num("noteMax"),
        ),
    )
    .expect("icd_exterior.rs");
}
