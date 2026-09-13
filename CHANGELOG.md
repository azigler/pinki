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
- **Forward tolerance for event types.** An event whose `type` this build does not know
  is now read, warned about on stderr with its line number, and ignored by the fold,
  instead of aborting the whole ledger. One unknown line no longer costs you the file.
  A line that is not an event at all is still fatal — "an event I have not heard of" and
  "not an event" are different things, and only the first is survivable.

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

- **`meta` on `amend` too**, by the same rule and through the same `--meta` flag:
  `pinki amend <ID> --until ISO [--meta '<json object>']` writes it last on the amend
  line, and `show` hands it back under the horizon it belongs to — `horizons[].meta`
  in `--json`, one line beneath that horizon in the text block. An escalation ladder is
  the thing that writes most amends and exactly the writer with provenance to carry
  (which rung, which attempt, under which policy); shipping `amend` as the one
  meta-less event would have made a ladder project away the half of its row that says
  who was nudging. The declaration's own horizon carries no `meta` — a promise's
  provenance is the record's, already handed back with the record. Compatibility is
  unchanged from what the `amend` event itself already states: a ledger with an amend
  line in it needs this version or later, whether or not that line carries `meta`.

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

### Changed

- **The deadline in `ls`, `show` and `a2a task` is the current one**, not the
  declared one, once a promise has been amended. The declaration itself is never
  rewritten: `first declaration wins` is unchanged, and a fold that skips `amend`
  events still answers what it answered before — the declared horizon.

- **`--id` accepts an id you already have.** It used to refuse anything that was not
  `pnk_` plus six lowercase hex digits, which made the one affordance that looks like
  "carry your ledger over" not be one. A supplied id is now opaque: any non-blank
  string with no whitespace, no control characters and no invisible characters, up to
  128 characters. Minting
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

- **An id may not contain an invisible character.** A supplied id holding a character
  with the Unicode **`Default_Ignorable_Code_Point`** property — zero-width spaces and
  joiners, the bidi controls, variation selectors, tag characters, soft hyphen, the
  Hangul fillers — is refused (exit 2, nothing appended, message citing § 1 and naming
  the code point, which is the only way to see a character that renders as nothing).

  It is its own rule because neither of the other two catches it: U+200B ZERO WIDTH
  SPACE is `White_Space=No` in Unicode despite its name, and none of these are control
  characters, so `char::is_whitespace()` and `char::is_control()` are both `false`.
  Without the rule, two ids that are **not equal** can be **indistinguishable** — in
  `pinki ls`, in a terminal, in a code review, and to whoever is deciding whether § 7's
  join matched. An identifier whose equality the eye cannot check is not a handle.

  Non-ASCII is otherwise entirely fine, and tested: `υπόσχεση-91` and `約束-91` are
  accepted. This is a rule about invisibility, not about script. The set is Unicode
  16.0.0's (`DerivedCoreProperties.txt`), carried as a 17-range table in `src/id.rs`
  rather than a new dependency — Cargo.toml's list is DESIGN.md § 7's no-network
  invariant, so a crate has to earn itself, and this one would buy seventeen lines.

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

- **A ledger containing an `amend` line requires this version or later. v0.1.0 refuses
  it — the whole file, not the line.** Measured, not assumed:

  ```console
  $ pinki --version
  pinki 0.1.0
  $ PINKI_LEDGER=./with-amends.jsonl pinki ls --all
  pinki: ledger ./with-amends.jsonl: line 2 is not a valid pinki event: unknown
  variant `amend`, expected one of `promise`, `resolve`, `assess`
  $ echo $?
  1
  ```

  Its parser has no fallback for an unknown `type`, and a line that will not parse is
  fatal to the read. So this is **breaking for old readers**, though not for the record
  shape: the `promise` event is byte-identical to what v0.1.0 wrote, and an old reader
  handed a ledger with no amends in it is unaffected. The forward-tolerance above is
  the fix going forward, and it can only ever help the *next* event type — it cannot
  reach backwards into a binary already installed.

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

- **The invisible-character rule is a check on input too, and worth saying plainly.**
  It refuses a *declaration*; it does not retro-validate a ledger. A row whose id
  already holds an invisible character — written by another implementation, by a hand
  edit, or by an earlier build of this branch — is still read, shown, resolved and
  assessed by **this** version, because no read path re-checks an id's shape. An old
  reader reads it too, for the same reason it reads any other foreign id. The
  asymmetry is therefore the same one as above and no bigger: an id this version will
  not let you declare is an id both old and new readers will still show you. It is
  your ledger; nothing here rewrites or hides a line you already have.

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
  is refused, rc 2, and it has no `--meta` flag either (`error: unexpected argument
  '--meta' found`, rc 2). The stdin refusal, verbatim:

  ```
  unknown field `meta`, expected one of `id`, `promise`, `by`, `to`, `on`, `until`, `task`
  ```

So the honest statement is narrower than "old readers are fine" and narrower than
"old readers break": **a ledger with `meta` needs this version or later to be read
without silent loss.** Provenance is the whole reason the field exists, and a reader
that drops it exits 0 while telling you less than the file says.

One more limit, because "verbatim" should mean what it says: `meta` is stored as a
JSON object and re-emitted with its keys **sorted**, so key order is canonicalised on
the first write. Values, types and nesting round-trip exactly, and every read surface
agrees byte for byte with the ledger line — but if you are diffing bytes against your
own input, diff against the first write instead.

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

- **158 tests** — 96 unit, 62 driving the real binary — including the three-step
  escalation ladder from #9 end to end, and the forward-tolerance property tested
  where it actually lives: a real ledger *file* carrying a line of an unknown type,
  read by the real binary, folding to the same answer the known events alone give. The
  deliberately uncovered lines at the end of `tests/cli.rs` are unchanged in kind;
  their line numbers moved.

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
