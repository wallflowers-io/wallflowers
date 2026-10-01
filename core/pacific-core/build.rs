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
    let out = std::path::Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("icd_consts.rs");
    std::fs::write(
        out,
        format!(
            "/// `mls.intro.kinds.history.cap`, from the ICD at build.\npub const HISTORY_CAP: usize = {history_cap};\n\
             /// `mls.intro.kinds.history.cardsTotal`, from the ICD at build.\npub const HISTORY_CARDS_TOTAL: usize = {cards_total};\n\
             /// `kinds.forum.ops.\"forum.editDescription\".op`, from the ICD at build.\npub const FORUM_EDIT_DESCRIPTION: u32 = {describe_op};\n\
             /// `kinds.forum.ops.\"forum.editDescription\".args.description.maxBytes`, from the ICD at build.\npub const FORUM_DESCRIPTION_MAX: usize = {describe_max};\n"
        ),
    )
    .expect("icd_consts.rs");
    w98(&icd);
    links(&icd);
}

/// W-98's numbers and words (EVENTS-CORE), into OUT_DIR/icd_w98.rs: the per-kind visibility
/// defaults, each media arg's inline ceiling, the lineup's size, and the closed vocabularies the
/// event and post folds accept. Each is where the ICD states it; nothing here restates one.
fn w98(icd: &serde_json::Value) {
    let op = |kind: &str, op: &str| &icd["kinds"][kind]["ops"][op];
    let max_len = |kind: &str, o: &str, arg: &str| {
        op(kind, o)["args"][arg]["maxLength"].as_u64().unwrap_or_else(|| panic!("the ICD states {o}.{arg}'s maxLength"))
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
