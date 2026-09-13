# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Minor versions carry the work as it lands. **1.0.0 is reserved for done** — for the
point where the vocabulary and the A2A binding are stable enough that other people's
tools can depend on them, not merely read them.

## [Unreleased]

### Added

- **`meta`: one reserved, opaque object on the record — the provenance extension
  point** ([#6](https://github.com/azigler/pinki/issues/6)). The record was a closed
  set of seven keys, and `pinki promise` reads stdin with `deny_unknown_fields`, so an
  existing obligation row could not be piped in without first projecting away
  everything it carried about *provenance* — which component declared it, on whose
  behalf, under which policy. `meta` is where that goes: an optional JSON object,
  stored verbatim, handed back by `show`, `ls --json` and `a2a task`, and **never
  interpreted**. Accepted on stdin (`{"promise":…,"meta":{…}}`) and as
  `--meta '<json object>'`.

  The arithmetic fields stay closed, and that is the point of the shape: state is a
  fold over `on`, `until` and the resolve events, so a field the fold consults is a
  field two implementations can disagree about. The fold cannot see `meta` at all —
  tested by mutating `meta` on every event in a ledger and asserting that every
  computed state is identical.

- **`meta` on `resolve` and `assess` too**, under the same opaque rule, via `--meta`.
  An adopting system's resolve rows carry their own provenance — which component
  resolved this, on whose behalf, citing what — and that belongs to the act, not to
  the promise, so it rides the event that performed it. `show --json` returns it under
  `resolution.meta` and each `assessments[].meta`.

- Three shape rules at the edge, each exit 2 with nothing appended: `meta` must be an
  object (a consumer has to be able to read the one key it knows and ignore the rest),
  may not be empty (`{}` is provenance offered and left blank — omit it and no key is
  written at all), and is capped at 8 KiB, measured on its canonicalized, compact
  serialized form after parsing — not on the literal `--meta` input bytes, so a caller
  byte-budgeting input should expect whitespace stripped and keys sorted before the
  measurement (a ledger line is a line). Nothing *inside* it is checked: nesting,
  arrays, nulls and unknown keys are all fine, because opaque means opaque. On the
  stdin form of `promise`, a `--meta` flag is refused rather than silently ignored.
  Duplicate keys inside `meta` resolve last-wins on parse (`serde_json`'s documented
  behavior), silently.

### Compatibility with the transcript

**A ledger containing `meta` is readable by v0.1.0, and v0.1.0 will silently drop the
`meta` from everything it prints.** Measured against the installed v0.1.0 binary on a
meta-bearing ledger written by this version, rather than assumed:

- `ls --all`, `ls --all --json`, `show`, `show --json` and `a2a task` all exit **0**
  and report the correct states — and every one of them omits `meta`. The record's
  `meta`, the resolve's and the assess's are simply not in the output. A v0.1.0 reader
  cannot tell that any provenance was there.
- Nothing is lost from the **file**. v0.1.0 appends to a meta-bearing ledger normally
  (`resolve … --released` → rc 0), and the `meta` on the lines it did not write is
  untouched afterwards; this version still reads it back in full.
- v0.1.0 cannot *write* one, which is issue #6 itself: a stdin record carrying `meta`
  is refused — `unknown field 'meta', expected one of 'id', 'promise', 'by', 'to',
  'on', 'until', 'task'`, rc 2 — and it has no `--meta` flag (`error: unexpected
  argument '--meta' found`, rc 2).

So the honest statement is narrower than "old readers are fine" and narrower than
"old readers break": **a ledger with `meta` needs this version or later to be read
without silent loss.** Provenance is the whole reason the field exists, and a reader
that drops it exits 0 while telling you less than the file says.

One more limit, because "verbatim" should mean what it says: `meta` is stored as a
JSON object and re-emitted with its keys **sorted**, so key order is canonicalised on
the first write. Values, types and nesting round-trip exactly, and every read surface
agrees byte for byte with the ledger line — but if you are diffing bytes against your
own input, diff against the first write instead.

### Testing

- **159 tests** — 94 unit, 65 driving the real binary — at **98.04%** line coverage
  against CI's 97% floor. The uncovered set is eleven lines, enumerated with reasons at
  the end of `tests/cli.rs`.

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
