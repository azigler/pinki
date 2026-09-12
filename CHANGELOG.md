# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Minor versions carry the work as it lands. **1.0.0 is reserved for done** — for the
point where the vocabulary and the A2A binding are stable enough that other people's
tools can depend on them, not merely read them.

## [Unreleased]

### Added

- **The `amend` event, and the `amend` verb** — a deadline that moves, without a
  silent move and without a phantom record per move. `pinki amend <ID> --until ISO
  [--reason TEXT] [--by WHO]` appends `{"type":"amend","promise":…,"by":…,"until":…}`
  beside the declaration; the fold takes the latest amend's `until` as the current
  horizon and keeps every earlier one, which `show` lists in order (text and
  `--json`'s `horizons`). `ls`, `show` and the A2A `metadata` block all carry the
  horizon as it stands, so an outside observer computes the same `overdue` as the
  fleet that owns the promise. Closes
  [#9](https://github.com/azigler/pinki/issues/9), which arrived with the
  measurement that motivates it: an escalation ladder re-declaring the same id had an
  observer scoring a live promise overdue 32 minutes before its owner did.
- **DESIGN.md § 8's second open question is settled** — the `amend` event, not
  re-declaration and not supersede-with-lineage, with the reasoning and the rejected
  alternatives recorded in place (§ 4 and § 8.2).

### Changed

- **The deadline in `ls`, `show` and `a2a task` is the current one**, not the
  declared one, once a promise has been amended. The declaration itself is never
  rewritten — `first declaration wins` is unchanged, and a reader that does not know
  the `amend` type folds exactly the v0.1.0 answer rather than a broken one.

### Refused, on purpose

- **Only the debtor may amend** (§ 8.2). An amend naming anyone else is refused with
  the rule cited and nothing appended; an observer who thinks a deadline should move
  is making an assessment, which is a different speech act with its own verb. Like
  every other rule here this is enforced where input arrives, never in the fold —
  pinki authenticates nobody, and a joined ledger may carry an amend written by
  something looser.
- **A resolved promise cannot be amended.** The first resolve wins and ends it; there
  is no horizon left to move.

### Testing

- **155 tests** — 94 unit, 61 driving the real binary — at **98.32% line coverage**,
  including the three-step escalation ladder from #9 end to end and the proof that a
  fold which skips `amend` events still answers exactly what v0.1.0 answered. The ten
  deliberately uncovered lines at the end of `tests/cli.rs` are unchanged; their line
  numbers moved.

## [0.1.0] - 2026-08-28

The nucleus: enough of pinki to actually use, and enough tests to believe it.

### Added

- **The vocabulary.** One promise record — `id`, `promise`, `by`, `to`, `on`,
  `until`, `task` — where a debtor owes a creditor a thing by a deadline, and `on`
  points at the promise this one waits on. Deadlines are required: a promise without
  one is a wish.
- **The fold.** State is *computed* from the append-only log, never stored. Seven
  computed states (`conditional`, `detached`, `overdue`, `satisfied`, `cancelled`,
  `released`, `expired`) and one that is only ever published, never computed:
  `violated`. `overdue` is arithmetic — `now > until`, unresolved — and every
  implementation gets the same answer. Assessment is a speech act, attributed and
  timestamped, and pinki publishes contradicting judgments side by side rather than
  letting whoever wrote first win.
- **Six verbs** over a JSONL ledger at `$PINKI_LEDGER` (default
  `./.pinki/ledger.jsonl`): `promise`, `resolve`, `assess`, `ls`, `show`, and `a2a`.
  Satisfying a promise requires evidence that points outward at something a second
  party could go and look at; abandoning one requires a reason. Both rules are
  enforced at the edge where input arrives, never in the fold — which has to be able
  to read a ledger somebody else wrote.
- **Two A2A emitters**, `a2a card` and `a2a task`, writing the AgentCard extension
  declaration and the URI-prefixed `metadata` block to stdout. They call nothing:
  getting a block to a peer is the job of the A2A client you already run.
- **Exit codes as a contract.** `0` success, `1` operational failure, `2` malformed
  input or usage — matching what clap's own parse errors return. Errors go to
  stderr, so `--json` and both `a2a` verbs stay pipeable under every outcome.

### Security

- **The no-network invariant is structural, not a promise.** DESIGN.md §7 says pinki
  makes no network calls, and the dependency list is what enforces it: nothing in the
  tree — `clap`, `serde`, `serde_json`, `jiff` with default features off — can open a
  socket. No HTTP client, no async runtime, no TLS stack, no DNS resolver, and no
  transitive path to one. A tool that cannot carry a message cannot distort one.
  Adding a dependency that *can* reach the network breaks this even if no call is
  ever written; audit `cargo tree`, not just the source.

### Testing

- **134 tests** — 80 unit, 54 driving the real binary against a real ledger file —
  at **97.99% line coverage** (97.60% region). CI gates at 97%.
- The ten lines that remain uncovered are enumerated with reasons at the end of
  `tests/cli.rs`. All ten are unreachable defensive arms, the `is_terminal()` refusal
  that needs a tty no test harness has, or the failure arm of an assertion that does
  not run while the suite is green. An unexplained gap and a deliberate one look
  identical in a coverage report; these are on the record as deliberate.

[Unreleased]: https://github.com/azigler/pinki/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/azigler/pinki/releases/tag/v0.1.0
