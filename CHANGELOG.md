# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Minor versions carry the work as it lands. **1.0.0 is reserved for done** — for the
point where the vocabulary and the A2A binding are stable enough that other people's
tools can depend on them, not merely read them.

## [Unreleased]

### Changed

- **`--id` accepts an id you already have.** It used to refuse anything that was not
  `pnk_` plus six lowercase hex digits, which made the one affordance that looks like
  "carry your ledger over" not be one. A supplied id is now opaque: any non-blank
  string with no whitespace and no control characters, up to 128 characters. Minting
  is untouched — leave `--id` off and you get a `pnk_` id exactly as before — and the
  stdin record's `"id"` takes the same forms, by the same code. Closes
  [#5](https://github.com/azigler/pinki/issues/5).

  Two consequences, both from the issue. **Adoption**: an arriving ledger keeps the
  ids other systems already reference, instead of needing a permanent side table
  mapping them to pinki's. **The join**: DESIGN.md § 7 offers the ledger join as
  pinki's answer to commitment misalignment, and it is keyed by promise id — so it
  now works between parties who did not both mint here, which is the only version of
  it that was ever worth much.

  What did **not** change, on purpose: there is no `external_id` field. Two id fields
  is two ways to name one promise and a decision, per reader, about which one the `on`
  edge and the A2A `metadata` block key on. One opaque field is the smaller thing that
  works. And uniqueness is unchanged and non-negotiable — declaring an id the ledger
  already declares is refused with § 4's first-declaration-wins message whether it was
  minted or supplied, which is the collision check the six-hex space never really had.

- **`pnk_` is reserved for minted ids.** A supplied id wearing that prefix without
  being six lowercase hex digits is refused (exit 2, nothing appended). The
  reservation keeps "was this minted here?" answerable from the id alone, which is
  what lets an unknown-id error distinguish a typo — `pnk_bogus` could never have
  been declared — from a plain lookup miss. An adopter gives up nothing: an id space
  that already starts with `pnk_` is pinki's own.

- **The unknown-id hint no longer calls a foreign id malformed.** `pinki show bogus`
  used to add "(and `bogus` is not even a well-formed pinki id)". Since § 1 now admits
  a caller's own id, that sentence is false — `bogus` is a perfectly declarable id —
  so the hint fires only for the reserved-prefix case above.

### Compatibility

- **This one is not breaking, and that is a measured claim rather than a hope.** A
  ledger containing foreign ids is read by **v0.1.0** — the whole file, every verb —
  because nothing about the record's *shape* changed: the `promise` event carries the
  same keys in the same order, and v0.1.0's `--id` refusal was a check on **input**,
  never on what it reads back. Observed against the v0.1.0 binary, on a ledger this
  build wrote:

  ```console
  $ pinki --version
  pinki 0.1.0
  $ PINKI_LEDGER=./foreign.jsonl pinki ls --all
  pr-20260906203134-225e24cd  satisfied  2099-01-01T00:00:00Z  reviewer -> author   hand back a reviewed schema
  pnk_cfb64b                  detached   2099-01-02T00:00:00Z  author -> publisher  ship it
  $ echo $?
  0
  ```

  `show`, `ls --json` and `a2a task` on the foreign id all succeed on v0.1.0 too, and
  so do `resolve` and `assess` against it — those verbs never validated an id's shape,
  they only looked it up. The single thing v0.1.0 cannot do with a foreign id is
  **declare** one: `pinki promise --id pr-20260906203134-225e24cd` still exits 2
  there, which is issue #5 itself.

  So the upgrade is one-directional in exactly one place: a ledger with foreign ids in
  it can be read by an old reader and *appended to* by an old reader; it just cannot
  have had that first line written by one.

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
