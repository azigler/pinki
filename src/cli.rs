//! The argument surface — DESIGN.md §5's six verbs, and nothing else.
//!
//! This module only *describes* the command line. Everything it accepts is then
//! validated by [`crate::verbs`], because several of §5's rules are ours to state in
//! our own words: an `--evidence` list that is empty or blank (§4: "A resolve event
//! with an empty `evidence` list is malformed, and pinki says so"), an `--until` that
//! is not an instant, and the stdin form of `promise`. Rules that clap can express
//! without editorializing — mutual exclusion, requiredness of a positional — live
//! here.

use clap::{ArgGroup, Args, Parser, Subcommand};

/// pinki — a tiny promise broker: an append-only log of obligations between named
/// agents.
#[derive(Debug, Parser)]
#[command(
    name = "pinki",
    version,
    about = "A tiny promise broker: an append-only log of obligations between named agents.",
    long_about = "pinki records who owes what to whom, by when — and computes state from an \
append-only log rather than storing it. It makes no network calls: the ledger is a file at \
$PINKI_LEDGER (default ./.pinki/ledger.jsonl), and every verb reads and writes JSON."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Speak a promise into existence. With no TEXT, reads one record as JSON on stdin.
    Promise(PromiseArgs),
    /// End a promise: satisfied (with evidence), cancelled (with a reason), or released.
    Resolve(ResolveArgs),
    /// Publish an assessment of a promise. Never terminal — the promise stays as open as it was.
    Assess(AssessArgs),
    /// List promises, filtered by computed state.
    Ls(LsArgs),
    /// Show one promise: its record, its state, its resolution, and every assessment.
    Show(ShowArgs),
    /// Emit an A2A block to stdout. Calls nothing.
    A2a {
        #[command(subcommand)]
        command: A2aCommand,
    },
}

#[derive(Debug, Args)]
pub struct PromiseArgs {
    /// The thing owed, as free text. Omit it to read a full record as JSON on stdin.
    pub text: Option<String>,

    /// The debtor — who owes. An A2A AgentCard identity.
    #[arg(long, value_name = "WHO")]
    pub by: Option<String>,

    /// The creditor — who is owed.
    #[arg(long, value_name = "WHO")]
    pub to: Option<String>,

    /// Deadline, as an ISO-8601 instant with an offset (e.g. 2026-09-01T17:00Z).
    #[arg(long, value_name = "ISO")]
    pub until: Option<String>,

    /// Antecedent: the id of the promise this one waits on.
    #[arg(long, value_name = "ID")]
    pub on: Option<String>,

    /// The A2A Task.id this promise is about, when there is one.
    #[arg(long, value_name = "ID")]
    pub task: Option<String>,

    /// Use this id instead of minting one. Any non-blank string with no whitespace and
    /// no control characters, up to 128 characters — bring your own id space. The
    /// `pnk_` prefix is reserved for ids pinki mints.
    #[arg(long, value_name = "ID")]
    pub id: Option<String>,
}

#[derive(Debug, Args)]
#[command(group(
    ArgGroup::new("outcome")
        .required(true)
        .multiple(false)
        .args(["satisfied", "cancelled", "released"])
))]
pub struct ResolveArgs {
    /// The promise being resolved.
    pub id: String,

    /// Resolved by the debtor, with evidence. Requires --evidence.
    #[arg(long)]
    pub satisfied: bool,

    /// Resolved by the debtor, with a reason. Requires --reason.
    #[arg(long)]
    pub cancelled: bool,

    /// The creditor lets the debtor off. A reason is encouraged, not required.
    #[arg(long)]
    pub released: bool,

    /// A reference pointing outward at something another party could go and look at.
    /// Repeatable. Required with --satisfied.
    #[arg(long, value_name = "REF")]
    pub evidence: Vec<String>,

    /// Why. Required with --cancelled, optional with --released.
    #[arg(long, value_name = "TEXT")]
    pub reason: Option<String>,

    /// Who is claiming this. Defaults to the debtor for --satisfied/--cancelled and
    /// to the creditor for --released. pinki records the claim; it never checks it.
    #[arg(long, value_name = "WHO")]
    pub by: Option<String>,
}

#[derive(Debug, Args)]
pub struct AssessArgs {
    /// The promise being assessed.
    pub id: String,

    /// Assert that this promise was broken. A judgment, attributed to --observer.
    #[arg(long, required = true)]
    pub violated: bool,

    /// Who is making the assessment.
    #[arg(long, required = true, value_name = "WHO")]
    pub observer: String,

    /// Free text supporting the assessment.
    #[arg(long, value_name = "TEXT")]
    pub note: Option<String>,
}

#[derive(Debug, Args)]
#[command(group(
    ArgGroup::new("scope")
        .multiple(false)
        .args(["open", "overdue", "all"])
))]
pub struct LsArgs {
    /// Everything not in a terminal state: conditional + detached + overdue. The default.
    #[arg(long)]
    pub open: bool,

    /// The overdue subset alone.
    #[arg(long)]
    pub overdue: bool,

    /// Every promise, terminal states included.
    #[arg(long)]
    pub all: bool,

    /// Only promises owed BY this identity.
    #[arg(long, value_name = "WHO")]
    pub by: Option<String>,

    /// Only promises owed TO this identity.
    #[arg(long, value_name = "WHO")]
    pub to: Option<String>,

    /// Emit a JSON array instead of a table.
    #[arg(long)]
    pub json: bool,
}

impl LsArgs {
    /// Which computed states to show. `--open` when nothing is asked for: §5's `ls`
    /// is a working list, and a terminal promise is not work.
    pub fn scope(&self) -> Scope {
        if self.all {
            Scope::All
        } else if self.overdue {
            Scope::Overdue
        } else {
            Scope::Open
        }
    }
}

/// The state filter `ls` applies — §5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// conditional + detached + overdue.
    Open,
    /// overdue alone.
    Overdue,
    /// Everything, terminal states included.
    All,
}

#[derive(Debug, Args)]
pub struct ShowArgs {
    /// The promise to show.
    pub id: String,

    /// Emit one JSON object instead of a human-readable block.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Subcommand)]
pub enum A2aCommand {
    /// Emit the AgentCard extension block.
    Card,
    /// Emit the Task metadata block carrying one promise record.
    Task {
        /// The promise to carry.
        id: String,
    },
}
