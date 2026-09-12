//! The log's four event types — DESIGN.md §4.
//!
//! Append-only JSONL. `promise` speaks an obligation into existence, `amend` moves its
//! horizon without rewriting anything, `resolve` ends one, and `assess` publishes
//! somebody's judgment about one. State is a fold over these (see [`crate::state`]);
//! nothing here stores a state.
//!
//! ## The wire shape is the contract
//!
//! §4's examples are normative down to key order: `ts` first, then `type`, then the
//! body. [`Event`] gets that by holding `ts` and flattening the tagged [`EventBody`]
//! after it.
//!
//! ## The deliberate field-name collision
//!
//! §4 uses `promise` for two different things, and this module reproduces that
//! literally rather than "fixing" it:
//!
//! - in a **`promise`** event, `id` is the promise's id and `promise` is the free
//!   text of the thing owed;
//! - in an **`amend`**, **`resolve`** or **`assess`** event, `promise` is the **id
//!   being amended, resolved or assessed**.
//!
//! It reads correctly in both places — "the promise" is the text when you are
//! speaking it and the referent when you are pointing at it — and the log is the
//! interoperability surface, so it is not ours to tidy.

use serde::{Deserialize, Serialize};

use crate::record::Promise;

/// The asserted state §3 gives us today. Assessments are speech acts, so the field
/// is a plain string rather than a closed enum: pinki *publishes* judgments and never
/// computes them, and a vocabulary check here would make pinki the arbiter of which
/// judgments may be expressed — plus it would hard-fail on a joined ledger written by
/// something that knows an assessment word we do not.
pub const VIOLATED: &str = "violated";

/// One line of the ledger.
///
/// `ts` is kept as the raw ISO-8601 string it arrived as. The fold never compares
/// event timestamps — only `until` against an injected `now` — so parsing them would
/// buy nothing and would let a ledger line become unreadable over a field nothing
/// reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub ts: String,
    #[serde(flatten)]
    pub body: EventBody,
}

impl Event {
    /// Build an event with an explicit timestamp.
    pub fn new(ts: impl Into<String>, body: EventBody) -> Self {
        Self {
            ts: ts.into(),
            body,
        }
    }

    /// The promise id this event is about — the record's `id` for a `promise` event,
    /// the referenced id for `amend`, `resolve` and `assess`.
    pub fn subject(&self) -> &str {
        match &self.body {
            EventBody::Promise(p) => &p.id,
            EventBody::Amend { promise, .. }
            | EventBody::Resolve { promise, .. }
            | EventBody::Assess { promise, .. } => promise,
        }
    }

    /// The resolution carried by a `resolve` event, if this is one.
    pub fn resolution(&self) -> Option<&Resolution> {
        match &self.body {
            EventBody::Resolve { resolution, .. } => Some(resolution),
            _ => None,
        }
    }
}

/// The body of an event, internally tagged on `type` so it flattens into [`Event`]
/// with `ts` ahead of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum EventBody {
    /// Speaks a promise into existence. The body *is* the §1 record, verbatim — one
    /// definition of the field set, so the log and the A2A `metadata` block cannot
    /// drift apart.
    Promise(Promise),
    /// Moves a promise's horizon — §4's `amend`, and the answer to §8's second open
    /// question. It never rewrites the `promise` event: the declaration stays exactly
    /// as it was spoken, and the deadline history is the log's to keep.
    Amend {
        /// The id whose horizon is moving.
        promise: String,
        /// Who is moving it. §8.2 admits only the debtor; the check is the `amend`
        /// verb's, not the fold's, for the same reason every other rule is enforced at
        /// the edge — a joined ledger may carry one written by something stricter or
        /// looser than this build.
        by: String,
        /// The new deadline, ISO-8601.
        until: String,
        /// Why the deadline moved. Encouraged, not required — an escalation ladder
        /// amends on a clock and has one reason for every rung.
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// Ends a promise.
    Resolve {
        /// The id being resolved.
        promise: String,
        #[serde(flatten)]
        resolution: Resolution,
    },
    /// Publishes an assessment. Never terminal: §4 — "an assessed promise stays
    /// exactly as open as it was."
    Assess {
        /// The id being assessed.
        promise: String,
        state: String,
        observer: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
}

impl EventBody {
    /// A `promise` event body.
    pub fn promise(record: Promise) -> Self {
        EventBody::Promise(record)
    }

    /// An `amend` event body: the same promise, a new horizon.
    pub fn amend(
        promise: impl Into<String>,
        by: impl Into<String>,
        until: impl Into<String>,
        reason: Option<String>,
    ) -> Self {
        EventBody::Amend {
            promise: promise.into(),
            by: by.into(),
            until: until.into(),
            reason,
        }
    }

    /// `resolve … --satisfied`. Issued by the debtor; `evidence` is non-optional
    /// (§4), and the type says so.
    pub fn satisfied(
        promise: impl Into<String>,
        by: impl Into<String>,
        evidence: Vec<String>,
    ) -> Self {
        EventBody::Resolve {
            promise: promise.into(),
            resolution: Resolution::Satisfied {
                by: by.into(),
                evidence,
            },
        }
    }

    /// `resolve … --cancelled`. Issued by the debtor; a reason is required.
    pub fn cancelled(
        promise: impl Into<String>,
        by: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        EventBody::Resolve {
            promise: promise.into(),
            resolution: Resolution::Cancelled {
                by: by.into(),
                reason: reason.into(),
            },
        }
    }

    /// `resolve … --released`. Issued by the creditor; a reason is encouraged, not
    /// required.
    pub fn released(
        promise: impl Into<String>,
        by: impl Into<String>,
        reason: Option<String>,
    ) -> Self {
        EventBody::Resolve {
            promise: promise.into(),
            resolution: Resolution::Released {
                by: by.into(),
                reason,
            },
        }
    }

    /// An `assess` event publishing [`VIOLATED`].
    pub fn violated(
        promise: impl Into<String>,
        observer: impl Into<String>,
        note: Option<String>,
    ) -> Self {
        EventBody::Assess {
            promise: promise.into(),
            state: VIOLATED.to_string(),
            observer: observer.into(),
            note,
        }
    }
}

/// How a promise ended — DESIGN.md §4's resolve table, as a type.
///
/// Tagged on `as` and flattened into the `resolve` body, which puts the wire keys in
/// §4's order: `promise`, `as`, `by`, then the outcome's own payload. Each variant
/// carries `by` because §4's "Issued by" column differs per outcome — the debtor
/// satisfies or cancels, the creditor releases — so it is genuinely a different
/// party's name in each case, not a repeated field.
///
/// Modelling the payload per outcome is what makes the §4 requirements
/// unconstructible to violate: there is no `Satisfied` without an `evidence` list and
/// no `Cancelled` without a reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "as", rename_all = "lowercase")]
pub enum Resolution {
    /// Resolved by the debtor, with evidence: one or more outward-pointing
    /// references. An empty list is malformed — §4, "done that points at nothing is
    /// the exact failure this whole category of tool exists to catch" — but the
    /// emptiness check belongs to the verb that accepts input, not to the fold, which
    /// must be able to read a ledger somebody else wrote.
    Satisfied { by: String, evidence: Vec<String> },
    /// Resolved by the debtor, with a reason.
    Cancelled { by: String, reason: String },
    /// Resolved by the creditor letting the debtor off.
    Released {
        by: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

impl Resolution {
    /// The `as` word, as it appears on the wire.
    pub fn as_str(&self) -> &'static str {
        match self {
            Resolution::Satisfied { .. } => "satisfied",
            Resolution::Cancelled { .. } => "cancelled",
            Resolution::Released { .. } => "released",
        }
    }

    /// Who issued the resolve. pinki records the claim; it does not check that this
    /// matches the promise's `by` (§4: "It records who claimed what").
    pub fn by(&self) -> &str {
        match self {
            Resolution::Satisfied { by, .. }
            | Resolution::Cancelled { by, .. }
            | Resolution::Released { by, .. } => by,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> Promise {
        Promise {
            id: "pnk_4f3a91".into(),
            promise: "hand back a reviewed schema".into(),
            by: "…/reviewer".into(),
            to: "…/author".into(),
            on: Some("pnk_0c2b77".into()),
            until: "2026-09-01T17:00:00Z".into(),
            task: None,
        }
    }

    #[test]
    fn promise_event_matches_design_section_4() {
        let ev = Event::new("2026-08-28T20:14:03Z", EventBody::promise(record()));
        assert_eq!(
            serde_json::to_string(&ev).unwrap(),
            r#"{"ts":"2026-08-28T20:14:03Z","type":"promise","id":"pnk_4f3a91","promise":"hand back a reviewed schema","by":"…/reviewer","to":"…/author","on":"pnk_0c2b77","until":"2026-09-01T17:00:00Z"}"#
        );
    }

    #[test]
    fn satisfied_event_matches_design_section_4() {
        let ev = Event::new(
            "2026-09-01T16:02:11Z",
            EventBody::satisfied(
                "pnk_4f3a91",
                "…/reviewer",
                vec!["https://example.org/reviews/91".into()],
            ),
        );
        assert_eq!(
            serde_json::to_string(&ev).unwrap(),
            r#"{"ts":"2026-09-01T16:02:11Z","type":"resolve","promise":"pnk_4f3a91","as":"satisfied","by":"…/reviewer","evidence":["https://example.org/reviews/91"]}"#
        );
    }

    #[test]
    fn amend_event_matches_design_section_4() {
        let ev = Event::new(
            "2026-09-01T17:05:00Z",
            EventBody::amend(
                "pnk_4f3a91",
                "…/reviewer",
                "2026-09-01T17:15:00Z",
                Some("nudge 1".into()),
            ),
        );
        assert_eq!(
            serde_json::to_string(&ev).unwrap(),
            r#"{"ts":"2026-09-01T17:05:00Z","type":"amend","promise":"pnk_4f3a91","by":"…/reviewer","until":"2026-09-01T17:15:00Z","reason":"nudge 1"}"#
        );
    }

    #[test]
    fn amend_omits_an_absent_reason() {
        let ev = Event::new(
            "2026-09-01T17:05:00Z",
            EventBody::amend("pnk_4f3a91", "…/reviewer", "2026-09-01T17:15:00Z", None),
        );
        let json = serde_json::to_string(&ev).unwrap();
        assert!(!json.contains("reason"), "{json}");
        assert!(
            json.ends_with(r#""until":"2026-09-01T17:15:00Z"}"#),
            "{json}"
        );
    }

    #[test]
    fn an_amend_is_not_a_resolution() {
        // It moves the horizon; it does not end anything. §4 — the `resolve` table is
        // the only way out.
        let ev = Event::new(
            "2026-09-01T17:05:00Z",
            EventBody::amend("pnk_4f3a91", "…/reviewer", "2026-09-01T17:15:00Z", None),
        );
        assert!(ev.resolution().is_none());
        assert_eq!(ev.subject(), "pnk_4f3a91");
    }

    #[test]
    fn assess_event_matches_design_section_4() {
        let ev = Event::new(
            "2026-09-02T09:00:00Z",
            EventBody::violated(
                "pnk_88de10",
                "…/author",
                Some("nothing shipped, no reason given".into()),
            ),
        );
        assert_eq!(
            serde_json::to_string(&ev).unwrap(),
            r#"{"ts":"2026-09-02T09:00:00Z","type":"assess","promise":"pnk_88de10","state":"violated","observer":"…/author","note":"nothing shipped, no reason given"}"#
        );
    }

    #[test]
    fn cancelled_and_released_shapes() {
        let c = Event::new(
            "2026-08-30T00:00:00Z",
            EventBody::cancelled("pnk_1", "…/debtor", "upstream schema was withdrawn"),
        );
        assert_eq!(
            serde_json::to_string(&c).unwrap(),
            r#"{"ts":"2026-08-30T00:00:00Z","type":"resolve","promise":"pnk_1","as":"cancelled","by":"…/debtor","reason":"upstream schema was withdrawn"}"#
        );

        let r = Event::new(
            "2026-08-30T00:00:00Z",
            EventBody::released("pnk_1", "…/creditor", None),
        );
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            r#"{"ts":"2026-08-30T00:00:00Z","type":"resolve","promise":"pnk_1","as":"released","by":"…/creditor"}"#
        );

        let rr = Event::new(
            "2026-08-30T00:00:00Z",
            EventBody::released("pnk_1", "…/creditor", Some("no longer needed".into())),
        );
        assert!(serde_json::to_string(&rr)
            .unwrap()
            .ends_with(r#""as":"released","by":"…/creditor","reason":"no longer needed"}"#));
    }

    #[test]
    fn assess_omits_absent_note() {
        let ev = Event::new(
            "2026-09-02T09:00:00Z",
            EventBody::violated("pnk_1", "…/author", None),
        );
        let json = serde_json::to_string(&ev).unwrap();
        assert!(!json.contains("note"), "{json}");
    }

    #[test]
    fn every_event_type_round_trips() {
        for ev in [
            Event::new("2026-08-28T20:14:03Z", EventBody::promise(record())),
            Event::new(
                "2026-09-01T16:02:11Z",
                EventBody::satisfied("pnk_4f3a91", "…/reviewer", vec!["sha:abc".into()]),
            ),
            Event::new(
                "2026-09-01T16:02:11Z",
                EventBody::cancelled("pnk_4f3a91", "…/reviewer", "withdrawn"),
            ),
            Event::new(
                "2026-09-01T16:02:11Z",
                EventBody::released("pnk_4f3a91", "…/author", Some("fine by me".into())),
            ),
            Event::new(
                "2026-09-02T09:00:00Z",
                EventBody::violated("pnk_88de10", "…/author", None),
            ),
            Event::new(
                "2026-09-01T17:05:00Z",
                EventBody::amend(
                    "pnk_4f3a91",
                    "…/reviewer",
                    "2026-09-01T17:15:00Z",
                    Some("nudge 1".into()),
                ),
            ),
            Event::new(
                "2026-09-01T17:20:00Z",
                EventBody::amend("pnk_4f3a91", "…/reviewer", "2026-09-01T17:30:00Z", None),
            ),
        ] {
            let text = serde_json::to_string(&ev).unwrap();
            let back: Event = serde_json::from_str(&text).unwrap_or_else(|e| {
                panic!("failed to read back {text}: {e}");
            });
            assert_eq!(ev, back, "round-trip changed {text}");
        }
    }

    #[test]
    fn deserializes_design_section_4_lines_verbatim() {
        let lines = [
            r#"{"ts":"2026-08-28T20:14:03Z","type":"promise","id":"pnk_4f3a91","promise":"hand back a reviewed schema","by":"…/reviewer","to":"…/author","on":"pnk_0c2b77","until":"2026-09-01T17:00:00Z"}"#,
            r#"{"ts":"2026-09-01T16:02:11Z","type":"resolve","promise":"pnk_4f3a91","as":"satisfied","by":"…/reviewer","evidence":["https://example.org/reviews/91"]}"#,
            r#"{"ts":"2026-09-02T09:00:00Z","type":"assess","promise":"pnk_88de10","state":"violated","observer":"…/author","note":"nothing shipped, no reason given"}"#,
            r#"{"ts":"2026-09-01T15:00:00Z","type":"amend","promise":"pnk_4f3a91","by":"…/reviewer","until":"2026-09-01T18:00:00Z","reason":"the schema landed late"}"#,
        ];
        let events: Vec<Event> = lines
            .iter()
            .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{l}: {e}")))
            .collect();
        assert_eq!(events[0].subject(), "pnk_4f3a91");
        assert_eq!(events[1].subject(), "pnk_4f3a91");
        assert_eq!(events[1].resolution().unwrap().as_str(), "satisfied");
        assert_eq!(events[1].resolution().unwrap().by(), "…/reviewer");
        assert_eq!(events[2].subject(), "pnk_88de10");
        assert!(events[2].resolution().is_none());
        assert_eq!(events[3].subject(), "pnk_4f3a91");
        assert!(events[3].resolution().is_none());
        assert!(matches!(
            &events[3].body,
            EventBody::Amend { until, reason: Some(reason), .. }
                if until == "2026-09-01T18:00:00Z" && reason == "the schema landed late"
        ));
    }
}
