//! The two A2A blocks — A2A-EXTENSION.md §Surface 1 and §Surface 3.
//!
//! Both are pure functions from data to JSON text. Nothing here opens a socket,
//! and that is the point: DESIGN.md §5 — "the two `a2a` verbs **only write JSON to
//! stdout**. They do not call anything." Getting the block to an A2A peer is your
//! client's job.
//!
//! Every block is serialized straight to a string rather than through
//! [`serde_json::Value`]. That is deliberate: `to_value` collects into a sorted map,
//! which would reorder the promise record away from DESIGN.md §1's field order,
//! while serializing a struct directly preserves declaration order.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::event::{Event, EventBody};

#[cfg(test)]
use crate::record::Promise;

/// The v0 URI remains available for one minor release.
pub const URI_V0: &str = "https://github.com/azigler/pinki/ext/promise/v0";
/// v1 uses a new URI because the metadata value changes shape.
pub const URI_V1: &str = "https://github.com/azigler/pinki/ext/promise/v1";

/// The `description` A2A-EXTENSION.md §Surface 1 puts on the declaration, verbatim.
const DESCRIPTION: &str =
    "Promises made by this card's subject: deadline, evidence on satisfaction, reason on abandonment.";

/// The `metadata` key a promise record rides under — §Surface 3.
pub fn metadata_key() -> String {
    format!("{URI_V0}/promise")
}

/// The AgentCard block — §Surface 1.
///
/// `required` is `false`, always: "A promise is voluntary; an extension that refused
/// to talk to agents who do not track obligations would be a strange way to express
/// that." `params` is empty, and `params.assessments` is deliberately absent —
/// §Surface 1 reads absence as "not published here", and pinki has no way to know a
/// URL where its operator publishes assessments. Inventing one would be a claim
/// about the world made by a tool that cannot see it.
pub fn card() -> Result<String, serde_json::Error> {
    card_for(URI_V0)
}

pub fn card_v1() -> Result<String, serde_json::Error> {
    card_for(URI_V1)
}

fn card_for(uri: &'static str) -> Result<String, serde_json::Error> {
    let block = Card {
        capabilities: Capabilities {
            extensions: vec![Extension {
                uri,
                description: DESCRIPTION,
                required: false,
                params: BTreeMap::new(),
            }],
        },
    };
    serde_json::to_string_pretty(&block)
}

/// The Task **metadata block** for one promise — §Surface 3.
///
/// This is the object you merge into a `Task`'s `metadata`, not a whole `Task`.
/// pinki has no A2A task `status` and no way to learn one, so it emits no fabricated
/// Task envelope. The value is the same detail read model as `show --json`, with the
/// declaration still at the same path for older readers. Provenance belongs to
/// the extension value, not the A2A Task envelope.
pub fn task_metadata<T: Serialize>(
    detail: &T,
    as_of: &str,
    ledger_ref: &str,
) -> Result<String, serde_json::Error> {
    let mut block = BTreeMap::new();
    block.insert(
        metadata_key(),
        TaskValue {
            detail,
            provenance: Provenance { as_of, ledger_ref },
        },
    );
    serde_json::to_string_pretty(&block)
}

/// v1 carries an ordered list under one metadata key. Callers validate all ids
/// before this function serializes anything, so an unknown id cannot leak a
/// partial JSON block to stdout.
pub fn task_metadata_v1<T: Serialize>(
    details: &[T],
    ids: &[String],
    node: &str,
    seat: &str,
    as_of: &str,
    ledger_ref: &str,
    record_refs: &[String],
) -> Result<String, serde_json::Error> {
    let reference = format!("{node}:{ledger_ref}");
    let record_refs: Vec<String> = record_refs
        .iter()
        .map(|digest| format!("{node}:{digest}"))
        .collect();
    let promises: Vec<_> = details
        .iter()
        .zip(ids)
        .zip(&record_refs)
        .map(|((detail, id), record_ref)| V1TaskValue {
            detail,
            node,
            qid: format!("{node}:{seat}:{id}"),
            provenance: V1Provenance {
                as_of,
                ledger_ref: &reference,
                record_ref,
            },
        })
        .collect();
    let mut block = BTreeMap::new();
    block.insert(format!("{URI_V1}/promises"), promises);
    serde_json::to_string_pretty(&block)
}

/// A path-free reference to the exact ledger bytes that supplied the fold.
/// The same contents in a copied ledger have the same reference.
pub fn ledger_ref(contents: &str) -> String {
    let digest = Sha256::digest(contents.as_bytes());
    format!("sha256:{digest:x}")
}

/// A path-free reference to one promise's own history: the `sha256:` digest of the
/// ledger lines that can change this promise's computed read model, in log order.
///
/// `events` and `contents` must be the pair [`crate::ledger::read_existing_snapshot`]
/// returned: one event per non-blank line of `contents`, in the same order.
///
/// The selected lines are:
///
/// - every event whose subject is `id`: its declaration (and any re-declaration),
///   `amend`, `resolve`, `correct` and `assess` lines; and
/// - for each antecedent reachable through the `on` chain (the first declaration of
///   an id supplies its `on`; a cycle is visited once), the events that can change a
///   dependent's computed state: that antecedent's `promise`, `resolve` and
///   `correct` lines. Its `amend` and `assess` lines are left out, because a
///   dependent's state reads only whether the antecedent is satisfied, ended
///   unsatisfied, or still open.
///
/// Lines of an event type this build does not know are never selected: the fold
/// ignores them, so they change no read model. Each selected line is hashed as it
/// stands in the file, without its line terminator and followed by one `\n`, so the
/// digest does not depend on re-serialization and survives a copied ledger. The
/// caller adds the node prefix, as for the ledger reference.
pub fn record_ref(events: &[Event], contents: &str, id: &str) -> String {
    let lines: Vec<&str> = contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    debug_assert_eq!(lines.len(), events.len());

    let mut first_on: BTreeMap<&str, Option<&str>> = BTreeMap::new();
    for event in events {
        if let EventBody::Promise(record) = &event.body {
            first_on
                .entry(record.id.as_str())
                .or_insert(record.on.as_deref());
        }
    }
    let mut chain: BTreeSet<&str> = BTreeSet::new();
    let mut next = Some(id);
    while let Some(current) = next {
        if !chain.insert(current) {
            break;
        }
        next = first_on.get(current).copied().flatten();
    }

    let mut hasher = Sha256::new();
    for (event, line) in events.iter().zip(&lines) {
        let subject = event.subject();
        let selected = subject == id
            || (chain.contains(subject)
                && matches!(
                    event.body,
                    EventBody::Promise(_) | EventBody::Resolve { .. } | EventBody::Correct { .. }
                ));
        if selected {
            hasher.update(line.as_bytes());
            hasher.update(b"\n");
        }
    }
    format!("sha256:{:x}", hasher.finalize())
}

#[derive(Serialize)]
struct TaskValue<'a, T: Serialize> {
    #[serde(flatten)]
    detail: &'a T,
    provenance: Provenance<'a>,
}

#[derive(Serialize)]
struct V1TaskValue<'a, T: Serialize> {
    #[serde(flatten)]
    detail: &'a T,
    node: &'a str,
    qid: String,
    provenance: V1Provenance<'a>,
}

/// v1 provenance: `ledger_ref` is the coarse whole-ledger reference, `record_ref`
/// the per-promise one. `record_ref` is an additive v1 field.
#[derive(Serialize)]
struct V1Provenance<'a> {
    as_of: &'a str,
    ledger_ref: &'a str,
    record_ref: &'a str,
}

#[derive(Serialize)]
struct Provenance<'a> {
    as_of: &'a str,
    ledger_ref: &'a str,
}

#[derive(Serialize)]
struct Card {
    capabilities: Capabilities,
}

#[derive(Serialize)]
struct Capabilities {
    extensions: Vec<Extension>,
}

#[derive(Serialize)]
struct Extension {
    uri: &'static str,
    description: &'static str,
    required: bool,
    params: BTreeMap<String, serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> Promise {
        Promise {
            id: "pnk_4f3a91".into(),
            promise: "hand back a reviewed schema".into(),
            by: "https://example.org/agents/reviewer".into(),
            to: "https://example.org/agents/author".into(),
            on: Some("pnk_0c2b77".into()),
            until: "2026-09-01T17:00:00Z".into(),
            task: Some("a2a-task-9c1f0e".into()),
            meta: None,
        }
    }

    #[test]
    fn card_matches_surface_1() {
        let value: serde_json::Value = serde_json::from_str(&card().unwrap()).unwrap();
        let ext = &value["capabilities"]["extensions"][0];
        assert_eq!(ext["uri"], URI_V0);
        assert_eq!(ext["description"], DESCRIPTION);
        assert_eq!(ext["required"], serde_json::json!(false));
        assert_eq!(ext["params"], serde_json::json!({}));
        assert_eq!(
            value["capabilities"]["extensions"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn card_never_invents_an_assessments_url() {
        assert!(!card().unwrap().contains("assessments"));
    }

    #[test]
    fn task_block_is_one_uri_prefixed_key_carrying_the_record() {
        let json = task_metadata(&record(), "2026-09-01T16:00:00Z", "sha256:abc").unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let object = value.as_object().unwrap();
        assert_eq!(object.len(), 1);
        let (key, carried) = object.iter().next().unwrap();
        assert_eq!(
            key,
            "https://github.com/azigler/pinki/ext/promise/v0/promise"
        );
        let mut record_value = carried.clone();
        assert_eq!(record_value["provenance"]["as_of"], "2026-09-01T16:00:00Z");
        assert_eq!(record_value["provenance"]["ledger_ref"], "sha256:abc");
        record_value.as_object_mut().unwrap().remove("provenance");
        let back: Promise = serde_json::from_value(record_value).unwrap();
        assert_eq!(back, record());
    }

    #[test]
    fn task_block_keeps_the_records_field_order() {
        let json = task_metadata(&record(), "2026-09-01T16:00:00Z", "sha256:abc").unwrap();
        let order: Vec<&str> = ["id", "promise", "by", "to", "on", "until", "task"]
            .into_iter()
            .collect();
        let mut cursor = 0;
        for field in order {
            let needle = format!("\"{field}\":");
            let at = json[cursor..]
                .find(&needle)
                .unwrap_or_else(|| panic!("{field} missing or out of order in:\n{json}"));
            cursor += at + needle.len();
        }
    }
}
