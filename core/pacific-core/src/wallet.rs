//! wallet — a treasury a group holds together, and the ledger that cannot drift
//! from it quietly.
//!
//! # What this is NOT
//!
//! **There are no voting ops here.** A release is decided by the base RATIFY
//! op-group ([`crate::coordinator`] — `ratify.propose` / `ratify.vote` /
//! `ratify.close`), which is present on every object, folded by the Coordinator
//! itself, and already proven across four devices in `m4_ratify.rs`. Adding a
//! second voting mechanism beside it would be the actual mistake: RATIFY's
//! `RProposal.payload` is documented as generalising to "an opaque inner delta
//! the passed close activates", and a release request is exactly that.
//!
//! So this module is the OTHER half — the money — and it is four ops:
//! the policy that says how much a decision must carry, and the three
//! statements that say what actually happened to the funds.
//!
//! # Custody, stated plainly
//!
//! The funds are OFF-PLATFORM: a real bank account a real person holds. Pacific
//! never touches them, and that is not a limitation to engineer around — a
//! reducer is pure by contract ([`crate::object::ObjectType::reduce`]: no IO, no
//! clock, no ambient reads beyond `op.ctx`). A fold that could move money would
//! not be a fold.
//!
//! What the log holds is therefore two different kinds of claim, and the whole
//! design turns on keeping them apart:
//!
//! - **A decision** — proposed, voted, closed. Cryptographically attributable,
//!   converges identically on every replica, and is THE authority on whether a
//!   release was authorised. That is RATIFY.
//! - **An attestation** — "I moved €1,800 on the 14th, reference FT2409X". A
//!   signed statement by a named member about the world. It is **testimony, not
//!   proof**, and nothing here pretends otherwise.
//!
//! This module cannot stop a member paying money out with no decision behind it.
//! What it does is make the gap between the two LOUD, NAMED and DATED — which is
//! precisely the bargain [`crate::membership::MembershipLog::divergence`] already
//! strikes with the MLS roster, and [`Wallet::divergence`] deliberately wears the
//! same shape: an `Option` where `None` is the clean case, and a taxonomy of
//! drift rather than a boolean.
//!
//! Note the parallel in the signature, too. `MembershipLog::divergence` takes the
//! roster from OUTSIDE rather than reaching for MLS; `Wallet::divergence` takes
//! the set of passed releases from outside rather than reaching into RATIFY. A
//! base facet that had to know about the governance layer would not be a base
//! facet.
//!
//! # Amounts are integer minor units, never floats
//!
//! €1,800.00 is `180000`. [`crate::geo`] stores coordinates as fixed-point e7 for
//! this exact reason — "float non-determinism would break log replay" — and two
//! replicas that disagreed about a euro would be worse than two that disagreed
//! about a latitude.
//!
//! # Wiring (the base op-group)
//!
//! Like [`crate::geo`], [`crate::membership`], [`crate::visibility`] and
//! [`crate::publication`], this is a BASE op-group in a reserved high op-id band
//! so it can never collide with a type's own ops (which number from 0). A kind
//! that carries a treasury embeds a `wallet: Wallet` in its `State`, splices
//! [`WALLET_OPS`] into its `ops()`, and routes [`is_wallet_op`] to
//! [`reduce_wallet`] from `reduce`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::coordinator::ArgVal;
use crate::coordinator::Args;
use crate::object::{Authority, Commutativity, DeltaRejection, MemberId, Op, OpDecl};
use crate::object_args::{opt_text, req_int, req_text};

/// Reserved base-op band, beside geo's `0xF000_xxxx`, membership's `0xF001_xxxx`,
/// visibility's `0xF002_xxxx` and publication's `0xF003_xxxx`.
pub const OP_SET_POLICY: u32 = 0xF004_0000;
pub const OP_RECORD_DEPOSIT: u32 = 0xF004_0001;
pub const OP_ATTEST_SETTLEMENT: u32 = 0xF004_0002;
pub const OP_ATTEST_BALANCE: u32 = 0xF004_0003;

/// The base wallet ops, to be included in a type's `ops()`.
///
/// **The policy is owner/sequenced** for the same reason visibility and
/// publication are: how a group's money may be released is the owner's assertion
/// about their own object, and two concurrent policies naming different
/// thresholds must resolve to one answer. A commutative merge would leave the
/// treasury governed by two constitutions at once.
///
/// **The three ledger ops are any-member/commutative**, and that is forced rather
/// than chosen: the spec invariant in [`crate::object::OpDecl::is_well_formed`]
/// says a commutative op MUST be any-member, and these must be commutative
/// because a member recording a payment cannot be made to wait for the owner to
/// come online and sequence it. Money moves when it moves.
///
/// Authority is therefore NOT the permission system here — it is the
/// spine/broadcast split. Role gating (only an Admin may attest) is a
/// PRECONDITION inside the reducer, which is legal because roles fold off the
/// sequenced spine and the Coordinator folds the spine in full before any
/// commutative op. See [`reduce_wallet`].
pub static WALLET_OPS: &[OpDecl] = &[
    OpDecl {
        op_id: OP_SET_POLICY,
        name: "base.setWalletPolicy",
        authority: Authority::Owner,
        commutativity: Commutativity::Sequenced,
    },
    OpDecl {
        op_id: OP_RECORD_DEPOSIT,
        name: "base.recordDeposit",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_ATTEST_SETTLEMENT,
        name: "base.attestSettlement",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_ATTEST_BALANCE,
        name: "base.attestBalance",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
];

/// True if `op_id` is a base wallet op, so a kind can route it before its own.
pub fn is_wallet_op(op_id: u32) -> bool {
    matches!(
        op_id,
        OP_SET_POLICY | OP_RECORD_DEPOSIT | OP_ATTEST_SETTLEMENT | OP_ATTEST_BALANCE
    )
}

/// How a release of a given size must be decided. The rule belongs to the
/// AMOUNT, not to the wallet: one threshold for a treasury is either too brittle
/// for buying timber or too loose for spending half of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Rule {
    /// No poll. Logged, visible, and inside the band an Admin may simply pay it.
    /// A rule nobody can be bothered to follow gets routed around, so the small
    /// band exists to keep the large ones credible.
    Petty,
    /// More than half of the ballots that expressed a preference.
    Majority,
    /// Two thirds of the ballots that expressed a preference.
    Supermajority,
    /// Nobody rejected. The one-person veto, in its right place at the top.
    Consent,
}

impl Rule {
    pub fn parse(s: &str) -> Result<Self, DeltaRejection> {
        Ok(match s {
            "petty" => Self::Petty,
            "majority" => Self::Majority,
            "supermajority" => Self::Supermajority,
            "consent" => Self::Consent,
            _ => return Err(DeltaRejection::MalformedArgs),
        })
    }
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Petty => "petty",
            Self::Majority => "majority",
            Self::Supermajority => "supermajority",
            Self::Consent => "consent",
        }
    }
}

/// One rung of the policy: everything strictly below `ceiling` is decided by
/// `rule`, needing `quorum` of the electorate to have cast a ballot at all.
///
/// **Quorum and threshold are separate numbers on purpose.** The threshold is of
/// the ballots that expressed a preference; the quorum is of the electorate.
/// Collapse them and two members awake at 3am can move the whole wallet.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Band {
    /// Exclusive upper bound in minor units. `None` is the top band.
    pub ceiling: Option<i64>,
    pub rule: Rule,
    pub quorum: u32,
}

/// What a member BELOW the voting line is shown.
///
/// **This is presentation, not confidentiality, and the name of the type is the
/// only place that can be said once.** Every member of the object holds the keys
/// and folds the same log, so a hidden figure is a courtesy the reader's own
/// device extends — not a secret being kept from them. A member who must not see
/// the balance must not be in the object; that is what `membership == access`
/// means, and no field here can substitute for it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Disclosure {
    /// Everything, to everyone in the roster.
    #[default]
    Full,
    /// The decisions and their outcomes, without the figures.
    Decisions,
    /// The balance alone.
    Totals,
}

impl Disclosure {
    pub fn parse(s: &str) -> Result<Self, DeltaRejection> {
        Ok(match s {
            "full" => Self::Full,
            "decisions" => Self::Decisions,
            "totals" => Self::Totals,
            _ => return Err(DeltaRejection::MalformedArgs),
        })
    }
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Decisions => "decisions",
            Self::Totals => "totals",
        }
    }
}

/// Money in. Keyed by `reference` in [`Wallet::deposits`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deposit {
    pub amount: i64,
    /// Unix ms the depositor stamped. Event time, author-supplied — a reducer
    /// has no clock, so this is testimony like everything else here.
    pub at: i64,
    pub source: String,
    pub by: MemberId,
}

/// Money out, as attested by the member who says they moved it. Keyed by
/// `reference` in [`Wallet::settlements`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settlement {
    pub amount: i64,
    pub at: i64,
    /// The RATIFY proposal this discharges — `None` for a payment made with no
    /// decision behind it, which is legitimate below the petty band and a
    /// divergence above it. This one field is what makes the ledger answerable.
    pub proposal: Option<String>,
    pub memo: String,
    pub by: MemberId,
}

/// "The real account showed this, on this date, and I am the one saying so."
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Count {
    pub amount: i64,
    pub at: i64,
}

/// The folded treasury.
///
/// The balance is deliberately NOT a field. It is `deposits − settlements`,
/// derived on read — for the same reason RATIFY's `Outcome` is derived from the
/// frozen tally: a stored total is a second source of truth that can disagree
/// with the log that produced it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wallet {
    /// False until a policy has been set. A group with no treasury is a
    /// different thing from a treasury with nothing in it, and the two must not
    /// render the same.
    pub open: bool,
    /// ISO-4217, lowercase. "" when unopened.
    pub currency: String,
    /// A NAME, never a credential — "Triodos ••2291". The object has no business
    /// holding the second one and no use for it.
    pub account: String,
    pub disclosure: Disclosure,
    /// Hours a passed release waits before it may be settled. ADVISORY: a pure
    /// reducer has no clock, so nothing here can enforce it. It is checked
    /// against the attester's own `at` by the caller, and a breach surfaces as a
    /// divergence rather than a rejection — refusing the delta would lose the
    /// record of a payment that really happened.
    pub cooloff_hours: u32,
    /// Read in order; the first band whose ceiling the amount is under wins.
    pub bands: Vec<Band>,
    /// OR-set keyed by the payment reference, under a single-writer discipline —
    /// `event.rs`'s ledger keyed by `pi`, exactly. Only the treasurer authors
    /// these in practice, so the set behaves as an ordered ledger while a
    /// late-joining member still converges from any delivery order.
    pub deposits: BTreeMap<String, Deposit>,
    pub settlements: BTreeMap<String, Settlement>,
    /// Per-attester LWW by `at`: each member's most recent statement about what
    /// the real account held. Keyed by author so two treasurers counting on
    /// different days do not overwrite each other.
    pub counts: BTreeMap<MemberId, Count>,
    /// What this object is a part of, and in what role (`base.setParent`). `None`
    /// until a parent names it as a part.
    pub parent: Option<crate::parent::ParentRef>,
}

/// The drift between what the log says and what the world is claimed to be.
/// `None` from [`Wallet::divergence`] is the clean case.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WalletDivergence {
    /// Payment references with no passed decision behind them, above the petty
    /// band. The fold cannot prevent these; it can refuse to let them be quiet.
    pub unauthorised: Vec<String>,
    /// Paid more than the decision authorised.
    pub overpaid: Vec<String>,
    /// `(derived, counted)` — the ledger and the last real count disagree.
    pub unreconciled: Option<(i64, i64)>,
}

impl Wallet {
    /// The band governing `amount` — the first whose ceiling it falls under.
    /// `None` when no policy has been set, which is why every caller must treat
    /// an unopened wallet as "no releases are authorised" rather than "all are".
    pub fn band(&self, amount: i64) -> Option<&Band> {
        self.bands
            .iter()
            .find(|b| b.ceiling.is_none_or(|c| amount < c))
    }

    /// The ceiling under which a payment needs no decision at all. 0 when there
    /// is no petty band, which correctly makes every payment answerable.
    pub fn petty_ceiling(&self) -> i64 {
        match self.bands.first() {
            Some(b) if b.rule == Rule::Petty => b.ceiling.unwrap_or(i64::MAX),
            _ => 0,
        }
    }

    /// Deposits less settlements, in minor units. Derived, never stored.
    pub fn balance(&self) -> i64 {
        let in_: i64 = self.deposits.values().map(|d| d.amount).sum();
        let out: i64 = self.settlements.values().map(|s| s.amount).sum();
        in_ - out
    }

    /// The most recent count any member has attested, by `at`.
    pub fn latest_count(&self) -> Option<&Count> {
        self.counts.values().max_by_key(|c| c.at)
    }

    /// WHERE THE LEDGER AND THE DECISIONS DISAGREE.
    ///
    /// `authorised` maps a passed RATIFY proposal id to the amount it authorised.
    /// It is supplied by the caller rather than read from here, for the same
    /// reason [`crate::membership::MembershipLog::divergence`] takes the roster:
    /// the MLS tree is authoritative for membership and RATIFY is authoritative
    /// for decisions, and a facet that reached for either would be inventing a
    /// second source of truth.
    ///
    /// Read it as: the log says what was paid, RATIFY says what was agreed, and
    /// this is the one function allowed to notice that they are not the same.
    pub fn divergence(&self, authorised: &BTreeMap<String, i64>) -> Option<WalletDivergence> {
        let mut d = WalletDivergence::default();
        let petty = self.petty_ceiling();

        for (reference, s) in &self.settlements {
            match &s.proposal {
                // Below the petty band a payment needs no decision — that is what
                // the band is FOR, and flagging it would train every reader to
                // ignore the flag that matters.
                None => {
                    if s.amount >= petty {
                        d.unauthorised.push(reference.clone());
                    }
                }
                Some(pid) => match authorised.get(pid) {
                    None => d.unauthorised.push(reference.clone()),
                    Some(&cap) if s.amount > cap => d.overpaid.push(reference.clone()),
                    Some(_) => {}
                },
            }
        }

        let derived = self.balance();
        if let Some(c) = self.latest_count() {
            if c.amount != derived {
                d.unreconciled = Some((derived, c.amount));
            }
        }

        if d.unauthorised.is_empty() && d.overpaid.is_empty() && d.unreconciled.is_none() {
            None
        } else {
            Some(d)
        }
    }
}

/// Parse the band schedule off the policy delta.
///
/// One band per line, `ceiling|rule|quorum`, with `*` for the top band's
/// ceiling. Text rather than a structured arg because [`Args`] carries only
/// `Int` and `Text` — the same constraint `event.rs` meets by newline-separating
/// its lineup.
///
/// Validated whole before anything is assigned. Bands must ASCEND and must end
/// in an open-topped band, or there would be an amount the policy does not
/// govern — and "no rule covers this payment" is not a state a treasury may
/// reach by omission.
fn parse_bands(spec: &str) -> Result<Vec<Band>, DeltaRejection> {
    let mut out: Vec<Band> = Vec::new();
    for line in spec.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let mut it = line.split('|');
        let (c, r, q) = match (it.next(), it.next(), it.next(), it.next()) {
            (Some(c), Some(r), Some(q), None) => (c.trim(), r.trim(), q.trim()),
            _ => return Err(DeltaRejection::MalformedArgs),
        };
        let ceiling = match c {
            "*" => None,
            n => Some(n.parse::<i64>().map_err(|_| DeltaRejection::MalformedArgs)?),
        };
        if ceiling.is_some_and(|v| v <= 0) {
            return Err(DeltaRejection::MalformedArgs);
        }
        out.push(Band {
            ceiling,
            rule: Rule::parse(r)?,
            quorum: q.parse::<u32>().map_err(|_| DeltaRejection::MalformedArgs)?,
        });
    }
    if out.is_empty() {
        return Err(DeltaRejection::MalformedArgs);
    }
    // Ascending, and only the LAST may be open-topped. The open band is `None`,
    // which sorts against nothing — so the final pair is allowed to be
    // `(Some, None)` and every other pair must be two ascending ceilings.
    let last = out.len() - 1;
    for i in 0..last {
        match (out[i].ceiling, out[i + 1].ceiling) {
            (Some(a), Some(b)) if a < b => {}
            (Some(_), None) if i + 1 == last => {}
            _ => return Err(DeltaRejection::MalformedArgs),
        }
    }
    if out[last].ceiling.is_some() {
        return Err(DeltaRejection::MalformedArgs);
    }
    Ok(out)
}

/// Fold a base wallet op.
///
/// # Role gating lives here, and why that is legal
///
/// [`Authority`] has exactly two levels, so there is no rung for Admin to stand
/// on — the constraint `group.rs` names when it says widening `may_mint` "means
/// teaching the reducer to read `member_roles` at reduce time (a new authority
/// level), which is a governance decision". A treasury is where that decision
/// arrives, so the ledger ops declare `AnyMember` and take `is_treasurer` as a
/// PRECONDITION, rejecting with [`DeltaRejection::Unauthorized`].
///
/// This is deterministic — which is the only thing that matters for a fold —
/// because the roles it reads fold off the owner-sequenced spine, and the
/// Coordinator folds the spine IN FULL before any commutative delta. Every
/// replica therefore evaluates this check against the same role map.
///
/// It also means a demotion is retroactive: re-folding the log after someone
/// ceases to be a treasurer drops the attestations they authored. That is a real
/// consequence and the right one for a permission — but it is the same hazard
/// RATIFY freezes against with `RClose`, and a wallet that wanted tenure-aware
/// attestation would have to freeze likewise.
// ---- authoring -------------------------------------------------------------
//
// The four constructors this module was missing. Every other op-carrying module
// has them — `event::set_tickets_args`, `place::set_access_args`,
// `contact::supply_args` — because the app never assembles an `Args` map by
// hand: `node::post_to_group` calls `coordinator::forum_post_full` and hands the
// result to the write path. Without these, a caller reaching the wallet had to
// build the map itself, and the reducer's contract is strict enough that it
// would mostly have failed — silently, because `Coordinator::state()` discards
// reduce errors.
//
// `bands` is the reason this matters most. It is a text mini-format — one
// `ceiling|rule|quorum` per line, `*` for the top band — and a caller formatting
// that by hand is exactly the class of mistake a constructor exists to remove.
// [`set_policy_args`] takes `&[Band]` and writes the spec, so the parser in
// [`parse_bands`] and the writer here cannot drift: the round-trip is pinned by
// `bands_round_trip` below.

/// Serialize bands to the spec `parse_bands` reads. Ceilings ascend; `None` is
/// the top band and is written `*`.
pub fn bands_spec(bands: &[Band]) -> String {
    bands
        .iter()
        .map(|b| {
            let ceiling = match b.ceiling {
                Some(c) => c.to_string(),
                None => "*".to_string(),
            };
            format!("{}|{}|{}", ceiling, b.rule.as_str(), b.quorum)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `base.setWalletPolicy` — open the treasury and state its constitution.
/// Owner/sequenced. `currency` is ISO-4217 alpha-3; `cooloff_hours` is bounded
/// to a year by the reducer.
pub fn set_policy_args(
    currency: &str,
    bands: &[Band],
    cooloff_hours: u32,
    disclosure: Option<Disclosure>,
    account: Option<&str>,
) -> Args {
    let mut a = Args::new();
    a.insert("currency".into(), ArgVal::Text(currency.to_ascii_lowercase()));
    a.insert("bands".into(), ArgVal::Text(bands_spec(bands)));
    a.insert("cooloffHours".into(), ArgVal::Int(cooloff_hours as i64));
    if let Some(d) = disclosure {
        a.insert("disclosure".into(), ArgVal::Text(d.as_str().into()));
    }
    if let Some(acct) = account {
        a.insert("account".into(), ArgVal::Text(acct.into()));
    }
    a
}

/// `base.recordDeposit` — money arrived. Any-member/commutative, treasurer-only
/// at reduce time. `amount` is in minor units and must be positive: a refund is
/// a deposit, not a negative settlement.
pub fn record_deposit_args(reference: &str, amount: i64, at: i64, source: Option<&str>) -> Args {
    let mut a = ledger_map(reference, amount, at);
    if let Some(s) = source {
        a.insert("source".into(), ArgVal::Text(s.into()));
    }
    a
}

/// `base.attestSettlement` — money left. `proposal` is OPTIONAL and its absence
/// is meaningful: a payment with no decision behind it is exactly what
/// [`Wallet::divergence`] is for, so it is recorded rather than refused.
pub fn attest_settlement_args(
    reference: &str,
    amount: i64,
    at: i64,
    proposal: Option<&str>,
    memo: Option<&str>,
) -> Args {
    let mut a = ledger_map(reference, amount, at);
    if let Some(p) = proposal.filter(|s| !s.is_empty()) {
        a.insert("proposal".into(), ArgVal::Text(p.into()));
    }
    if let Some(m) = memo {
        a.insert("memo".into(), ArgVal::Text(m.into()));
    }
    a
}

/// `base.attestBalance` — what the account actually held on a date. The amount
/// may be negative (an overdrawn account is a fact); `at` may not be zero or
/// before it, and is the per-author LWW key.
pub fn attest_balance_args(amount: i64, at: i64) -> Args {
    let mut a = Args::new();
    a.insert("amount".into(), ArgVal::Int(amount));
    a.insert("at".into(), ArgVal::Int(at));
    a
}

/// The three fields every ledger statement carries, in one place so the two
/// statements cannot disagree about their shape.
fn ledger_map(reference: &str, amount: i64, at: i64) -> Args {
    let mut a = Args::new();
    a.insert("reference".into(), ArgVal::Text(reference.trim().into()));
    a.insert("amount".into(), ArgVal::Int(amount));
    a.insert("at".into(), ArgVal::Int(at));
    a
}

pub fn reduce_wallet(
    wallet: &mut Wallet,
    op: &Op<'_>,
    is_treasurer: bool,
) -> Result<(), DeltaRejection> {
    match op.op_id {
        OP_SET_POLICY => {
            // Owner/sequenced: the spine already rejected a non-owner author.
            // Validate EVERY field before the first mutation — a reduce-time
            // rejection is skipped at fold, and a half-applied policy would
            // leave a treasury with bands from one constitution and a quorum
            // from another as its permanent state.
            let currency = req_text(op.args, "currency")?.to_ascii_lowercase();
            if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_alphabetic()) {
                return Err(DeltaRejection::MalformedArgs);
            }
            let bands = parse_bands(req_text(op.args, "bands")?)?;
            let disclosure = match opt_text(op.args, "disclosure") {
                Some(s) => Disclosure::parse(&s)?,
                None => Disclosure::default(),
            };
            let cooloff = req_int(op.args, "cooloffHours")?;
            if !(0..=8760).contains(&cooloff) {
                // A year is already absurd; beyond it the window stops being a
                // cooling-off period and becomes a way to brick the treasury.
                return Err(DeltaRejection::MalformedArgs);
            }
            let account = opt_text(op.args, "account").unwrap_or_default();

            wallet.open = true;
            wallet.currency = currency;
            wallet.account = account;
            wallet.disclosure = disclosure;
            wallet.cooloff_hours = cooloff as u32;
            wallet.bands = bands;
            Ok(())
        }

        OP_RECORD_DEPOSIT => {
            if !is_treasurer {
                return Err(DeltaRejection::Unauthorized);
            }
            let (reference, amount, at) = ledger_args(op.args)?;
            let source = opt_text(op.args, "source").unwrap_or_default();
            // A payment into a wallet nobody has opened has no currency to be
            // denominated in and no policy to be released under.
            if !wallet.open {
                return Err(DeltaRejection::PreconditionFailed);
            }
            wallet.deposits.insert(
                reference,
                Deposit {
                    amount,
                    at,
                    source,
                    by: *op.author,
                },
            );
            Ok(())
        }

        OP_ATTEST_SETTLEMENT => {
            if !is_treasurer {
                return Err(DeltaRejection::Unauthorized);
            }
            let (reference, amount, at) = ledger_args(op.args)?;
            if !wallet.open {
                return Err(DeltaRejection::PreconditionFailed);
            }
            // The proposal id is OPTIONAL and its absence is meaningful: it is a
            // payment with no decision behind it. Recording it is the point —
            // `Wallet::divergence` is what has an opinion about it. Refusing the
            // delta would delete the evidence of the very thing worth catching.
            let proposal = opt_text(op.args, "proposal").filter(|s| !s.is_empty());
            let memo = opt_text(op.args, "memo").unwrap_or_default();
            wallet.settlements.insert(
                reference,
                Settlement {
                    amount,
                    at,
                    proposal,
                    memo,
                    by: *op.author,
                },
            );
            Ok(())
        }

        OP_ATTEST_BALANCE => {
            if !is_treasurer {
                return Err(DeltaRejection::Unauthorized);
            }
            let amount = req_int(op.args, "amount")?;
            let at = req_int(op.args, "at")?;
            if at <= 0 {
                return Err(DeltaRejection::MalformedArgs);
            }
            // A count may legitimately be negative (an overdrawn account is a
            // fact), so only the date is bounded. Per-author LWW by `at`: an
            // older count never overwrites a newer one, so the fold is
            // order-independent without needing a gen tiebreak.
            let slot = wallet.counts.entry(*op.author).or_insert(Count { amount, at });
            if at >= slot.at {
                *slot = Count { amount, at };
            }
            Ok(())
        }

        _ => Err(DeltaRejection::UnknownType),
    }
}

/// The three args every ledger row carries. Pulled out because getting the
/// validation subtly different between deposits and settlements is exactly the
/// bug that would not show up until the two disagreed about a total.
fn ledger_args(args: &Args) -> Result<(String, i64, i64), DeltaRejection> {
    let reference = req_text(args, "reference")?.trim().to_string();
    if reference.is_empty() || reference.len() > 128 {
        return Err(DeltaRejection::MalformedArgs);
    }
    let amount = req_int(args, "amount")?;
    // Zero is not a payment and a negative one is a different op that does not
    // exist — a refund is a deposit. Signed subtraction is how the balance is
    // derived, so a negative settlement would silently ADD to the balance.
    if amount <= 0 {
        return Err(DeltaRejection::MalformedArgs);
    }
    let at = req_int(args, "at")?;
    if at <= 0 {
        return Err(DeltaRejection::MalformedArgs);
    }
    Ok((reference, amount, at))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The writer and the parser are two halves of one format; this is what stops
    /// them drifting. A band spec that survives a round trip is a band spec the
    /// reducer will accept, which is the only guarantee a caller actually wants.
    #[test]
    fn bands_round_trip() {
        let bands = vec![
            Band { ceiling: Some(5_000), rule: Rule::Petty, quorum: 0 },
            Band { ceiling: Some(100_000), rule: Rule::Majority, quorum: 3 },
            Band { ceiling: None, rule: Rule::Consent, quorum: 5 },
        ];
        let spec = bands_spec(&bands);
        assert_eq!(spec, "5000|petty|0\n100000|majority|3\n*|consent|5");
        assert_eq!(parse_bands(&spec).expect("the writer's output parses"), bands);
    }

    /// The constructors exist so a caller cannot get the arguments wrong; the
    /// proof of that is that what they build actually FOLDS. A wallet delta with
    /// hand-built args mostly does not — and fails silently, because
    /// `Coordinator::state()` discards reduce errors.
    #[test]
    fn constructed_policy_args_are_accepted_by_the_reducer() {
        let bands = vec![Band { ceiling: None, rule: Rule::Consent, quorum: 2 }];
        let args = set_policy_args("EUR", &bands, 48, Some(Disclosure::default()), Some("acct-1"));
        let mut w = Wallet::default();
        let op = Op {
            op_id: OP_SET_POLICY,
            args: &args,
            author: &[7u8; 32],
            pos: None,
            ctx: &ReduceContext { members: &[], owner: [7u8; 32], epoch: 0 },
        };
        reduce_wallet(&mut w, &op, true).expect("the constructor's args reduce cleanly");
        assert!(w.open, "the treasury opened");
        assert_eq!(w.currency, "eur", "currency is normalised to lower case");
        assert_eq!(w.cooloff_hours, 48);
        assert_eq!(w.bands.len(), 1);
    }

    use super::*;
    use crate::coordinator::ArgVal;
    use crate::object::{Op, ReduceContext};

    const OWNER: MemberId = [1u8; 32];
    const TREASURER: MemberId = [2u8; 32];

    fn args(pairs: &[(&str, ArgVal)]) -> Args {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }
    fn t(s: &str) -> ArgVal {
        ArgVal::Text(s.into())
    }

    const BANDS: &str = "10000|petty|0\n100000|majority|4\n500000|supermajority|5\n*|consent|7";

    fn apply(
        w: &mut Wallet,
        op_id: u32,
        a: Args,
        author: MemberId,
        treasurer: bool,
    ) -> Result<(), DeltaRejection> {
        let members = [OWNER, TREASURER];
        let ctx = ReduceContext {
            members: &members,
            owner: OWNER,
            epoch: 1,
        };
        reduce_wallet(
            w,
            &Op {
                op_id,
                args: &a,
                author: &author,
                pos: None,
                ctx: &ctx,
            },
            treasurer,
        )
    }

    fn opened() -> Wallet {
        let mut w = Wallet::default();
        apply(
            &mut w,
            OP_SET_POLICY,
            args(&[
                ("currency", t("EUR")),
                ("bands", t(BANDS)),
                ("cooloffHours", ArgVal::Int(24)),
                ("account", t("Triodos ••2291")),
            ]),
            OWNER,
            true,
        )
        .expect("policy");
        w
    }

    #[test]
    fn policy_opens_the_wallet_and_normalises_currency() {
        let w = opened();
        assert!(w.open);
        assert_eq!(w.currency, "eur", "ISO-4217 is stored lowercase");
        assert_eq!(w.bands.len(), 4);
        assert_eq!(w.petty_ceiling(), 10000);
        assert_eq!(w.disclosure, Disclosure::Full, "the default is full");
    }

    /// The band schedule is the policy: a gap in it is an amount no rule governs,
    /// which is not a state a treasury may reach by omission.
    #[test]
    fn band_schedule_must_ascend_and_be_open_topped() {
        for bad in [
            "100000|majority|4",                       // not open-topped
            "*|consent|7\n10000|petty|0",              // open band not last
            "100000|majority|4\n10000|petty|0\n*|consent|7", // descending
            "10000|nonsense|0\n*|consent|7",           // unknown rule
            "10000|petty\n*|consent|7",                // wrong arity
            "",                                        // empty
        ] {
            assert!(parse_bands(bad).is_err(), "must reject: {bad:?}");
        }
        assert!(parse_bands(BANDS).is_ok());
    }

    #[test]
    fn bands_select_by_amount() {
        let w = opened();
        assert_eq!(w.band(9_999).unwrap().rule, Rule::Petty);
        assert_eq!(w.band(10_000).unwrap().rule, Rule::Majority, "exclusive bound");
        assert_eq!(w.band(180_000).unwrap().rule, Rule::Supermajority);
        assert_eq!(w.band(900_000).unwrap().rule, Rule::Consent);
    }

    #[test]
    fn balance_is_derived_from_the_two_ledgers() {
        let mut w = opened();
        apply(&mut w, OP_RECORD_DEPOSIT, args(&[
            ("reference", t("FC-114")), ("amount", ArgVal::Int(1_000_000)),
            ("at", ArgVal::Int(1_000)), ("source", t("grant")),
        ]), TREASURER, true).unwrap();
        apply(&mut w, OP_ATTEST_SETTLEMENT, args(&[
            ("reference", t("TR-1")), ("amount", ArgVal::Int(31_000)),
            ("at", ArgVal::Int(2_000)), ("proposal", t("p3")),
        ]), TREASURER, true).unwrap();
        assert_eq!(w.balance(), 969_000);
    }

    /// The OR-set key is the payment reference, so a redelivered delta is
    /// idempotent rather than a second payment.
    #[test]
    fn a_replayed_settlement_does_not_double_count() {
        let mut w = opened();
        let a = args(&[
            ("reference", t("TR-1")), ("amount", ArgVal::Int(31_000)),
            ("at", ArgVal::Int(2_000)),
        ]);
        apply(&mut w, OP_RECORD_DEPOSIT, args(&[
            ("reference", t("D")), ("amount", ArgVal::Int(100_000)), ("at", ArgVal::Int(1)),
        ]), TREASURER, true).unwrap();
        apply(&mut w, OP_ATTEST_SETTLEMENT, a.clone(), TREASURER, true).unwrap();
        apply(&mut w, OP_ATTEST_SETTLEMENT, a, TREASURER, true).unwrap();
        assert_eq!(w.settlements.len(), 1);
        assert_eq!(w.balance(), 69_000);
    }

    #[test]
    fn a_non_treasurer_cannot_move_the_ledger() {
        let mut w = opened();
        for op in [OP_RECORD_DEPOSIT, OP_ATTEST_SETTLEMENT, OP_ATTEST_BALANCE] {
            let e = apply(&mut w, op, args(&[
                ("reference", t("X")), ("amount", ArgVal::Int(10)), ("at", ArgVal::Int(1)),
            ]), TREASURER, false);
            assert_eq!(e, Err(DeltaRejection::Unauthorized), "op {op:#x}");
        }
        assert!(w.deposits.is_empty() && w.settlements.is_empty() && w.counts.is_empty());
    }

    #[test]
    fn a_zero_or_negative_row_is_refused() {
        let mut w = opened();
        for bad in [0i64, -1] {
            assert_eq!(
                apply(&mut w, OP_ATTEST_SETTLEMENT, args(&[
                    ("reference", t("X")), ("amount", ArgVal::Int(bad)), ("at", ArgVal::Int(1)),
                ]), TREASURER, true),
                Err(DeltaRejection::MalformedArgs),
                "a negative settlement would ADD to the balance"
            );
        }
    }

    #[test]
    fn counts_are_per_author_lww_by_date() {
        let mut w = opened();
        let count = |amount, at| args(&[("amount", ArgVal::Int(amount)), ("at", ArgVal::Int(at))]);
        apply(&mut w, OP_ATTEST_BALANCE, count(500, 2_000), TREASURER, true).unwrap();
        apply(&mut w, OP_ATTEST_BALANCE, count(400, 1_000), TREASURER, true).unwrap();
        assert_eq!(w.counts[&TREASURER].amount, 500, "an older count never wins");
        apply(&mut w, OP_ATTEST_BALANCE, count(600, 3_000), TREASURER, true).unwrap();
        assert_eq!(w.counts[&TREASURER].amount, 600);
        // A second treasurer keeps their own slot rather than overwriting.
        apply(&mut w, OP_ATTEST_BALANCE, count(590, 2_500), OWNER, true).unwrap();
        assert_eq!(w.counts.len(), 2);
        assert_eq!(w.latest_count().unwrap().amount, 600, "latest by date, across authors");
    }

    /// The whole point of the module: the fold cannot stop a payment, but it can
    /// refuse to let one be quiet.
    #[test]
    fn divergence_names_what_the_decisions_do_not_cover() {
        let mut w = opened();
        apply(&mut w, OP_RECORD_DEPOSIT, args(&[
            ("reference", t("FC-114")), ("amount", ArgVal::Int(1_000_000)), ("at", ArgVal::Int(1)),
        ]), TREASURER, true).unwrap();

        // authorised: p3 for €310.00, p1 for €1,800.00
        let authorised: BTreeMap<String, i64> =
            [("p3".to_string(), 31_000), ("p1".to_string(), 180_000)].into();

        // 1. a settled, authorised payment — clean.
        apply(&mut w, OP_ATTEST_SETTLEMENT, args(&[
            ("reference", t("TR-1")), ("amount", ArgVal::Int(31_000)),
            ("at", ArgVal::Int(2)), ("proposal", t("p3")),
        ]), TREASURER, true).unwrap();
        // 2. under the petty band with no proposal — legitimate, and NOT flagged.
        apply(&mut w, OP_ATTEST_SETTLEMENT, args(&[
            ("reference", t("CARD-9")), ("amount", ArgVal::Int(9_000)), ("at", ArgVal::Int(3)),
        ]), TREASURER, true).unwrap();
        apply(&mut w, OP_ATTEST_BALANCE, args(&[
            ("amount", ArgVal::Int(960_000)), ("at", ArgVal::Int(4)),
        ]), TREASURER, true).unwrap();
        assert_eq!(w.divergence(&authorised), None, "clean: 1_000_000 − 40_000 = 960_000");

        // 3. above the petty band with no proposal — the headline case.
        apply(&mut w, OP_ATTEST_SETTLEMENT, args(&[
            ("reference", t("TR-9")), ("amount", ArgVal::Int(90_000)),
            ("at", ArgVal::Int(5)), ("memo", t("insurance")),
        ]), TREASURER, true).unwrap();
        // 4. paid against a proposal that never passed.
        apply(&mut w, OP_ATTEST_SETTLEMENT, args(&[
            ("reference", t("TR-10")), ("amount", ArgVal::Int(1_000)),
            ("at", ArgVal::Int(6)), ("proposal", t("p-never")),
        ]), TREASURER, true).unwrap();
        // 5. paid more than the decision authorised.
        apply(&mut w, OP_ATTEST_SETTLEMENT, args(&[
            ("reference", t("TR-11")), ("amount", ArgVal::Int(200_000)),
            ("at", ArgVal::Int(7)), ("proposal", t("p1")),
        ]), TREASURER, true).unwrap();

        let d = w.divergence(&authorised).expect("three problems");
        assert_eq!(d.unauthorised, vec!["TR-10", "TR-9"], "sorted by reference (BTreeMap)");
        assert_eq!(d.overpaid, vec!["TR-11"]);
        assert_eq!(
            d.unreconciled,
            Some((669_000, 960_000)),
            "the count is now stale — surfaced, not silently corrected"
        );
    }

    /// A wallet with no petty band makes EVERY payment answerable, which is the
    /// safe reading of a missing rung rather than the permissive one.
    #[test]
    fn no_petty_band_means_nothing_is_unattributed() {
        let mut w = Wallet::default();
        apply(&mut w, OP_SET_POLICY, args(&[
            ("currency", t("gbp")), ("bands", t("*|consent|7")), ("cooloffHours", ArgVal::Int(0)),
        ]), OWNER, true).unwrap();
        assert_eq!(w.petty_ceiling(), 0);
        apply(&mut w, OP_ATTEST_SETTLEMENT, args(&[
            ("reference", t("X")), ("amount", ArgVal::Int(1)), ("at", ArgVal::Int(1)),
        ]), TREASURER, true).unwrap();
        let d = w.divergence(&BTreeMap::new()).expect("one penny, unexplained");
        assert_eq!(d.unauthorised, vec!["X"]);
    }

    #[test]
    fn the_ledger_is_shut_until_a_policy_opens_it() {
        let mut w = Wallet::default();
        assert_eq!(
            apply(&mut w, OP_RECORD_DEPOSIT, args(&[
                ("reference", t("X")), ("amount", ArgVal::Int(10)), ("at", ArgVal::Int(1)),
            ]), TREASURER, true),
            Err(DeltaRejection::PreconditionFailed)
        );
    }

    #[test]
    fn an_unknown_op_in_the_band_is_loud() {
        let mut w = opened();
        assert_eq!(
            apply(&mut w, 0xF004_00FF, args(&[]), OWNER, true),
            Err(DeltaRejection::UnknownType)
        );
    }

    #[test]
    fn the_band_is_reserved_and_distinct() {
        for id in [OP_SET_POLICY, OP_RECORD_DEPOSIT, OP_ATTEST_SETTLEMENT, OP_ATTEST_BALANCE] {
            assert!(is_wallet_op(id));
            assert_eq!(id & 0xFFFF_0000, 0xF004_0000, "wallet owns 0xF004");
        }
        // The neighbouring bands are not ours.
        for id in [
            crate::geo::OP_SET_LOCATION,
            crate::membership::OP_MEMBER_JOINED,
            crate::visibility::OP_SET_VISIBILITY,
            crate::publication::OP_PUBLISH,
        ] {
            assert!(!is_wallet_op(id));
        }
    }

    /// The spec invariant every op table must hold: a commutative op cannot be
    /// owner-gated, because a broadcast op cannot be checked before it spreads.
    #[test]
    fn ops_are_well_formed() {
        for op in WALLET_OPS {
            assert!(op.is_well_formed(), "{} violates the spec invariant", op.name);
        }
    }
}
