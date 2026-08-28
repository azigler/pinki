//! pinki — a tiny promise broker.
//!
//! See `docs/DESIGN.md`. This binary is layer 3: "a CLI over an append-only log."
//! Layers 1 and 2 — the vocabulary and the A2A binding — are documents, and are
//! adoptable without ever running this.
//!
//! **Wave 1 lands the model, the log, and the fold. The verbs are not here yet.**

// Wave 1 is the data model, the ledger, and the fold; the six verbs of DESIGN.md §5
// that consume them are the next commit. Every item below is exercised by this
// crate's unit tests, but `dead_code` does not count test-only use, so without this
// the wave-1 build is a wall of warnings about a surface that is complete and
// deliberately not yet wired to `main`. Delete this line when the verbs land — if it
// then produces warnings, they are real.
#![allow(dead_code)]

mod event;
mod id;
mod ledger;
mod record;
mod state;

fn main() {
    // The verbs (`promise`, `resolve`, `assess`, `ls`, `show`, `a2a`) arrive with the
    // clap wiring in the next wave. Until then this exits quietly rather than
    // pretending to be a CLI.
}
