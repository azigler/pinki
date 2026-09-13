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

use std::collections::BTreeMap;

use serde::Serialize;

use crate::record::Promise;

/// The extension URI. Provisional — A2A-EXTENSION.md §6, and DESIGN.md §8's first
/// open question. It lives here, once: a breaking change MUST use a new URI, so the
/// string is a single edit rather than a grep.
pub const URI: &str = "https://github.com/azigler/pinki/ext/promise/v0";

/// The `description` A2A-EXTENSION.md §Surface 1 puts on the declaration, verbatim.
const DESCRIPTION: &str =
    "Promises made by this card's subject: deadline, evidence on satisfaction, reason on abandonment.";

/// The `metadata` key a promise record rides under — §Surface 3.
pub fn metadata_key() -> String {
    format!("{URI}/promise")
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
    let block = Card {
        capabilities: Capabilities {
            extensions: vec![Extension {
                uri: URI,
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
/// pinki has no task `status` and no way to learn one, and a promise with no `task`
/// field has no Task id to put on a fabricated envelope — emitting either would be
/// asserting A2A state pinki cannot observe. One key, one value: the §1 record
/// verbatim.
pub fn task_metadata(record: &Promise) -> Result<String, serde_json::Error> {
    let mut block: BTreeMap<String, &Promise> = BTreeMap::new();
    block.insert(metadata_key(), record);
    serde_json::to_string_pretty(&block)
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
        assert_eq!(ext["uri"], URI);
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
        let json = task_metadata(&record()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let object = value.as_object().unwrap();
        assert_eq!(object.len(), 1);
        let (key, carried) = object.iter().next().unwrap();
        assert_eq!(
            key,
            "https://github.com/azigler/pinki/ext/promise/v0/promise"
        );
        let back: Promise = serde_json::from_value(carried.clone()).unwrap();
        assert_eq!(back, record());
    }

    #[test]
    fn task_block_keeps_the_records_field_order() {
        let json = task_metadata(&record()).unwrap();
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

    #[test]
    fn task_block_carries_no_computed_state() {
        // A2A-EXTENSION.md §2: `overdue` is never an A2A state, and no computed state
        // rides the metadata block at all — it carries the record, nothing else.
        let json = task_metadata(&record()).unwrap();
        assert!(!json.contains("state"));
        assert!(!json.contains("overdue"));
    }
}
