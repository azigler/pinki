//! The promise record — DESIGN.md §1.
//!
//! One record type. No status field (state is computed from the log, never stored),
//! no `created` field (the `promise` event's own `ts` carries it), and none of the
//! priority/tag/assignee/project apparatus a task tracker would grow.

use serde::{Deserialize, Serialize};

/// A promise: who owes what to whom, by when.
///
/// Field order here is load-bearing. serde emits struct fields in declaration
/// order, and §1's example — and the `metadata` block in A2A-EXTENSION.md
/// §Surface 3, which carries this record verbatim — reads
/// `id, promise, by, to, on, until, task`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Promise {
    /// Stable identifier. Opaque.
    pub id: String,
    /// Free text. The thing owed. pinki never parses it.
    pub promise: String,
    /// The debtor — who owes. An A2A AgentCard identity.
    pub by: String,
    /// The creditor — who is owed.
    pub to: String,
    /// Antecedent: the `id` of another promise. Absent means born owed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on: Option<String>,
    /// Deadline, ISO-8601. Required — §1: a promise without one is a wish.
    pub until: String,
    /// The A2A `Task.id` this promise is *about*, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full() -> Promise {
        Promise {
            id: "pnk_4f3a91".into(),
            promise: "hand back a reviewed schema".into(),
            by: "https://example.org/agents/reviewer".into(),
            to: "https://example.org/agents/author".into(),
            on: Some("pnk_0c2b77".into()),
            until: "2026-09-01T17:00:00Z".into(),
            task: Some("a2a-task-9c1f0e".into()),
        }
    }

    #[test]
    fn serializes_in_design_section_1_field_order() {
        let json = serde_json::to_string(&full()).unwrap();
        assert_eq!(
            json,
            r#"{"id":"pnk_4f3a91","promise":"hand back a reviewed schema","by":"https://example.org/agents/reviewer","to":"https://example.org/agents/author","on":"pnk_0c2b77","until":"2026-09-01T17:00:00Z","task":"a2a-task-9c1f0e"}"#
        );
    }

    #[test]
    fn omits_absent_optional_fields() {
        let p = Promise {
            on: None,
            task: None,
            ..full()
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(
            !json.contains("\"on\""),
            "absent `on` must not appear: {json}"
        );
        assert!(
            !json.contains("\"task\""),
            "absent `task` must not appear: {json}"
        );
        assert_eq!(
            json,
            r#"{"id":"pnk_4f3a91","promise":"hand back a reviewed schema","by":"https://example.org/agents/reviewer","to":"https://example.org/agents/author","until":"2026-09-01T17:00:00Z"}"#
        );
    }

    #[test]
    fn round_trips() {
        let p = full();
        let back: Promise = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(p, back);
    }
}
