//! The AW26 Project demo as a SET OF REAL OPERATIONS — the Work-tab analogue of
//! `demo.rs`'s DM script. A two-person slice of the Aries AW26 season, expressed
//! as real Project deltas: the OWNER authors the owner-sequenced structure
//! (configure / timeline items / schedule / assignees / a `blocks` dependency),
//! and EACH actor authors its own any-member commutative progress. Both devices
//! fold to the identical board — so the mockup's content is provably
//! reconstructable through the real pipe (identity → deltas → MLS → relay → fold).
//!
//! (Kept in its own module, not `demo.rs`, so the Project seed and the DM seed can
//! evolve independently.)
//!
//! The crossed-codes "tagging" pain point is modelled as a real `blocks`
//! dependency (tagging → shoot), so the shoot's `blocked` state is DERIVED from
//! the graph, never asserted — exactly the reducer's honest-by-default rule.

use crate::coordinator::{ArgVal, Args};
use crate::node::Node;
use crate::project as p;
use crate::CoreError;

/// One step of a scripted Project. Structure steps are authored by the OWNER only
/// (owner-sequenced); `Progress` is authored by the named actor (any member).
pub enum ProjectStep {
    Configure {
        title: &'static str,
        status: &'static str,
    },
    Item {
        id: &'static str,
        kind: &'static str,
        title: &'static str,
    },
    Schedule {
        id: &'static str,
        at: i64,
    },
    Assign {
        item: &'static str,
        actor: &'static str,
    },
    Dep {
        edge: &'static str,
        from: &'static str,
        to: &'static str,
    },
    Progress {
        actor: &'static str,
        item: &'static str,
        status: &'static str,
    },
}

/// A scripted Project between two actors. `owner` authors all structure; both
/// actors author their own progress. Actor handles ("nye"/"harvey") resolve to
/// real member ids in the executor.
pub struct ProjectScript {
    pub owner: &'static str,
    pub steps: &'static [ProjectStep],
}

// Fixed AW26 schedule instants (epoch-seconds, no clock read).
const AT_TAGGING: i64 = 1_752_019_200; // ~9 Jul 2026
const AT_SHOOT: i64 = 1_752_451_200; //   ~14 Jul 2026
const AT_LAUNCH: i64 = 1_756_339_200; //  ~28 Aug 2026

/// The AW26 Drop-1 two-person slice (Nye = owner/ecomm, Harvey = ecomm). The
/// tagging task blocks the flat shoot (crossed codes), so the shoot is DERIVED
/// blocked until tagging is done.
pub const AW26_PROJECT: ProjectScript = ProjectScript {
    owner: "nye",
    steps: &[
        ProjectStep::Configure {
            title: "AW26 · No Problemo",
            status: "active",
        },
        ProjectStep::Item {
            id: "tagging",
            kind: "action",
            title: "Tagging — Drop 1 (crossed codes)",
        },
        ProjectStep::Item {
            id: "shoot",
            kind: "action",
            title: "Flat shoot — day 1",
        },
        ProjectStep::Item {
            id: "launch",
            kind: "event",
            title: "AW26 Drop 1 launch",
        },
        ProjectStep::Schedule {
            id: "tagging",
            at: AT_TAGGING,
        },
        ProjectStep::Schedule {
            id: "shoot",
            at: AT_SHOOT,
        },
        ProjectStep::Schedule {
            id: "launch",
            at: AT_LAUNCH,
        },
        ProjectStep::Assign {
            item: "tagging",
            actor: "harvey",
        },
        ProjectStep::Assign {
            item: "shoot",
            actor: "nye",
        },
        // the crossed-codes tagging blocks the shoot until it is done.
        ProjectStep::Dep {
            edge: "tag-blocks-shoot",
            from: "tagging",
            to: "shoot",
        },
        // each actor reports its own progress (any-member, commutative).
        ProjectStep::Progress {
            actor: "harvey",
            item: "tagging",
            status: "in_progress",
        },
        ProjectStep::Progress {
            actor: "nye",
            item: "shoot",
            status: "open",
        },
    ],
};

impl Node {
    /// Execute THIS device's half of a scripted Project: if we are the owner,
    /// author every owner-sequenced structure step; regardless, author the progress
    /// steps whose actor is `my_actor`. Assignee actors resolve to real member ids
    /// (`my_actor` → us, `peer_actor` → `peer_id`). Ends with a `sync`, so authored
    /// deltas flush and inbound peer deltas fold in.
    ///
    /// RUN ONCE per install (owner-sequenced ops dedup a duplicate `itemId`, and
    /// `gen` is monotone) — the caller still guards it, like the DM seed.
    pub async fn seed_project_script(
        &self,
        object_id_hex: &str,
        script: &ProjectScript,
        my_actor: &str,
        peer_actor: &str,
        peer_id: &[u8; 32],
    ) -> Result<(), CoreError> {
        let me_hex = hex::encode(self.id.identity_pk());
        let peer_hex = hex::encode(peer_id);
        let member_hex = |actor: &str| -> String {
            if actor == my_actor {
                me_hex.clone()
            } else if actor == peer_actor {
                peer_hex.clone()
            } else {
                String::new() // unknown actor — setAssignee rejects loudly at fold
            }
        };
        let is_owner = my_actor == script.owner;

        for step in script.steps {
            match step {
                ProjectStep::Configure { title, status } if is_owner => {
                    let mut a = Args::new();
                    a.insert("title".into(), ArgVal::Text((*title).into()));
                    a.insert("status".into(), ArgVal::Text((*status).into()));
                    self.apply(object_id_hex, p::OP_CONFIGURE, a)
                        .await?;
                }
                ProjectStep::Item { id, kind, title } if is_owner => {
                    let mut a = Args::new();
                    a.insert("itemId".into(), ArgVal::Text((*id).into()));
                    a.insert("kind".into(), ArgVal::Text((*kind).into()));
                    a.insert("title".into(), ArgVal::Text((*title).into()));
                    self.apply(object_id_hex, p::OP_ADD_TIMELINE_ITEM, a)
                        .await?;
                }
                ProjectStep::Schedule { id, at } if is_owner => {
                    let mut a = Args::new();
                    a.insert("itemId".into(), ArgVal::Text((*id).into()));
                    a.insert("at".into(), ArgVal::Int(*at));
                    self.apply(object_id_hex, p::OP_SET_ITEM_SCHEDULE, a)
                        .await?;
                }
                ProjectStep::Assign { item, actor } if is_owner => {
                    let mut a = Args::new();
                    a.insert("itemId".into(), ArgVal::Text((*item).into()));
                    a.insert("member".into(), ArgVal::Text(member_hex(actor)));
                    a.insert("assigned".into(), ArgVal::Int(1));
                    self.apply(object_id_hex, p::OP_SET_ASSIGNEE, a)
                        .await?;
                }
                ProjectStep::Dep { edge, from, to } if is_owner => {
                    let mut a = Args::new();
                    a.insert("edgeId".into(), ArgVal::Text((*edge).into()));
                    a.insert("from".into(), ArgVal::Text((*from).into()));
                    a.insert("to".into(), ArgVal::Text((*to).into()));
                    a.insert("kind".into(), ArgVal::Text("blocks".into()));
                    self.apply(object_id_hex, p::OP_ADD_DEPENDENCY, a)
                        .await?;
                }
                ProjectStep::Progress {
                    actor,
                    item,
                    status,
                } if *actor == my_actor => {
                    let mut a = Args::new();
                    a.insert("itemId".into(), ArgVal::Text((*item).into()));
                    a.insert("status".into(), ArgVal::Text((*status).into()));
                    // gen is stamped by project_author for commutative ops.
                    self.apply(object_id_hex, p::OP_SET_ITEM_PROGRESS, a)
                        .await?;
                }
                // steps not for this device (structure when not owner, or another
                // actor's progress) are skipped.
                _ => {}
            }
        }

        self.sync_once().await?;
        Ok(())
    }

    /// The app-facing seed: resolve THIS device's actor from its display name and
    /// the peer from the project roster, then run our half of `AW26_PROJECT`.
    /// Returns false if this identity isn't a scripted actor (seeds nothing) —
    /// mirrors `seed_dm_demo`.
    pub async fn seed_project_demo(&self, object_id_hex: &str) -> Result<bool, CoreError> {
        let my_name = self.display_name()?;
        let my_actor = match crate::demo::actor_for_display_name(&my_name) {
            Some(a) => a,
            None => return Ok(false),
        };
        let peer_actor = match my_actor {
            "nye" => "harvey",
            "harvey" => "nye",
            _ => return Ok(false),
        };
        let group_id =
            hex::decode(object_id_hex).map_err(|_| CoreError::Directory("bad object id".into()))?;
        let me = self.id.identity_pk();
        let peer_id = self
            .dir
            .group_members(&group_id)?
            .into_iter()
            .find(|m| *m != me)
            .unwrap_or(me);
        self.seed_project_script(object_id_hex, &AW26_PROJECT, my_actor, peer_actor, &peer_id)
            .await?;
        Ok(true)
    }
}
