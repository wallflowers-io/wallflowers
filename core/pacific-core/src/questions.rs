//! questions — the QUESTIONS facet: what a Site's owner or admins ask its members, and each
//! member's answer (W-98 Members: "Site owner/admin can define free questions and multiselect
//! polls"; ICD 2.3.1 `facets.questions`, band `0xF00C`).
//!
//! A question has no id: it IS the (author, gen) of its `base.defineQuestion`, as a proposal is
//! its `ratify.propose`. It counts while its author is the owner or holds admin in the Site's own
//! folded roles, as `group.editFace`'s writes do, so a revoked admin's questions, their
//! retirements and the answers to their questions stop counting. `base.retireQuestion` closes one
//! to new answers; it and its answers stay, marked retired. `base.answerQuestion` is per (author,
//! question) LWW by gen; an empty answer clears it. `member` is a leaf on the roster (ICD
//! `principals`), so the answers are roster members' only.
//!
//! The facet pattern, as [`crate::profiles`]: a type holds [`Questions`], splices
//! [`QUESTION_OPS`] into its `ops()` and delegates to [`reduce_questions`].

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

use crate::coordinator::{ArgVal, Args};
use crate::group::GroupRole;
use crate::object::{Authority, Commutativity, DeltaRejection, MemberId, Op, OpDecl};
use crate::object_args::{arg_hex32, req_int, req_text};

pub const OP_DEFINE_QUESTION: u32 = 0xF00C_0000;
pub const OP_RETIRE_QUESTION: u32 = 0xF00C_0001;
pub const OP_ANSWER_QUESTION: u32 = 0xF00C_0002;

pub const MAX_TEXT_BYTES: usize = 1024;
pub const MIN_OPTIONS: usize = 2;
pub const MAX_OPTIONS: usize = 24;
pub const MAX_LABEL_CHARS: usize = 80;
pub const MAX_HINT_BYTES: usize = 200;
pub const MAX_ANSWER_BYTES: i64 = 4096;

pub static QUESTION_OPS: &[OpDecl] = &[
    OpDecl {
        op_id: OP_DEFINE_QUESTION,
        name: "base.defineQuestion",
        authority: Authority::OwnerOrRole(GroupRole::Admin),
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_RETIRE_QUESTION,
        name: "base.retireQuestion",
        authority: Authority::OwnerOrRole(GroupRole::Admin),
        commutativity: Commutativity::Commutative,
    },
    OpDecl {
        op_id: OP_ANSWER_QUESTION,
        name: "base.answerQuestion",
        authority: Authority::AnyMember,
        commutativity: Commutativity::Commutative,
    },
];

pub fn is_question_op(op_id: u32) -> bool {
    QUESTION_OPS.iter().any(|d| d.op_id == op_id)
}

/// One member's answer: free text, option indexes, or both where the question takes both.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub text: Option<String>,
    pub choices: Vec<usize>,
    pub gen: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub text: String,
    /// None for a free question.
    pub options: Option<Vec<String>>,
    pub multi: bool,
    pub free: bool,
    pub max: Option<usize>,
    pub hint: Option<String>,
    /// The free-text answer's cap, where the question takes free text.
    pub text_max: Option<usize>,
    pub retired: bool,
    pub answers: BTreeMap<MemberId, Answer>,
}

/// Every question that counts, by (gen, author): oldest first.
pub type Questions = BTreeMap<(u64, MemberId), Question>;

fn gen_of(args: &Args) -> Result<u64, DeltaRejection> {
    u64::try_from(req_int(args, "gen")?).map_err(|_| DeltaRejection::MalformedArgs)
}

fn opt_int(args: &Args, key: &str) -> Result<Option<i64>, DeltaRejection> {
    match crate::arg_reads::get(args, key) {
        None => Ok(None),
        Some(ArgVal::Int(n)) => Ok(Some(*n)),
        Some(_) => Err(DeltaRejection::MalformedArgs),
    }
}

fn opt_text<'a>(args: &'a Args, key: &str) -> Result<Option<&'a str>, DeltaRejection> {
    match crate::arg_reads::get(args, key) {
        None => Ok(None),
        Some(ArgVal::Text(t)) => Ok(Some(t)),
        Some(_) => Err(DeltaRejection::MalformedArgs),
    }
}

fn flag(v: Option<i64>) -> Result<Option<bool>, DeltaRejection> {
    match v {
        None => Ok(None),
        Some(0) => Ok(Some(false)),
        Some(1) => Ok(Some(true)),
        Some(_) => Err(DeltaRejection::MalformedArgs),
    }
}

/// The owner at the delta's epoch, or a roster member holding admin in the object's roles.
fn owner_or_admin(roles: &BTreeMap<MemberId, GroupRole>, op: &Op<'_>) -> bool {
    *op.author == op.ctx.owner || (op.ctx.is_member(op.author) && roles.get(op.author) == Some(&GroupRole::Admin))
}

/// The question (target_author, target_gen) names.
fn target(args: &Args) -> Result<(u64, MemberId), DeltaRejection> {
    let author = arg_hex32(args, "target_author")?;
    let gen = u64::try_from(req_int(args, "target_gen")?).map_err(|_| DeltaRejection::MalformedArgs)?;
    Ok((gen, author))
}

/// The three ops, over whatever State holds the facet and its roles. Every arg is validated
/// before the first mutation: a refused delta leaves the state as it was.
pub fn reduce_questions(
    questions: &mut Questions,
    roles: &BTreeMap<MemberId, GroupRole>,
    op: &Op<'_>,
) -> Result<(), DeltaRejection> {
    let args = op.args;
    match op.op_id {
        OP_DEFINE_QUESTION => {
            let text = req_text(args, "text")?;
            if text.is_empty() || text.len() > MAX_TEXT_BYTES {
                return Err(DeltaRejection::MalformedArgs);
            }
            let options = match opt_text(args, "options")? {
                None => None,
                Some(j) => {
                    let labels: Vec<String> = serde_json::from_str(j).map_err(|_| DeltaRejection::MalformedArgs)?;
                    let distinct = labels.iter().collect::<BTreeSet<_>>().len() == labels.len();
                    let sized = |l: &String| !l.is_empty() && l.chars().count() <= MAX_LABEL_CHARS;
                    if !(MIN_OPTIONS..=MAX_OPTIONS).contains(&labels.len()) || !distinct || !labels.iter().all(sized) {
                        return Err(DeltaRejection::MalformedArgs);
                    }
                    Some(labels)
                }
            };
            let multi = flag(opt_int(args, "multi")?)?.unwrap_or(false);
            let free = flag(opt_int(args, "free")?)?.unwrap_or(options.is_none());
            // Several choices, and a cap on them, only among options; a question takes an answer.
            if (multi && options.is_none()) || (!free && options.is_none()) {
                return Err(DeltaRejection::MalformedArgs);
            }
            let max = match opt_int(args, "max")? {
                None => None,
                Some(m) => {
                    let count = options.as_ref().map_or(0, Vec::len) as i64;
                    if !multi || m < MIN_OPTIONS as i64 || m > count {
                        return Err(DeltaRejection::MalformedArgs);
                    }
                    Some(m as usize)
                }
            };
            let hint = match opt_text(args, "hint")? {
                Some(h) if h.len() > MAX_HINT_BYTES => return Err(DeltaRejection::MalformedArgs),
                Some(h) if !h.is_empty() => Some(h.to_string()),
                _ => None,
            };
            let text_max = match opt_int(args, "textMax")? {
                None => free.then_some(MAX_ANSWER_BYTES as usize),
                Some(n) if free && (1..=MAX_ANSWER_BYTES).contains(&n) => Some(n as usize),
                Some(_) => return Err(DeltaRejection::MalformedArgs),
            };
            let gen = gen_of(args)?;
            if !owner_or_admin(roles, op) {
                return Err(DeltaRejection::Unauthorized);
            }
            let key = (gen, *op.author);
            if questions.contains_key(&key) {
                return Err(DeltaRejection::PreconditionFailed);
            }
            questions.insert(key, Question { text: text.to_string(), options, multi, free, max, hint, text_max, retired: false, answers: BTreeMap::new() });
            Ok(())
        }
        OP_RETIRE_QUESTION => {
            let key = target(args)?;
            gen_of(args)?;
            if !owner_or_admin(roles, op) {
                return Err(DeltaRejection::Unauthorized);
            }
            questions.get_mut(&key).ok_or(DeltaRejection::PreconditionFailed)?.retired = true;
            Ok(())
        }
        OP_ANSWER_QUESTION => {
            let key = target(args)?;
            let gen = gen_of(args)?;
            if !op.ctx.is_member(op.author) {
                return Err(DeltaRejection::Unauthorized);
            }
            let q = questions.get_mut(&key).ok_or(DeltaRejection::PreconditionFailed)?;
            if q.retired {
                return Err(DeltaRejection::PreconditionFailed);
            }
            // What the answer carries is read against the question it answers, one arg at a time.
            let text = match opt_text(args, "text")? {
                Some(t) if !t.is_empty() => {
                    if !q.free || q.text_max.is_some_and(|m| t.len() > m) {
                        return Err(DeltaRejection::MalformedArgs);
                    }
                    Some(t.to_string())
                }
                _ => None,
            };
            let choices: Vec<usize> = match opt_text(args, "choices")? {
                None => Vec::new(),
                Some(j) => serde_json::from_str(j).map_err(|_| DeltaRejection::MalformedArgs)?,
            };
            if !choices.is_empty() {
                let count = q.options.as_ref().map_or(0, Vec::len);
                let distinct = choices.iter().collect::<BTreeSet<_>>().len() == choices.len();
                let cap = if q.multi { q.max.unwrap_or(count) } else { 1 };
                if count == 0 || !distinct || choices.iter().any(|c| *c >= count) || choices.len() > cap {
                    return Err(DeltaRejection::MalformedArgs);
                }
            }
            if q.answers.get(op.author).is_some_and(|held| held.gen > gen) {
                return Ok(());
            }
            if text.is_none() && choices.is_empty() {
                q.answers.remove(op.author);
            } else {
                q.answers.insert(*op.author, Answer { text, choices, gen });
            }
            Ok(())
        }
        _ => Err(DeltaRejection::UnknownType),
    }
}

/// The view's `questions`: `[{author, gen, text, options, multi, free, retired, answers:
/// {<member key>: {text, choices, gen}}, tally}]`, oldest first, with `max`, `hint` and `textMax`,
/// defineQuestion's declared args; answers and tally from members on `roster` only.
pub fn view(questions: &Questions, roster: &[MemberId]) -> Value {
    Value::Array(
        questions
            .iter()
            .map(|((gen, author), q)| {
                let answers: Vec<(&MemberId, &Answer)> = q.answers.iter().filter(|(m, _)| roster.contains(m)).collect();
                let mut tally = vec![0u64; q.options.as_ref().map_or(0, Vec::len)];
                for (_, a) in &answers {
                    for c in &a.choices {
                        tally[*c] += 1;
                    }
                }
                json!({
                    "author": hex::encode(author), "gen": gen, "text": q.text, "options": q.options,
                    "multi": q.multi, "free": q.free, "max": q.max, "hint": q.hint, "textMax": q.text_max,
                    "retired": q.retired,
                    "answers": answers.iter().map(|(m, a)| (hex::encode(m), json!({ "text": a.text, "choices": a.choices, "gen": a.gen }))).collect::<serde_json::Map<_, _>>(),
                    "tally": tally,
                })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ops the Group splices into its table are `QUESTION_OPS`' own.
    #[test]
    fn base_question_ops_are_spliced_verbatim() {
        use crate::object::ObjectType;
        let key = |d: &OpDecl| (d.op_id, d.name, d.authority, d.commutativity);
        for want in QUESTION_OPS {
            assert_eq!(crate::group::GroupType::op(want.op_id).map(key), Some(key(want)));
        }
    }

    /// The caps are the ICD's: the args' summaries state the numbers this fold holds.
    #[test]
    fn the_caps_are_the_icds() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../coordination/delta-graph.icd.json");
        let doc: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let args = &doc["facets"]["questions"]["ops"]["base.defineQuestion"]["args"];
        let said = |k: &str| args[k]["summary"].as_str().unwrap().to_string();
        assert!(said("text").contains(&format!("at most {MAX_TEXT_BYTES} bytes")));
        assert!(said("options").contains(&format!("{MIN_OPTIONS} to {MAX_OPTIONS} distinct labels, each 1 to {MAX_LABEL_CHARS} characters")));
        assert!(said("hint").contains(&format!("at most {MAX_HINT_BYTES} bytes")));
        assert!(said("textMax").contains(&format!("1 to {MAX_ANSWER_BYTES}")));
    }
}
