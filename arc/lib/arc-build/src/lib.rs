//! arc-build — what commit is this binary?
//!
//! The Arc does not ship an image. `hosting/deploy-arc.sh` cross-compiles on a laptop and scps
//! five binaries to `/opt/arc/bin/`, so there is no registry, no tag and no OCI label to carry
//! provenance. Before this crate there was nothing at all: the binaries running on kenjin-01 on
//! 18 Sep 2026 were dated 10 Sep and matched NO commit in this repository — the arc log jumps
//! 13 Aug → 16 Sep — because they were built from somebody's working tree on a day nobody wrote
//! down. What was live could only be established by probing the wire for behaviour.
//!
//! This is the same argument `site/justfile` makes for the site ("A deploy ships the
//! COMMIT, not whatever happens to be in the working tree"), carried to a binary: the commit is
//! compiled INTO each plane, so the binary answers for itself rather than being vouched for by a
//! file next to it that anyone could have written.
//!
//! ## Three states, not two
//!
//! * **stamped, clean** — `commit` names a real commit and the tree matched it. The binary is
//!   that commit.
//! * **stamped, dirty** — `commit` names a commit and `dirty:true` says the tree had uncommitted
//!   changes on top. The commit is a LOWER BOUND on what is in here, not a description of it.
//! * **unstamped** — a plain `cargo build`. `commit` is the string `unknown` and every rendering
//!   says so loudly, in the house style: a thing that is not the real article announces itself.
//!   This is the normal state for a local dev build and it is not an error.
//!
//! ## `core` is not decoration
//!
//! The workspace `[patch]` in `arc/Cargo.toml` redirects `pacific-core` and `pacific-wire` at the
//! SIBLING checkout `../core`, which is a separate repository with its own heavy uncommitted
//! work. So the arc commit alone does not describe these binaries — roughly half of `node` comes
//! from a tree the arc SHA cannot name. `core` records that checkout's commit and dirtiness
//! (`<sha>` or `<sha>-dirty`) so the stamp does not quietly overclaim.
//!
//! ## Reading it back
//!
//! * over the wire — `GET /v1/version` on the gateway
//! * on the box, from the binary itself — `/opt/arc/bin/<plane> --version`
//! * on the box, without running it — `strings /opt/arc/bin/<plane> | grep '@(#)arc-build'`
//! * on the box, for all five at once — `/opt/arc/build.json`, which the deploy writes by
//!   ASKING each swapped-in binary `--version` rather than by asserting what it shipped.

include!(concat!(env!("OUT_DIR"), "/stamp.rs"));

/// Keeps [`MARKER`] in the binary's rodata even in a plane that never renders it, so
/// `strings`/`grep` works on every stamped artefact and not only the talkative ones.
#[used]
static MARKER_ANCHOR: &str = MARKER;

/// Minimal JSON string escaping. This crate has no serde on purpose (see Cargo.toml), and the
/// values are git refs and timestamps, so the mandatory escapes are the whole job.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// This plane's stamp as a JSON object.
///
/// Field names follow `site`'s `/version.json` (`commit`, `ref`, `dirty`, `built`) so the
/// two deploys answer "what is live?" in the same vocabulary, plus `plane`, `core` and `stamped`,
/// which the site has no need of.
pub fn json(plane: &str) -> String {
    format!(
        "{{\"plane\":\"{}\",\"commit\":\"{}\",\"short\":\"{}\",\"ref\":\"{}\",\"dirty\":{},\
         \"core\":\"{}\",\"built\":\"{}\",\"stamped\":{}}}",
        esc(plane),
        esc(COMMIT),
        esc(SHORT),
        esc(REF),
        DIRTY,
        esc(CORE),
        esc(BUILT),
        STAMPED,
    )
}

/// One human line for the process log. An unstamped or dirty build says so in words, not by the
/// absence of something the reader would have to notice.
pub fn line(plane: &str) -> String {
    if !STAMPED {
        return format!(
            "{plane}: UNSTAMPED BUILD — this binary does not know which commit it is. \
             Built outside its deploy script (a plain `cargo build` or an image build); \
             fine locally, never trust it on a box."
        );
    }
    let dirty = if DIRTY { " +DIRTY-TREE (commit is a lower bound, not a description)" } else { "" };
    format!("{plane}: build {SHORT} ({REF}){dirty} core={CORE} built={BUILT}")
}

/// Handle `--version`/`-V` — print [`json`] to stdout and exit 0 — otherwise log [`line`] to
/// stderr and return.
///
/// It exits the process, which a library function normally has no business doing; that IS what
/// `--version` means, and putting it in one place is what stops five planes from each inventing
/// their own spelling of it. Call it as the FIRST thing in `main`, before opening databases or
/// minting identities, so asking a binary what it is has no side effects.
pub fn stamp_as(plane: &str) {
    if std::env::args().skip(1).any(|a| a == "--version" || a == "-V") {
        println!("{}", json(plane));
        std::process::exit(0);
    }
    eprintln!("{}", line(plane));
}

/// [`stamp_as`] with the binary naming itself. `CARGO_BIN_NAME` expands at the CALL site, so the
/// name is read from cargo rather than transcribed into five `main`s where it could rot.
#[macro_export]
macro_rules! stamp {
    () => {
        $crate::stamp_as(env!("CARGO_BIN_NAME"))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stamp must never be silently absent. Whichever state this test build is in, the
    /// rendering has to say which one — that is the entire contract.
    #[test]
    fn every_state_announces_itself() {
        let l = line("plane");
        if STAMPED {
            assert_ne!(COMMIT, "unknown", "a stamped build must carry a real commit");
            assert!(l.contains(SHORT), "the log line must name the commit: {l}");
            assert_eq!(DIRTY, l.contains("DIRTY"), "dirty must be stated when true, absent when not");
        } else {
            assert_eq!(COMMIT, "unknown");
            assert!(l.contains("UNSTAMPED"), "an unstamped build must say so: {l}");
        }
    }

    /// `strings <binary> | grep '@(#)arc-build'` is the read path that needs no running process,
    /// so the marker must carry the same commit the JSON does.
    #[test]
    fn marker_and_json_agree() {
        assert!(MARKER.starts_with("@(#)arc-build "), "marker prefix is the grep key: {MARKER}");
        assert!(MARKER.contains(&format!("commit={COMMIT}")));
        assert!(MARKER.contains(&format!("dirty={DIRTY}")));
        assert!(json("p").contains(&format!("\"commit\":\"{COMMIT}\"")));
    }

    /// The JSON is hand-rolled, so the escaping is the part that can produce a body no client can
    /// parse. A branch name with a quote in it must not be able to do that.
    #[test]
    fn json_escapes_its_inputs() {
        assert_eq!(esc("a\"b\\c"), "a\\\"b\\\\c");
        assert_eq!(esc("a\nb"), "a\\nb");
        assert_eq!(esc("\u{1}"), "\\u0001");
        assert!(json("pl\"ane").contains("\"plane\":\"pl\\\"ane\""));
    }

    /// `plane` is the only field a caller supplies, and the five deployed binaries must be
    /// distinguishable in the deploy manifest.
    #[test]
    fn json_names_the_plane() {
        assert!(json("arc-gateway").starts_with("{\"plane\":\"arc-gateway\","));
        assert!(json("semaphore").contains("\"plane\":\"semaphore\""));
    }
}
