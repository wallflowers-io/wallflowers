//! The demo as a SET OF OPERATIONS between users — one source of truth.
//!
//! Per the founder's framing: the demo must not be data we *display*, it must be
//! the residue of real operations flowing through the actual pipe (identity →
//! deltas → MLS-encrypt → seal → relay → folded projection). This module holds
//! the canonical script and the executor that runs it through the real `Node`.
//! BOTH the headless fitness test and the app (via pacific-ffi) call the same
//! `seed_dm_script`, so what the UI shows is provably reconstructable through
//! real ops. When we can rebuild demo-equivalent complexity through this route,
//! the system works.
//!
//! Ordering: posts are stamped with a LAMPORT clock (node.rs::next_lamport) and
//! the transcript sorts by (gen=lamport, author), so a message sorts after
//! everything its author had seen — causal send order. Two posts that never saw
//! each other are genuinely concurrent and fall back to the deterministic
//! author-bytes tiebreak.

use crate::node::Node;
use crate::CoreError;

/// One scripted line: which ACTOR authors it, and the text. `actor` is a stable
/// logical handle ("nye"/"harvey"); each device maps its own identity to one.
pub struct Line {
    pub actor: &'static str,
    pub text: &'static str,
}

/// A scripted 1:1 conversation between two actors.
pub struct DmScript {
    pub a: &'static str,
    pub b: &'static str,
    pub lines: &'static [Line],
}

impl DmScript {
    /// The lines THIS actor authors (the rest arrive over the relay from the peer).
    pub fn my_lines<'a>(&'a self, actor: &'a str) -> impl Iterator<Item = &'a Line> + 'a {
        self.lines.iter().filter(move |l| l.actor == actor)
    }
    pub fn is_actor(&self, actor: &str) -> bool {
        actor == self.a || actor == self.b
    }
}

/// The Nye↔Harvey Drop-1 tagging thread (the same content the mockup showed —
/// now expressed as operations, authored by whichever device is that actor).
pub const NYE_HARVEY: DmScript = DmScript {
    a: "nye",
    b: "harvey",
    lines: &[
        Line { actor: "harvey", text: "morning — pulling Drop 1 stock for Tuesday's shoot and the tags don't match the rail" },
        Line { actor: "nye",    text: "same failure as the sale? NP30211 got stamped on the coral shorts, NP30222 doesn't even exist in the catalogue" },
        Line { actor: "harvey", text: "exactly the same shape. found two more — NP20033.06 and NP20009.06 are swapped in the sheet" },
        Line { actor: "nye",    text: "this is the exact case for Tom's API — tags come off Sysiphus once, nobody re-types codes" },
        Line { actor: "harvey", text: "Bea's untangling with the design office. shoot list is fine, it's just the codes" },
    ],
};

/// Resolve an app display name to a script actor handle ("Nye Thompson" → "nye").
/// Returns None if this identity isn't a scripted actor (then it seeds nothing).
pub fn actor_for_display_name(name: &str) -> Option<&'static str> {
    match name
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "nye" => Some("nye"),
        "harvey" => Some("harvey"),
        _ => None,
    }
}

impl Node {
    /// Execute THIS device's half of a scripted DM with `peer`: author each line
    /// whose actor is `my_actor` (a real `author_post` Delta → relay), then drain
    /// so the peer's lines fold in. Requires the connection to `peer` to already
    /// be Connected (pairing done).
    ///
    /// RUN ONCE per install: posts carry a per-author generation counter, so
    /// re-running would author fresh (higher-gen) duplicates. The caller guards
    /// it (the app's ChatSeed "seeded" flag; the headless test runs it once).
    pub async fn seed_dm_script(
        &self,
        peer_id: &[u8; 32],
        script: &DmScript,
        my_actor: &str,
    ) -> Result<(), CoreError> {
        for line in script.my_lines(my_actor) {
            self.author_post(peer_id, line.text).await?;
        }
        self.sync_once().await?;
        Ok(())
    }
}
