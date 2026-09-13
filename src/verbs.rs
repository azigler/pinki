//! The six verbs — DESIGN.md §5.
//!
//! Every verb is the same three moves: read the ledger, fold it into states at
//! `now`, and then either print or append exactly one event. Nothing here holds
//! state between runs, nothing caches, and nothing touches anything but the ledger
//! file (§5: "There is no state anywhere else, no daemon, and no network.").
//!
//! ## Where validation lives
//!
//! The strictness DESIGN.md asks for is enforced *here*, at the edge where input
//! arrives, and deliberately not in [`crate::state`]'s fold — which must be able to
//! read a ledger somebody else wrote, including one that breaks these rules. So:
//! `--evidence` must be present and non-blank for `--satisfied` (§4), `--until` must
//! parse as an instant, and a stdin record may not carry a field pinki does not know.
//!
//! ## Exit codes
//!
//! See [`Fail`]. `2` is malformed input, `1` is an operational failure, `0` is
//! success — and every error message goes to stderr, never stdout, so a `--json`
//! pipeline never receives prose.

use std::collections::HashSet;
use std::fmt;
use std::io::{self, IsTerminal, Read};
use std::str::FromStr;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::a2a;
use crate::cli::{
    A2aCommand, AssessArgs, Command, LsArgs, PromiseArgs, ResolveArgs, Scope, ShowArgs,
};
use crate::event::{Event, EventBody, Resolution};
use crate::id;
use crate::ledger;
use crate::record::Promise;
use crate::state::{fold, PromiseView, State};

/// A command that did not succeed.
///
/// Two kinds, because they mean different things to whoever is reading:
///
/// - [`Fail::Usage`] — the command as typed is malformed. Exit **2**, which is also
///   what clap's own parse errors exit with, so a caller sees one code for "you asked
///   for something that isn't a command."
/// - [`Fail::Op`] — the command was well formed and could not be carried out: an id
///   nothing declares, a promise already resolved, a ledger that will not read. Exit
///   **1**.
///
/// Success is **0**. Errors are printed to stderr by `main`.
#[derive(Debug)]
pub enum Fail {
    /// Malformed input or a usage error. Exit code 2.
    Usage(String),
    /// The command was well formed but could not be carried out. Exit code 1.
    Op(String),
}

impl Fail {
    /// The process exit code for this failure.
    pub fn code(&self) -> i32 {
        match self {
            Fail::Usage(_) => 2,
            Fail::Op(_) => 1,
        }
    }
}

impl fmt::Display for Fail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Fail::Usage(message) | Fail::Op(message) => f.write_str(message),
        }
    }
}

/// Dispatch one parsed command.
pub fn run(command: Command) -> Result<(), Fail> {
    match command {
        Command::Promise(args) => promise(args),
        Command::Resolve(args) => resolve(args),
        Command::Assess(args) => assess(args),
        Command::Ls(args) => ls(args),
        Command::Show(args) => show(args),
        Command::A2a { command } => match command {
            A2aCommand::Card => a2a_card(),
            A2aCommand::Task { id } => a2a_task(&id),
        },
    }
}

// ---------------------------------------------------------------- promise

/// A promise as it arrives from stdin — §5: "`pinki promise` reads JSON on stdin like
/// everything else."
///
/// A local mirror of [`Promise`] rather than the record itself, for two reasons:
/// `id` is optional on the way in (pinki mints one), and `deny_unknown_fields` is a
/// property of *input*, not of the record. A record deserialized without it would
/// drop a typo'd key silently — `"untl"` becoming a missing deadline — which is
/// precisely the failure this tool exists to catch.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Incoming {
    #[serde(default)]
    id: Option<String>,
    promise: String,
    by: String,
    to: String,
    #[serde(default)]
    on: Option<String>,
    until: String,
    #[serde(default)]
    task: Option<String>,
}

fn promise(args: PromiseArgs) -> Result<(), Fail> {
    let incoming = match args.text {
        // Flag mode. Chosen by the presence of TEXT, so stdin is never read here and
        // a command that looks complete on the line behaves as it looks.
        Some(text) => Incoming {
            id: args.id,
            promise: text,
            by: required_flag(args.by, "--by")?,
            to: required_flag(args.to, "--to")?,
            on: args.on,
            until: required_flag(args.until, "--until")?,
            task: args.task,
        },
        None => read_incoming_from_stdin()?,
    };

    let text = non_blank(&incoming.promise, "the promise text")?;
    let by = non_blank(&incoming.by, "`by`")?;
    let to = non_blank(&incoming.to, "`to`")?;
    let until = parse_until(&incoming.until)?;

    // §1's id policy, checked before the ledger is consulted for the same reason
    // `--until` is: an id pinki will not accept is malformed whether or not the ledger
    // happens to hold it. A supplied id is *opaque* — any non-blank string with no
    // whitespace and no control characters, bounded, and not squatting the minted
    // `pnk_` namespace — because an adopter's existing ids are already referenced from
    // elsewhere, and §7's join needs both parties' keys, not pinki-minted ones.
    let supplied = match incoming.id {
        Some(given) => {
            let given = given.trim().to_string();
            id::check_supplied(&given)
                .map_err(|e| Fail::Usage(format!("`{given}` will not do as an id: {e}")))?;
            Some(given)
        }
        None => None,
    };

    let mut events = read_ledger()?;
    let now = now_to_the_second();

    let id = match supplied {
        // Being permissive about the *shape* is safe because uniqueness is not
        // negotiable: this is §4's first-declaration-wins guard, and it does not care
        // which space the id came from. A supplied id that collides with one pinki
        // minted earlier is refused by exactly this line.
        Some(given) => {
            if fold(&events, now).get(&given).is_some() {
                return Err(Fail::Op(format!(
                    "`{given}` is already declared in {} — ids are stable, so pinki will not \
                     redeclare one (§4: the first declaration wins)",
                    ledger::path().display()
                )));
            }
            given
        }
        // Ask the ledger which ids are spoken for rather than trusting the size of
        // the id space. `subject()` covers ids named by a dangling resolve or assess
        // too, which a joined ledger routinely carries.
        None => {
            let taken: HashSet<String> = events.iter().map(|e| e.subject().to_string()).collect();
            id::mint(&taken).map_err(|e| Fail::Op(e.to_string()))?
        }
    };

    let on = match incoming.on {
        Some(antecedent) => {
            let antecedent = non_blank(&antecedent, "`on`")?;
            // §3's table and the fold both read an unknown antecedent as
            // `conditional` — "an id this ledger has never heard of has certainly not
            // satisfied" — so this is a warning, not a refusal. Ledgers get joined,
            // and half a conversation is a normal thing to hold.
            if fold(&events, now).get(&antecedent).is_none() {
                eprintln!(
                    "pinki: warning: nothing in {} declares `{antecedent}`; recording the \
                     promise as conditional on it anyway",
                    ledger::path().display()
                );
            }
            Some(antecedent)
        }
        None => None,
    };

    let record = Promise {
        id: id.clone(),
        promise: text,
        by,
        to,
        on,
        until,
        task: match incoming.task {
            Some(task) => Some(non_blank(&task, "`task`")?),
            None => None,
        },
    };

    let event = Event::new(now.to_string(), EventBody::promise(record));
    append(&event)?;
    events.push(event);

    report(&events, now, &id);
    Ok(())
}

/// Read one record as JSON from stdin.
///
/// A terminal on stdin means nothing was piped in — the user typed `pinki promise`
/// with no text and no input, and blocking on a tty they did not ask to fill would
/// look like a hang.
fn read_incoming_from_stdin() -> Result<Incoming, Fail> {
    const FORMS: &str = "give the promise text as an argument \
         (pinki promise TEXT --by WHO --to WHO --until ISO), or pipe one record as JSON on stdin";

    if io::stdin().is_terminal() {
        return Err(Fail::Usage(format!("no promise given: {FORMS}")));
    }

    let mut text = String::new();
    io::stdin()
        .read_to_string(&mut text)
        .map_err(|e| Fail::Usage(format!("could not read the record from stdin: {e}")))?;

    if text.trim().is_empty() {
        return Err(Fail::Usage(format!("nothing on stdin: {FORMS}")));
    }

    serde_json::from_str(&text).map_err(|e| {
        Fail::Usage(format!(
            "the record on stdin is not a promise: {e}\n\
             expected an object with `promise`, `by`, `to`, `until`, and optionally `id`, `on`, \
             `task` — no other keys"
        ))
    })
}

// ---------------------------------------------------------------- resolve

fn resolve(args: ResolveArgs) -> Result<(), Fail> {
    // The shape of the command is checked before the ledger is consulted: a malformed
    // resolve is malformed whether or not the id exists, and nothing should be
    // appended while we still have a question about what was asked for.
    let outcome = resolution_from(&args)?;

    let mut events = read_ledger()?;
    let now = now_to_the_second();

    let (default_by, existing) = {
        let folded = fold(&events, now);
        let view = folded
            .get(&args.id)
            .ok_or_else(|| unknown_id(&args.id, "resolve"))?;
        let default_by = match outcome {
            // §4's "Issued by" column: the debtor satisfies or cancels, the creditor
            // releases. An explicit --by that disagrees is recorded, never rejected —
            // pinki "records who claimed what".
            Outcome::Satisfied(_) | Outcome::Cancelled(_) => view.record.by.clone(),
            Outcome::Released(_) => view.record.to.clone(),
        };
        let existing = view.resolution.map(|event| {
            let kind = event
                .resolution()
                .map(Resolution::as_str)
                .unwrap_or("resolved");
            (kind.to_string(), event.ts.clone())
        });
        (default_by, existing)
    };

    if let Some((kind, ts)) = existing {
        return Err(Fail::Op(format!(
            "`{}` is already resolved: {kind} at {ts}. The first resolve wins (§4), so nothing \
             was appended — `pinki show {}` for the record",
            args.id, args.id
        )));
    }

    let by = match args.by {
        Some(by) => non_blank(&by, "`by`")?,
        None => default_by,
    };

    let body = match outcome {
        Outcome::Satisfied(evidence) => EventBody::satisfied(&args.id, by, evidence),
        Outcome::Cancelled(reason) => EventBody::cancelled(&args.id, by, reason),
        Outcome::Released(reason) => EventBody::released(&args.id, by, reason),
    };

    let event = Event::new(now.to_string(), body);
    append(&event)?;
    events.push(event);

    report(&events, now, &args.id);
    Ok(())
}

/// The three outcomes of §4's resolve table, after validation.
#[derive(Debug)]
enum Outcome {
    Satisfied(Vec<String>),
    Cancelled(String),
    Released(Option<String>),
}

/// §4's requirements, checked in pinki's own words rather than clap's.
///
/// Evidence is the one rule pinki is strict about: "'done' that points at nothing is
/// the exact failure this whole category of tool exists to catch." Zero references,
/// or one that is blank, is malformed input — exit 2, with nothing appended.
fn resolution_from(args: &ResolveArgs) -> Result<Outcome, Fail> {
    if args.satisfied {
        if args.reason.is_some() {
            return Err(Fail::Usage(
                "--reason is not accepted with --satisfied: satisfaction is claimed with \
                 --evidence, a reference pointing outward at something another party could go \
                 and look at. A prose summary is welcome in a note; it is not evidence (§4)"
                    .into(),
            ));
        }
        if args.evidence.is_empty() {
            return Err(Fail::Usage(
                "--satisfied requires at least one --evidence REF, and pinki will not append a \
                 resolve without one: a resolve event with an empty evidence list is malformed \
                 (§4). A reference is a URL, a file path, a commit SHA, or an A2A \
                 Artifact.artifact_id — pinki never fetches it, it only insists that it points \
                 somewhere"
                    .into(),
            ));
        }
        let mut evidence = Vec::with_capacity(args.evidence.len());
        for reference in &args.evidence {
            let reference = reference.trim();
            if reference.is_empty() {
                return Err(Fail::Usage(
                    "an --evidence value is empty: evidence must point at something another \
                     party could go and look at, so a blank reference is malformed (§4). \
                     Nothing was appended"
                        .into(),
                ));
            }
            evidence.push(reference.to_string());
        }
        return Ok(Outcome::Satisfied(evidence));
    }

    if args.cancelled {
        if !args.evidence.is_empty() {
            return Err(Fail::Usage(
                "--evidence is not accepted with --cancelled: a cancelled promise was not done, \
                 so there is nothing to point at. Say why with --reason (§4)"
                    .into(),
            ));
        }
        let reason = args.reason.as_deref().ok_or_else(|| {
            Fail::Usage(
                "--cancelled requires --reason TEXT: abandoning an obligation without saying why \
                 is the thing pinki exists to make visible (§4)"
                    .into(),
            )
        })?;
        return Ok(Outcome::Cancelled(non_blank(reason, "--reason")?));
    }

    // --released. Clap's ArgGroup makes this the only remaining case.
    if !args.evidence.is_empty() {
        return Err(Fail::Usage(
            "--evidence is not accepted with --released: releasing is the creditor letting the \
             debtor off, not a claim that the work was done (§4)"
                .into(),
        ));
    }
    let reason = match &args.reason {
        Some(reason) => Some(non_blank(reason, "--reason")?),
        None => None,
    };
    Ok(Outcome::Released(reason))
}

// ---------------------------------------------------------------- assess

fn assess(args: AssessArgs) -> Result<(), Fail> {
    let observer = non_blank(&args.observer, "--observer")?;
    let note = match &args.note {
        Some(note) => Some(non_blank(note, "--note")?),
        None => None,
    };

    let mut events = read_ledger()?;
    let now = now_to_the_second();

    // An already-resolved promise may still be assessed. §4: an assessment is "never
    // terminal", and §3 wants contradicting judgments visible rather than suppressed
    // by whoever wrote first.
    if fold(&events, now).get(&args.id).is_none() {
        return Err(unknown_id(&args.id, "assess"));
    }

    let event = Event::new(
        now.to_string(),
        EventBody::violated(&args.id, observer, note),
    );
    append(&event)?;
    events.push(event);

    report(&events, now, &args.id);
    Ok(())
}

// ---------------------------------------------------------------- ls

/// One `ls --json` row: the §1 record's fields, plus the computed state.
///
/// Flattened rather than nested so a consumer reads `.[0].until` and `.[0].state`
/// side by side, and serialized straight to a string so the record keeps §1's field
/// order with `state` appended.
#[derive(Serialize)]
struct Row<'a> {
    #[serde(flatten)]
    record: &'a Promise,
    state: &'static str,
}

fn ls(args: LsArgs) -> Result<(), Fail> {
    let events = read_ledger()?;
    let now = now_to_the_second();
    let folded = fold(&events, now);

    let scope = args.scope();
    let rows: Vec<&PromiseView> = folded
        .iter()
        .filter(|view| match scope {
            // `--open` is §5's "everything not in a terminal state".
            Scope::Open => view.state.is_open(),
            Scope::Overdue => view.state == State::Overdue,
            Scope::All => true,
        })
        // Orthogonal to the state filter, and exact: `by` and `to` are card
        // identities, and pinki resolves nothing (§6).
        .filter(|view| args.by.as_deref().is_none_or(|by| view.record.by == by))
        .filter(|view| args.to.as_deref().is_none_or(|to| view.record.to == to))
        .collect();

    if args.json {
        let json: Vec<Row> = rows
            .iter()
            .map(|view| Row {
                record: view.record,
                state: view.state.as_str(),
            })
            .collect();
        // An empty ledger prints `[]` and exits 0: nobody has promised anything, which
        // is a legitimate state and not an error.
        println!("{}", encode(&json)?);
        return Ok(());
    }

    if rows.is_empty() {
        // Silence would be ambiguous here: an empty ledger and a filter that hid
        // everything look identical on stdout. The note goes to stderr so the table
        // stays exactly as machine-readable as it was, and the exit code stays 0 —
        // nobody having promised anything is a legitimate state, not an error.
        if folded.is_empty() {
            eprintln!(
                "pinki: the ledger at {} holds no promises",
                ledger::path().display()
            );
        } else {
            eprintln!(
                "pinki: no promises match; the ledger holds {} — `pinki ls --all` shows every state",
                folded.len()
            );
        }
        return Ok(());
    }

    let id_width = width(rows.iter().map(|view| view.id()));
    let state_width = width(rows.iter().map(|view| view.state.as_str()));
    let until_width = width(rows.iter().map(|view| view.record.until.as_str()));
    let parties: Vec<String> = rows
        .iter()
        .map(|view| format!("{} -> {}", view.record.by, view.record.to))
        .collect();
    let party_width = width(parties.iter().map(String::as_str));

    for (view, party) in rows.iter().zip(parties.iter()) {
        println!(
            "{:<id_width$}  {:<state_width$}  {:<until_width$}  {:<party_width$}  {}",
            view.id(),
            view.state.as_str(),
            view.record.until,
            party,
            ellipsize(&view.record.promise, 56),
        );
    }
    Ok(())
}

// ---------------------------------------------------------------- show

/// `show --json`: the record, the computed state, how it ended, and every judgment.
#[derive(Serialize)]
struct Detail<'a> {
    #[serde(flatten)]
    record: &'a Promise,
    state: &'static str,
    /// `null` until somebody resolves it — the field is always present so a consumer
    /// can test it rather than probe for it.
    resolution: Option<ResolutionJson<'a>>,
    /// Every assessment naming this promise, in log order. Two contradicting ones both
    /// appear: §3, "that is not a bug in pinki; it is the actual state of the world."
    assessments: Vec<AssessmentJson<'a>>,
}

#[derive(Serialize)]
struct ResolutionJson<'a> {
    #[serde(rename = "as")]
    kind: &'a str,
    ts: &'a str,
    by: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    evidence: Option<&'a [String]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'a str>,
}

#[derive(Serialize)]
struct AssessmentJson<'a> {
    ts: &'a str,
    state: &'a str,
    observer: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<&'a str>,
}

fn show(args: ShowArgs) -> Result<(), Fail> {
    let events = read_ledger()?;
    let now = now_to_the_second();
    let folded = fold(&events, now);
    let view = folded
        .get(&args.id)
        .ok_or_else(|| unknown_id(&args.id, "show"))?;

    let resolution = view.resolution.and_then(|event| {
        // `kind` and `by` are the same question for all three outcomes — §4 puts them
        // on every resolve — so they come off the accessors; only the payload differs.
        view.resolution_kind().map(|resolution| match resolution {
            Resolution::Satisfied { evidence, .. } => ResolutionJson {
                kind: resolution.as_str(),
                ts: &event.ts,
                by: resolution.by(),
                evidence: Some(evidence.as_slice()),
                reason: None,
            },
            Resolution::Cancelled { reason, .. } => ResolutionJson {
                kind: resolution.as_str(),
                ts: &event.ts,
                by: resolution.by(),
                evidence: None,
                reason: Some(reason.as_str()),
            },
            Resolution::Released { reason, .. } => ResolutionJson {
                kind: resolution.as_str(),
                ts: &event.ts,
                by: resolution.by(),
                evidence: None,
                reason: reason.as_deref(),
            },
        })
    });

    let assessments: Vec<AssessmentJson> = view
        .assessments
        .iter()
        .filter_map(|event| match &event.body {
            EventBody::Assess {
                state,
                observer,
                note,
                ..
            } => Some(AssessmentJson {
                ts: &event.ts,
                state,
                observer,
                note: note.as_deref(),
            }),
            _ => None,
        })
        .collect();

    if args.json {
        let detail = Detail {
            record: view.record,
            state: view.state.as_str(),
            resolution,
            assessments,
        };
        println!("{}", encode(&detail)?);
        return Ok(());
    }

    println!("{}  {}", view.id(), view.state.as_str());
    println!("  promise      {}", view.record.promise);
    println!("  by           {}", view.record.by);
    println!("  to           {}", view.record.to);
    if let Some(on) = &view.record.on {
        println!("  on           {on}");
    }
    println!("  until        {}", view.record.until);
    if let Some(task) = &view.record.task {
        println!("  task         {task}");
    }

    match &resolution {
        Some(resolution) => {
            println!(
                "  resolution   {} at {} by {}",
                resolution.kind, resolution.ts, resolution.by
            );
            if let Some(evidence) = resolution.evidence {
                for reference in evidence {
                    println!("    evidence   {reference}");
                }
            }
            if let Some(reason) = resolution.reason {
                println!("    reason     {reason}");
            }
        }
        None => println!("  resolution   (unresolved)"),
    }

    if assessments.is_empty() {
        println!("  assessments  (none)");
    } else {
        println!("  assessments");
        for assessment in &assessments {
            let note = assessment
                .note
                .map(|n| format!("  {n}"))
                .unwrap_or_default();
            println!(
                "    {}  {}  {}{note}",
                assessment.ts, assessment.state, assessment.observer
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- a2a

fn a2a_card() -> Result<(), Fail> {
    // Nothing on stderr, nothing but JSON on stdout — these two verbs are meant to be
    // piped into an A2A client, and prose in the pipe is a bug.
    println!(
        "{}",
        a2a::card().map_err(|e| Fail::Op(format!("could not encode the AgentCard block: {e}")))?
    );
    Ok(())
}

fn a2a_task(id: &str) -> Result<(), Fail> {
    let events = read_ledger()?;
    let folded = fold(&events, now_to_the_second());
    let view = folded.get(id).ok_or_else(|| unknown_id(id, "a2a task"))?;

    // No computed state rides the block. A2A-EXTENSION.md §2: `overdue` is never an
    // A2A state, and the metadata key carries the record verbatim.
    println!(
        "{}",
        a2a::task_metadata(view.record)
            .map_err(|e| Fail::Op(format!("could not encode the Task metadata block: {e}")))?
    );
    Ok(())
}

// ---------------------------------------------------------------- shared

/// Read the ledger, or fail operationally. A missing file is an empty ledger.
fn read_ledger() -> Result<Vec<Event>, Fail> {
    ledger::read().map_err(|e| Fail::Op(e.to_string()))
}

/// Append one event, or fail operationally.
fn append(event: &Event) -> Result<(), Fail> {
    ledger::append(event).map_err(|e| Fail::Op(e.to_string()))
}

/// `now`, truncated to the second.
///
/// §4's example timestamps are `2026-08-28T20:14:03Z`. Nanoseconds would be
/// precision pinki has no use for — the fold never compares event timestamps — and a
/// ledger is meant to be read by a human with `less`.
fn now_to_the_second() -> Timestamp {
    let now = Timestamp::now();
    Timestamp::from_second(now.as_second()).unwrap_or(now)
}

/// What every mutating verb prints: the id, a tab, and the state the ledger now folds
/// to. One line, two fields, `cut -f1`-able.
fn report(events: &[Event], now: Timestamp, id: &str) {
    let folded = fold(events, now);
    let state = folded.state(id).map(State::as_str).unwrap_or("unknown");
    println!("{id}\t{state}");
}

/// An id nothing in the ledger declares.
///
/// The hint is narrow on purpose. Since §1 admits a caller's own id, "this does not
/// look like a pinki id" is no longer evidence of anything — a fleet's own id space is
/// a perfectly good id here. The one shape that still earns a remark is an id wearing
/// the reserved `pnk_` prefix without being a minted one: that one could never have
/// been declared, by any route, so it is a typo rather than a lookup miss.
fn unknown_id(id: &str, verb: &str) -> Fail {
    let hint = if id.starts_with(id::PREFIX) && !id::is_well_formed(id) {
        format!(
            " (and `{id}` could never have been declared: `{}` is reserved for minted ids, \
             which are the prefix plus exactly six lowercase hex digits)",
            id::PREFIX
        )
    } else {
        String::new()
    };
    Fail::Op(format!(
        "cannot {verb} `{id}`: nothing in {} declares it{hint}",
        ledger::path().display()
    ))
}

/// A flag the flag-mode `promise` cannot do without.
fn required_flag(value: Option<String>, flag: &str) -> Result<String, Fail> {
    value.ok_or_else(|| {
        Fail::Usage(format!(
            "{flag} is required: pinki promise TEXT --by WHO --to WHO --until ISO"
        ))
    })
}

/// Trim, and refuse the empty string. Whitespace is not a value.
fn non_blank(value: &str, what: &str) -> Result<String, Fail> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(Fail::Usage(format!("{what} is empty")));
    }
    Ok(trimmed.to_string())
}

/// Parse `--until` as an instant, and store it normalized.
///
/// §1 requires a deadline and §3 computes `overdue` by comparing it to `now`, so a
/// deadline pinki cannot parse is not a deadline. A naive datetime is the interesting
/// rejection: `2026-09-01T17:00:00` names a wall clock somewhere unstated, and two
/// readers in different zones would compute different answers from it — which is
/// exactly what §3's "any two implementations produce the same answer" forbids.
fn parse_until(until: &str) -> Result<String, Fail> {
    let trimmed = until.trim();
    Timestamp::from_str(trimmed)
        .map(|t| t.to_string())
        .map_err(|e| {
            Fail::Usage(format!(
                "--until `{trimmed}` is not an ISO-8601 instant: {e}\n\
             a deadline must name an unambiguous moment, so it needs an offset — try \
             `2026-09-01T17:00Z`, `2026-09-01T17:00:00Z`, or `2026-09-01T10:00:00-07:00`"
            ))
        })
}

/// Pretty-printed JSON, straight to a string so struct field order survives.
fn encode<T: Serialize>(value: &T) -> Result<String, Fail> {
    serde_json::to_string_pretty(value)
        .map_err(|e| Fail::Op(format!("could not encode the output as JSON: {e}")))
}

/// The widest of a column's cells, in characters.
fn width<'a>(cells: impl Iterator<Item = &'a str>) -> usize {
    cells.map(|cell| cell.chars().count()).max().unwrap_or(0)
}

/// Truncate free text for the table, with an ASCII ellipsis.
///
/// The promise text is whatever a user wrote, so this counts characters rather than
/// bytes; pinki's own output stays ASCII, which is what keeps the columns aligned in
/// a terminal that does not agree with you about glyph widths.
fn ellipsize(text: &str, max: usize) -> String {
    let flattened: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if flattened.chars().count() <= max {
        return flattened;
    }
    let head: String = flattened.chars().take(max.saturating_sub(3)).collect();
    format!("{head}...")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_split_malformed_from_operational() {
        assert_eq!(Fail::Usage("x".into()).code(), 2);
        assert_eq!(Fail::Op("x".into()).code(), 1);
    }

    #[test]
    fn until_is_normalized_to_seconds_and_utc() {
        assert_eq!(
            parse_until("2026-09-01T17:00Z").unwrap(),
            "2026-09-01T17:00:00Z"
        );
        assert_eq!(
            parse_until("2026-09-01T10:00:00-07:00").unwrap(),
            "2026-09-01T17:00:00Z"
        );
    }

    #[test]
    fn naive_until_is_a_usage_error() {
        let err = parse_until("2026-09-01T17:00:00").unwrap_err();
        assert_eq!(err.code(), 2);
        let message = err.to_string();
        assert!(message.contains("offset"), "{message}");
        assert!(message.contains("2026-09-01T17:00Z"), "{message}");
    }

    #[test]
    fn blank_values_are_refused() {
        assert_eq!(non_blank("   ", "--reason").unwrap_err().code(), 2);
        assert_eq!(non_blank(" kept ", "--reason").unwrap(), "kept");
    }

    #[test]
    fn satisfied_without_evidence_is_malformed() {
        let args = ResolveArgs {
            id: "pnk_4f3a91".into(),
            satisfied: true,
            cancelled: false,
            released: false,
            evidence: Vec::new(),
            reason: None,
            by: None,
        };
        let err = resolution_from(&args).unwrap_err();
        assert_eq!(err.code(), 2);
        assert!(err.to_string().contains("--evidence"), "{err}");
    }

    #[test]
    fn blank_evidence_is_malformed() {
        let args = ResolveArgs {
            id: "pnk_4f3a91".into(),
            satisfied: true,
            cancelled: false,
            released: false,
            evidence: vec!["   ".into()],
            reason: None,
            by: None,
        };
        assert_eq!(resolution_from(&args).unwrap_err().code(), 2);
    }

    #[test]
    fn cancelled_needs_a_reason_and_refuses_evidence() {
        let bare = ResolveArgs {
            id: "pnk_4f3a91".into(),
            satisfied: false,
            cancelled: true,
            released: false,
            evidence: Vec::new(),
            reason: None,
            by: None,
        };
        assert_eq!(resolution_from(&bare).unwrap_err().code(), 2);

        let with_evidence = ResolveArgs {
            evidence: vec!["https://example.org/x".into()],
            reason: Some("upstream schema was withdrawn".into()),
            ..bare
        };
        assert_eq!(resolution_from(&with_evidence).unwrap_err().code(), 2);
    }

    #[test]
    fn released_takes_an_optional_reason() {
        let args = ResolveArgs {
            id: "pnk_4f3a91".into(),
            satisfied: false,
            cancelled: false,
            released: true,
            evidence: Vec::new(),
            reason: None,
            by: None,
        };
        assert!(matches!(
            resolution_from(&args).unwrap(),
            Outcome::Released(None)
        ));
    }

    #[test]
    fn ellipsize_counts_characters_and_stays_ascii() {
        assert_eq!(ellipsize("short", 10), "short");
        assert_eq!(ellipsize("abcdefghijkl", 8), "abcde...");
        // Multi-byte text must not be sliced mid-character.
        assert_eq!(ellipsize("ααααααααα", 5), "αα...");
        assert_eq!(ellipsize("a\tb", 10), "a b");
    }
}
