# pinki 🤙

[![CI](https://github.com/azigler/pinki/actions/workflows/ci.yml/badge.svg)](https://github.com/azigler/pinki/actions/workflows/ci.yml)

pinki is a promise ledger: who owes what to whom, by when, with evidence when it is done.
Use it when agents need to carry obligations across tasks, sessions, or organizations.

## Install

```text
cargo install --git https://github.com/azigler/pinki --tag v0.4.0 --locked
```

Or take the prebuilt Linux x86_64 binary from the v0.4.0 release:
https://github.com/azigler/pinki/releases/tag/v0.4.0. Check with `pinki --version`.

## Start a ledger and declare a promise

Run these examples in order, in ONE shell session, in an empty directory. The later
blocks reuse `PINKI_LEDGER` and `$id` from this one. An agent tool that starts a fresh
shell per call must re-export `PINKI_LEDGER` and pass the id literally.
The first `promise` creates the file. It prints an id, a tab, and the computed state.

```sh
export PINKI_LEDGER="$PWD/ledger.jsonl"
receipt=$(pinki promise "Review the schema" \
  --by https://example.org/agents/reviewer \
  --to https://example.org/agents/author \
  --until 2030-01-01T00:00:00Z \
  --task a2a-task-91)
id=${receipt%%$'\t'*}
printf '%s\n' "$receipt"
```

`by` owes the work; `to` is owed it. `--task` links an A2A Task.
`--on ID` makes it wait on another promise (state `conditional`). `--id ID` brings
your own id (avoid the reserved `pnk_` prefix; not one already declared).
Otherwise pinki mints `pnk_` plus six hex digits.

For a historical import, add `--ts ISO` to `promise`, `amend`, `resolve`, or
`assess`, or put `ts` in a promise record on stdin. The event gets
`"source":"supplied"`; without it pinki records `"source":"cli"`. The timestamp
may be past, but only 300 seconds ahead of the current clock. `show --json` and
`ls --json` include the declaration `ts`; `show` marks supplied event times.

## Check, change, and finish it

`ls` shows open promises by default. Add `--overdue` for the overdue subset,
`--all` to include resolved promises, or `--json` for a machine-readable array.

```sh
pinki ls --json
```

`show` gives one promise, its current deadline, resolution, and assessments.

```sh
pinki show "$id" --json
```

The `on` edge links a promise to the one it waits for. `ls --tree` shows the
forest, with each promise's computed state. `show ID --waiters` lists every
promise that waits on `ID`, including indirect waiters. Add `--json` for nested
tree rows or a flat waiter list with `depth` (direct waiters have depth 1).
`--all` includes settled antecedents in the tree; the usual `ls` state and
party filters still apply.

```sh
waiter_receipt=$(pinki promise "Publish the reviewed schema" \
  --by https://example.org/agents/author \
  --to https://example.org/agents/publisher \
  --until 2030-01-03T00:00:00Z --on "$id")
waiter_id=${waiter_receipt%%$'\t'*}
pinki ls --tree --all
pinki show "$id" --waiters --json
```

If `on` names an id absent from this ledger, the promise appears as a root
marked `unknown antecedent` in text and `unknown_antecedent` in tree JSON.

The debtor can move the deadline. pinki keeps each earlier deadline in the ledger.

```sh
pinki amend "$id" --until 2030-01-02T00:00:00Z --reason "Review needs another day"
```

Resolve with a reference that the other agent can inspect. Other outcomes are
`--cancelled --reason TEXT` and `--released` (by the creditor).

```sh
pinki resolve "$id" --satisfied --evidence https://example.org/reviews/schema-91
pinki ls --all
```

`pinki assess ID --violated --observer WHO` publishes an attributed judgment
without resolving the promise. An observer corrects or withdraws its own judgment with
`--supersedes TS` (the earlier assessment's `ts` from `show`), plus `--violated` or
`--retracted`. Only the same observer may supersede, and the earlier line stays in the
ledger. `show` marks each assessment as superseded or not, and as made before or after
the resolution (`after_resolution` in `--json`). `pinki export` gives a versioned JSON view (`--now ISO` makes it reproducible);
`--format jsonl` prints one promise per line.

## States and exit codes

| Computed state | Meaning |
|---|---|
| `conditional` | Waits on the promise named by `on`. |
| `detached` | Owed now and unresolved. |
| `overdue` | Detached and past its current deadline. |
| `satisfied` | Debtor resolved it with evidence. |
| `cancelled` | Debtor resolved it with a reason. |
| `released` | Creditor let the debtor off. |
| `expired` | Its antecedent ended without satisfaction. |

`violated` is only published by an assessment. It is never a computed state.

| Exit | Caller sees |
|---|---|
| `0` | Success. |
| `1` | Operational failure: missing ledger on a read, unknown id, resolved promise, or ledger I/O. |
| `2` | Malformed input or CLI usage. |

Errors go to stderr. `ls`, `show`, `export`, and `a2a task` exit 1 if the
ledger file is missing; an existing empty file reads as empty (exit 0).

## Teach another agent

Paste this into an AGENTS.md, CLAUDE.md, or agent prompt. Replace the path and identities:

```text
Track promises with pinki. Set PINKI_LEDGER to the shared ledger's absolute path.
To declare work you owe, run pinki promise "WHAT" --by YOUR_AGENT_CARD_URI
--to PEER_AGENT_CARD_URI --until ISO_8601_INSTANT. Keep the first tab-separated
field of stdout as its id. Check with pinki ls --json and pinki show ID --json.
When done, run pinki resolve ID --satisfied --evidence INSPECTABLE_REF.
To move your deadline, run pinki amend ID --until ISO_8601_INSTANT --reason TEXT.
If you cannot deliver, run pinki resolve ID --cancelled --reason TEXT.
Exit 1 means a missing ledger, an unknown id, or an already-resolved promise
(stderr says which). Exit 2 means a malformed command.
```

For A2A, advertise the card extension and merge the block into Task `metadata`.
Your client sets `A2A-Extensions: https://github.com/azigler/pinki/ext/promise/v0`.

```sh
pinki a2a card
pinki a2a task "$id"
```

`a2a task` emits this metadata shape (the id comes from the ledger):

```json
{"https://github.com/azigler/pinki/ext/promise/v0/promise": {"id": "pnk_4f3a91",
  "promise": "Review the schema",
  "by": "https://example.org/agents/reviewer", "to": "https://example.org/agents/author",
  "until": "2030-01-02T00:00:00Z", "task": "a2a-task-91"
}}
```

`pinki a2a` writes JSON to stdout; your A2A client sends it. pinki makes no network calls.

## Boundary and references

`overdue` is deadline arithmetic. pinki never decides that a promise was broken.
Read [DESIGN.md](docs/DESIGN.md), [A2A-EXTENSION.md](docs/A2A-EXTENSION.md),
[PRIOR-ART.md](docs/PRIOR-ART.md), [CONTRIBUTING.md](CONTRIBUTING.md), and
[CHANGELOG.md](CHANGELOG.md) for the design, wire format, lineage, and changes.

MIT — see [LICENSE](LICENSE).

*pinki builds on [A2A](https://a2a-protocol.org), an [AAIF](https://aaif.io)-hosted project. pinki is not affiliated with, endorsed by, or speaking for either.*
