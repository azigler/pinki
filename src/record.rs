//! The promise record — DESIGN.md §1.
//!
//! One record type. No status field (state is computed from the log, never stored),
//! no `created` field (the `promise` event's own `ts` carries it), and none of the
//! priority/tag/assignee/project apparatus a task tracker would grow.
//!
//! ## The one open door
//!
//! [`Promise::meta`] is an optional JSON **object**, opaque: pinki never reads it,
//! the fold never sees it, and it round-trips verbatim through the log, `show`, `ls`
//! and the A2A block. It exists so an adopting system can carry the provenance its
//! own rows already have — which component declared this, on whose behalf, under
//! which policy — instead of projecting it away at the seam (§1).
//!
//! The arithmetic fields above it stay **closed**, and that is the property worth
//! protecting: state is a fold over `on`, `until` and the resolve events, so a field
//! the fold consults is a field two implementations can disagree about. `meta` is
//! not one, by construction — nothing in [`crate::state`] can even see it.
//!
//! The rules that make it safe are enforced at the edge, in [`crate::verbs`], not
//! here: it must be an object, it may not be empty, and it is capped. Deserializing
//! is deliberately more permissive — the fold has to be able to read a ledger
//! somebody else wrote.

use jiff::{SignedDuration, Span, SpanRelativeTo};
use serde::{Deserialize, Serialize};

/// Opaque caller-supplied provenance — [`Promise::meta`].
///
/// A [`serde_json::Map`], so "it is an object" is a property of the type rather than
/// a check somebody has to remember. That map is a `BTreeMap`: keys come back
/// **sorted**, not in the order they were written. Values, types and nesting survive
/// exactly; key order does not, and §1 says so rather than implying more fidelity
/// than there is.
pub type Meta = serde_json::Map<String, serde_json::Value>;

/// The longest `within` pinki accepts: 3650 days (about ten years), in seconds.
pub const WITHIN_MAX_SECS: i64 = 3650 * 86_400;

/// Parse a stored `within` — an ISO-8601 duration — into a fixed length of time.
///
/// Only units with a fixed length are accepted: weeks and days count as 24 hours, and
/// years and months are refused because a month has no one length. The result is whole
/// seconds, strictly positive and at most [`WITHIN_MAX_SECS`]. `verbs` calls this at the edge to say why a value is
/// refused; the fold calls it again and reads any `Err` as "no relative horizon", so a
/// hand-written ledger cannot make the fold fail.
pub fn within_duration(text: &str) -> Result<SignedDuration, String> {
    let text = text.trim();
    if !text.starts_with(['P', 'p']) {
        return Err(format!(
            "`{text}` is not an ISO-8601 duration; it must start with P (PT24H, PT45M, P2D)"
        ));
    }
    let span: Span = text
        .parse()
        .map_err(|e| format!("`{text}` is not an ISO-8601 duration: {e}"))?;
    let duration = span
        .to_duration(SpanRelativeTo::days_are_24_hours())
        .map_err(|_| {
            format!("`{text}` has years or months, which have no fixed length; use d, h or m")
        })?;
    if duration.subsec_nanos() != 0 {
        return Err(format!("`{text}` is finer than one second"));
    }
    if duration <= SignedDuration::ZERO {
        return Err(format!("`{text}` is not a positive duration"));
    }
    if duration.as_secs() > WITHIN_MAX_SECS {
        return Err(format!("`{text}` is longer than 3650 days"));
    }
    Ok(duration)
}

/// The one stored form of a `within`: parse to whole seconds, then write `P<n>D` when
/// the count divides by 86400, else `PT<n>H` by 3600, else `PT<n>M` by 60, else
/// `PT<n>S`. `P1W`, `PT24H0M0S`, `PT86400S` and `PT007H` all say what `P7D`, `P1D`,
/// `P1D` and `PT7H` say. Fractions and commas are refused: `PT1.5H` is a decimal
/// hour, and one spelling of 90 minutes is enough.
pub fn canonical_within(text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.contains(['.', ',']) {
        return Err(format!(
            "`{text}` has a fraction or a comma; write whole units (PT90M, not PT1.5H)"
        ));
    }
    let secs = within_duration(text)?.as_secs();
    Ok(if secs % 86_400 == 0 {
        format!("P{}D", secs / 86_400)
    } else if secs % 3_600 == 0 {
        format!("PT{}H", secs / 3_600)
    } else if secs % 60 == 0 {
        format!("PT{}M", secs / 60)
    } else {
        format!("PT{secs}S")
    })
}

/// A promise: who owes what to whom, by when.
///
/// Field order here is load-bearing. serde emits struct fields in declaration
/// order, and §1's example — and the `metadata` block in A2A-EXTENSION.md
/// §Surface 3, which carries this record verbatim — reads
/// `id, promise, by, to, on, until, within, task` — with `meta`, when there is one, last.
///
/// `Eq` is deliberately absent from here down (and from [`crate::event::Event`],
/// which holds this). `meta` is arbitrary JSON, and [`serde_json::Value`] is only
/// `PartialEq` because JSON numbers include floats. Nothing keys a map or a set on a
/// record, so the bound was never load-bearing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Deadline, ISO-8601. Required — §1: a promise without one is a wish. With
    /// [`Promise::within`] it is the debtor's outer bound: "at the latest".
    pub until: String,
    /// A horizon relative to the antecedent: an ISO-8601 duration (`PT24H`, `P2D`)
    /// that runs from the moment `on` is satisfied. Only meaningful with `on`; the
    /// fold reads it, and an old reader ignores it and sees `until`. §1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub within: Option<String>,
    /// The A2A `Task.id` this promise is *about*, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    /// Opaque provenance, carried and never interpreted. Absent when nobody supplied
    /// any — `"meta":{}` on every line would be noise, so the key is simply not
    /// written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<Meta>,
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
            within: None,
            task: Some("a2a-task-9c1f0e".into()),
            meta: None,
        }
    }

    /// Parse a `meta` object out of its JSON text, the way the edge does.
    fn meta(json: &str) -> Meta {
        serde_json::from_str(json).unwrap_or_else(|e| panic!("bad test meta {json}: {e}"))
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
        assert!(
            !json.contains("\"meta\""),
            "absent `meta` must not appear at all — not even as an empty object: {json}"
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

    #[test]
    fn meta_serializes_last_and_verbatim() {
        let p = Promise {
            meta: Some(meta(
                r#"{"by":"offboard","policy":{"nudge":["t-24h","t-2h"]},"attempt":2}"#,
            )),
            ..full()
        };
        let json = serde_json::to_string(&p).unwrap();
        // §1's field order, with the open door at the end of the line.
        assert_eq!(
            json,
            r#"{"id":"pnk_4f3a91","promise":"hand back a reviewed schema","by":"https://example.org/agents/reviewer","to":"https://example.org/agents/author","on":"pnk_0c2b77","until":"2026-09-01T17:00:00Z","task":"a2a-task-9c1f0e","meta":{"attempt":2,"by":"offboard","policy":{"nudge":["t-24h","t-2h"]}}}"#
        );
    }

    #[test]
    fn meta_round_trips_with_its_nesting_and_types_intact() {
        let p = Promise {
            meta: Some(meta(
                r#"{"n":1,"f":1.5,"t":true,"nil":null,"list":[1,"two",{"three":3}],"deep":{"a":{"b":{"c":"d"}}}}"#,
            )),
            ..full()
        };
        let text = serde_json::to_string(&p).unwrap();
        let back: Promise = serde_json::from_str(&text).unwrap();
        assert_eq!(p, back);
        // And a second trip changes nothing, which is what "round-trips verbatim"
        // has to mean for a consumer reading the file twice.
        assert_eq!(text, serde_json::to_string(&back).unwrap());
    }

    #[test]
    fn meta_keys_come_back_sorted_not_as_written() {
        // The honest limit, tested rather than implied: `meta` is a JSON object, and
        // this map is a BTreeMap. Values survive exactly; key order is canonicalized.
        let p = Promise {
            meta: Some(meta(r#"{"zeta":1,"alpha":2}"#)),
            ..full()
        };
        let json = serde_json::to_string(&p).unwrap();
        assert!(
            json.ends_with(r#""meta":{"alpha":2,"zeta":1}}"#),
            "meta keys should be sorted: {json}"
        );
    }

    #[test]
    fn a_record_read_back_from_a_ledger_may_carry_meta_nobody_validated() {
        // Deserializing is permissive on purpose: the fold has to read a ledger
        // somebody else wrote, including one whose meta breaks the edge's rules.
        let line = r#"{"id":"pnk_4f3a91","promise":"x","by":"a","to":"b","until":"2026-09-01T17:00:00Z","meta":{}}"#;
        let back: Promise = serde_json::from_str(line).unwrap();
        assert_eq!(back.meta, Some(Meta::new()));
    }
    #[test]
    fn within_serializes_after_until_and_is_absent_when_not_given() {
        let p = Promise {
            within: Some("PT24H".into()),
            ..full()
        };
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(
            json,
            r#"{"id":"pnk_4f3a91","promise":"hand back a reviewed schema","by":"https://example.org/agents/reviewer","to":"https://example.org/agents/author","on":"pnk_0c2b77","until":"2026-09-01T17:00:00Z","within":"PT24H","task":"a2a-task-9c1f0e"}"#
        );
        assert!(!serde_json::to_string(&full()).unwrap().contains("within"));
        let back: Promise = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn within_duration_reads_fixed_length_iso_durations() {
        let secs = |text: &str| within_duration(text).unwrap().as_secs();
        assert_eq!(secs("PT24H"), 86_400);
        assert_eq!(secs("PT45M"), 2_700);
        assert_eq!(secs("P2D"), 172_800);
        assert_eq!(secs("P1W"), 604_800);
        assert_eq!(secs("P1DT12H"), 129_600);
        assert_eq!(secs("P3650D"), WITHIN_MAX_SECS);
    }

    #[test]
    fn within_duration_refuses_over_3650_days() {
        assert!(within_duration("P3651D").is_err());
        assert!(within_duration("PT87601H").is_err());
    }

    #[test]
    fn within_duration_refuses_what_has_no_fixed_positive_length() {
        for bad in [
            "", "24h", "PT0S", "PT0H", "-PT1H", "P1M", "P1Y", "PT0.5S", "bogus", "P",
        ] {
            assert!(within_duration(bad).is_err(), "{bad:?} should be refused");
        }
    }
}
