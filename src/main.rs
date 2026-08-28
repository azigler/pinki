//! pinki — a tiny promise broker.
//!
//! See `docs/DESIGN.md`. This binary is layer 3: "a CLI over an append-only log."
//! Layers 1 and 2 — the vocabulary and the A2A binding — are documents, and are
//! adoptable without ever running this.
//!
//! # Exit codes
//!
//! | Code | Meaning |
//! |---|---|
//! | `0` | Success. |
//! | `1` | Operational failure: an id nothing declares, a promise already resolved, a ledger that will not read or write. |
//! | `2` | Malformed input or a usage error — including clap's own parse errors, which already exit 2. |
//!
//! Errors are written to **stderr**, never stdout, so `--json` and the two `a2a`
//! verbs stay pipeable into a JSON consumer under every outcome.

mod a2a;
mod cli;
mod event;
mod id;
mod ledger;
mod record;
mod state;
mod verbs;

use clap::Parser;

fn main() {
    let cli = cli::Cli::parse();
    if let Err(failure) = verbs::run(cli.command) {
        eprintln!("pinki: {failure}");
        std::process::exit(failure.code());
    }
}
