//! A read-only projection of the fold for tools that draw promises.
//!
//! The ledger stays the source of truth. This document carries the record, its
//! horizons and the speech acts around it; it does not turn an assessment into a
//! computed state or infer that an overdue promise was broken.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::str::FromStr;

use jiff::Timestamp;
use serde::Serialize;

use crate::cli::{ExportArgs, ExportFormat};
use crate::event::{EventBody, Resolution};
use crate::ledger;
use crate::record::{Meta, Promise};
use crate::state::{fold, PromiseView, State};
use crate::verbs::Fail;

#[derive(Serialize)]
struct Document<'a> {
    format: &'static str,
    version: u8,
    summary: Summary,
    promises: Vec<ExportPromise<'a>>,
}

#[derive(Serialize)]
struct Summary {
    total: usize,
    states: BTreeMap<&'static str, usize>,
}

#[derive(Serialize)]
struct ExportPromise<'a> {
    #[serde(flatten)]
    record: Cow<'a, Promise>,
    original_until: &'a str,
    amendments: Vec<Amendment<'a>>,
    state: &'static str,
    resolution: Option<ResolutionView<'a>>,
    assessments: Vec<Assessment<'a>>,
    created_at: &'a str,
}

#[derive(Serialize)]
struct Amendment<'a> {
    until: &'a str,
    by: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'a str>,
    at: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    meta: Option<&'a Meta>,
}

#[derive(Serialize)]
struct ResolutionView<'a> {
    #[serde(rename = "as")]
    kind: &'a str,
    by: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    evidence: Option<&'a [String]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'a str>,
    at: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    meta: Option<&'a Meta>,
}

#[derive(Serialize)]
struct Assessment<'a> {
    state: &'a str,
    observer: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<&'a str>,
    at: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    meta: Option<&'a Meta>,
}

/// Emit the chosen projection without changing the ledger.
pub fn run(args: ExportArgs) -> Result<(), Fail> {
    let now = match args.now.as_deref() {
        Some(value) => parse_instant(value, "--now")?,
        None => Timestamp::now(),
    };
    let since = args
        .since
        .as_deref()
        .map(|value| parse_instant(value, "--since"))
        .transpose()?;
    let events = ledger::read().map_err(|e| Fail::Op(e.to_string()))?;
    let folded = fold(&events, now);

    let mut selected = Vec::new();
    for view in folded.iter() {
        if !args.state.is_empty() && !args.state.iter().any(|state| state == view.state.as_str()) {
            continue;
        }
        if args
            .debtor
            .as_deref()
            .is_some_and(|by| by != view.record.by)
        {
            continue;
        }
        if args
            .creditor
            .as_deref()
            .is_some_and(|to| to != view.record.to)
        {
            continue;
        }
        if let Some(since) = since {
            let created_at = view.horizons[0].ts;
            let created = Timestamp::from_str(created_at).map_err(|e| {
                Fail::Op(format!(
                    "cannot apply --since: promise `{}` has an unreadable declaration timestamp `{created_at}`: {e}",
                    view.id()
                ))
            })?;
            if created < since {
                continue;
            }
        }
        selected.push(view);
    }

    // Timestamps in a hand-written ledger may carry different offsets. Compare
    // instants when possible, then the raw text and id for a stable total order.
    selected.sort_by_key(|view| (time_key(view.horizons[0].ts), view.id()));
    let promises: Vec<ExportPromise<'_>> = selected.into_iter().map(project).collect();

    match args.format {
        ExportFormat::Json => {
            let mut states = BTreeMap::new();
            for state in [
                State::Conditional,
                State::Detached,
                State::Overdue,
                State::Satisfied,
                State::Cancelled,
                State::Released,
                State::Expired,
            ] {
                states.insert(state.as_str(), 0);
            }
            for promise in &promises {
                *states
                    .get_mut(promise.state)
                    .expect("every folded state is named") += 1;
            }
            let document = Document {
                format: "pinki-export",
                version: 1,
                summary: Summary {
                    total: promises.len(),
                    states,
                },
                promises,
            };
            println!("{}", encode(&document)?);
        }
        ExportFormat::Jsonl => {
            // No header or footer: every line has exactly the promise shape.
            for promise in &promises {
                println!("{}", encode(promise)?);
            }
        }
    }
    Ok(())
}

fn parse_instant(value: &str, flag: &str) -> Result<Timestamp, Fail> {
    Timestamp::from_str(value.trim()).map_err(|e| {
        Fail::Usage(format!(
            "{flag} `{value}` is not an ISO-8601 instant with an offset: {e}"
        ))
    })
}

fn time_key(ts: &str) -> (Option<Timestamp>, &str) {
    (Timestamp::from_str(ts).ok(), ts)
}

fn project<'a>(view: &'a PromiseView<'a>) -> ExportPromise<'a> {
    let mut amendments: Vec<Amendment<'_>> = view
        .amendments()
        .iter()
        .map(|horizon| Amendment {
            until: horizon.until,
            by: horizon.by,
            reason: horizon.reason,
            at: horizon.ts,
            meta: horizon.meta,
        })
        .collect();
    amendments.sort_by_key(|amendment| time_key(amendment.at));

    let resolution = view.resolution.and_then(|event| {
        view.resolution_kind().map(|kind| {
            let (evidence, reason) = match kind {
                Resolution::Satisfied { evidence, .. } => (Some(evidence.as_slice()), None),
                Resolution::Cancelled { reason, .. } => (None, Some(reason.as_str())),
                Resolution::Released { reason, .. } => (None, reason.as_deref()),
            };
            ResolutionView {
                kind: kind.as_str(),
                by: kind.by(),
                evidence,
                reason,
                at: &event.ts,
                meta: event.meta(),
            }
        })
    });

    let mut assessments: Vec<Assessment<'_>> = view
        .assessments
        .iter()
        .filter_map(|event| match &event.body {
            EventBody::Assess {
                state,
                observer,
                note,
                meta,
                ..
            } => Some(Assessment {
                state,
                observer,
                note: note.as_deref(),
                at: &event.ts,
                meta: meta.as_ref(),
            }),
            _ => None,
        })
        .collect();
    assessments.sort_by_key(|assessment| time_key(assessment.at));

    ExportPromise {
        record: view.current_record(),
        original_until: view.record.until.as_str(),
        amendments,
        state: view.state.as_str(),
        resolution,
        assessments,
        created_at: view.horizons[0].ts,
    }
}

fn encode<T: Serialize>(value: &T) -> Result<String, Fail> {
    serde_json::to_string(value)
        .map_err(|e| Fail::Op(format!("could not encode export as JSON: {e}")))
}
