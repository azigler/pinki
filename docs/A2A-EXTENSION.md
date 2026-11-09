# pinki as an A2A extension

> **v1 is the default CLI binding. v0 is deprecated and will be removed in the next minor release.**
> The URI remains provisional — see [§6](#6-the-uri-is-provisional).
> Read [DESIGN.md](DESIGN.md) first; this document only describes how that vocabulary
> rides [A2A](https://a2a-protocol.org).

Everything below was checked against the normative source — `specification/a2a.proto`
in [a2aproject/A2A](https://github.com/a2aproject/A2A) — and the
[extensions guide](https://a2a-protocol.org/latest/topics/extensions/), on 2026-08-28.
Where the spec says *should* rather than *must*, this document says so too.

## v1: one metadata key, an ordered promise list

The v1 URI is `https://github.com/azigler/pinki/ext/promise/v1`. It uses the
same three A2A surfaces described below. An AgentCard declares that URI with
`required: false`; a client activates it with the `A2A-Extensions` header; a
Task carries one metadata key, `<v1 URI>/promises`. The value is an array of
promise read records in the order requested. An array with one record is still
an array. A Task can therefore carry several promises.

Each record has the declaration fields from [DESIGN.md §1](DESIGN.md#1-the-record)
and the computed fields from `show --json`. It also has `node` (the publishing
host name), `qid` (`HOST:SEAT:ID`), and `provenance` with `as_of` and
`ledger_ref` and `record_ref`. `qid` is a `host:seat` address with the promise
id appended. The bare `id` stays in the record. `ledger_ref` is
`HOST:sha256:<digest>`; the digest covers the same ledger bytes as v0, the
whole ledger. All records in one response have the same ledger reference and
read instant. `record_ref` is the per-promise reference; see
[Comparing a held copy](#comparing-a-held-copy-ledger_ref-and-record_ref).

```json
{
  "https://github.com/azigler/pinki/ext/promise/v1/promises": [
    {
      "id": "pnk_4f3a91", "promise": "hand back a reviewed schema",
      "by": "reviewer", "to": "author", "until": "2026-09-01T17:00:00Z",
      "ts": "2026-08-28T20:14:03Z", "state": "detached",
      "horizons": [{"ts": "2026-08-28T20:14:03Z", "until": "2026-09-01T17:00:00Z", "by": "reviewer"}],
      "resolution": null, "corrections": [], "assessments": [],
      "node": "box", "qid": "box:reviewer:pnk_4f3a91",
      "provenance": {"as_of": "2026-09-01T16:05:00Z", "ledger_ref": "box:sha256:<ledger-snapshot-digest>",
                     "record_ref": "box:sha256:<this-promise-digest>"}
    }
  ]
}
```

The example shows the container and address fields. Run `pinki a2a card` for the v1
declaration and `pinki a2a task --node HOST --seat SEAT ID [ID ...]` for the
metadata block. `--seat` is required for v1. `--node` and `--seat` mean nothing
in v0, so `--ext v0` refuses them with a usage error (exit 2). `--node` defaults to `PINKI_NODE`,
then the short host name (from `hostname -s`). A set but empty `PINKI_NODE` is an
error, not a fallback. Node and seat must be non-empty and have no colon or
whitespace. IP addresses cannot be nodes. `--now ISO` fixes `as_of` as in v0.
An unknown id fails the whole command with no metadata on stdout.

### Comparing a held copy: `ledger_ref` and `record_ref`

Two references ride in `provenance`, and they answer different questions.

- `ledger_ref` names the **whole ledger**. A write to any promise by any seat
  changes it, so it differs on every held copy within minutes on a busy ledger.
  A difference means "re-read, something in the ledger changed". It does not
  mean this promise changed.
- `record_ref` names **this promise's own history**. It is `HOST:sha256:<digest>`
  with the same node prefix as `ledger_ref`. It changes whenever a line that can
  change this promise's computed record is appended. A write to another promise
  does not change it. It can also change on a line the fold then ignores (a
  re-declaration of the same id, or a correction of an antecedent by someone
  other than its resolver); that errs toward a re-read, never toward a missed
  change.

A client that holds a copy compares both with a fresh read from the
authoritative node: **re-read if `ledger_ref` differs; this promise changed only
if `record_ref` differs.** Equal `record_ref` means the held record is current in
every field except the time-dependent ones (`state` can still turn `overdue`;
judge that with `as_of`, as before). Equal `ledger_ref` means nothing in the
ledger changed, and implies equal `record_ref`. Compare references only when
both carry the same node prefix.

For example, a client holds promises `a` and `b`:

```text
held:   a  ledger_ref box:sha256:1111  record_ref box:sha256:aaaa
        b  ledger_ref box:sha256:1111  record_ref box:sha256:bbbb
fresh:  a  ledger_ref box:sha256:2222  record_ref box:sha256:a2a2   <- a was amended
        b  ledger_ref box:sha256:2222  record_ref box:sha256:bbbb   <- b is unchanged
```

`ledger_ref` differs for both, so the client re-reads. Only `a` has a new
`record_ref`, so only `a`'s held copy is stale. `b` is the same record.

**The rule for "this promise's own history".** The digest is SHA-256 over the
selected ledger lines in log order. Each line is hashed as it stands in the file
(no re-serialization), without its line terminator, followed by one `\n`. The
selected lines are:

1. Every event whose subject is this promise: its `promise` declaration, and
   every `amend`, `resolve`, `correct` and `assess` line that names it.
2. For each antecedent reachable through the `on` chain from this promise (an
   id's first declaration supplies its `on`; a cycle is followed once): that
   antecedent's `promise`, `resolve` and `correct` lines. Those can change this
   promise's computed `state` (an antecedent that ends unsatisfied expires its
   dependents). The antecedent's `amend` and `assess` lines are not selected: a
   dependent's state reads only whether its antecedent is satisfied, ended
   unsatisfied, or still open.

Lines of an event type the publisher does not know are never selected; the fold
ignores them. The same selected lines in a copied ledger give the same digest,
and a path never appears in it.

`record_ref` is an additive field inside v1. It is not a new URI, because it
changes no existing field and removes nothing (see [§6](#6-the-uri-is-provisional)).
A reader that does not know `record_ref` ignores it and falls back to
`ledger_ref`. A v1 reader **must** ignore `provenance` fields it does not know.
`ledger_ref` keeps its value and meaning. v0 has no `record_ref`.

For one minor release, `pinki a2a card --ext v0` and `pinki a2a task --ext v0 ID`
emit the original v0 blocks byte for byte. v0 is deprecated and will be removed
in the next minor release. The sections below retain its original record and
examples as the compatibility reference.

## 0. Why an extension at all

A2A has no deadline. `Task` carries `id`, `context_id`, `status`, `artifacts`,
`history`, `metadata`; `TaskStatus` carries `state`, `message`, and a `timestamp`
documented as *"ISO 8601 Timestamp when the status was recorded."* That is when
something **happened**, never when anything is **due**. There is no TTL, no due date,
and no expiry field anywhere in the message set.

This is a known gap.
[Issue #857, "Add Ability to Specify Deadlines for Tasks"](https://github.com/a2aproject/A2A/issues/857),
has been open since 2025-07-10. In June 2026 a project contributor replied that it
*"seems like a great idea for an A2A extension following the process here … if there
is enough adoption then it can be made official or even upstreamed in the main spec."*
That is the route this document takes. It is a contributor's suggestion on an open
issue, not a commitment from the project, and nothing here is endorsed by A2A.

## 1. The three surfaces, and only these three

A2A extensions are constrained. They exist "primarily to prevent breaking core type
validations," so an extension must not add fields to, or remove required fields from,
protocol-defined structures; for enums the guidance is to *"use existing enum values
and annotate additional semantic meaning in the `metadata` field."*

pinki therefore uses exactly three surfaces and introduces no new state, field, role,
or method.

### Surface 1: the AgentCard declaration

`AgentCapabilities.extensions` is a repeated `AgentExtension`, whose fields are
`uri`, `description`, `required` (bool), and `params` (a struct).

```json
{
  "capabilities": {
    "extensions": [
      {
        "uri": "https://github.com/azigler/pinki/ext/promise/v0",
        "description": "Promises made by this card's subject: deadline, evidence on satisfaction, reason on abandonment.",
        "required": false,
        "params": {
          "assessments": "https://example.org/agents/reviewer/assessments.jsonl"
        }
      }
    ]
  }
}
```

`required` is **false**, always. A promise is voluntary; an extension that refused to
talk to agents who do not track obligations would be a strange way to express that.

Declaring the extension asserts one thing that A2A itself does not model:

> **Obligations recorded under this URI bind the subject of this card, and survive
> the death of any session, connection, or process that made them.**

That is the whole reason the declaration lives on the *card* rather than in a task.
Cards are durable and discoverable; sessions are not. The agent that owes something
is the card's subject, and restarting it does not settle the debt.

`params.assessments` is optional: a URL where this agent publishes its own
assessments of promises it is party to. Absent means "not published here," never
"none exist."

### Surface 2: the request header

A client signals per-request activation with the `A2A-Extensions` header, a
comma-separated list of extension URIs:

```http
A2A-Extensions: https://github.com/azigler/pinki/ext/promise/v0
```

The server **should** echo back the extensions it actually activated. Note that this
is a *should*, not a guarantee — so **do not build logic that depends on the echo**.
Treat its absence as "unknown," not as "declined."

Because pinki never makes network calls ([DESIGN.md § 7](DESIGN.md#7-the-honest-limit)),
your A2A client sets this header, not pinki.

### Surface 3: URI-prefixed metadata keys

`Task.metadata` exists and is a free-form key/value struct. The extensions guide
routes custom attributes there, with URI-prefixed keys — shown by example
(`"https://example.com/ext/konami-code/v1/code": "motherlode"`) rather than mandated
in normative language. pinki follows the convention.

On a `Task` whose work a promise is about (excerpt showing the metadata block):

```json
{
  "id": "a2a-task-9c1f0e",
  "metadata": {
    "https://github.com/azigler/pinki/ext/promise/v0/promise": {
      "id":      "pnk_4f3a91",
      "promise": "hand back a reviewed schema",
      "by":      "https://example.org/agents/reviewer",
      "to":      "https://example.org/agents/author",
      "on":      "pnk_0c2b77",
      "until":   "2026-09-01T17:00:00Z",
      "task":    "a2a-task-9c1f0e",
      "ts":      "2026-08-28T20:14:03Z",
      "state":   "satisfied",
      "horizons": [{"ts":"2026-08-28T20:14:03Z","until":"2026-09-01T17:00:00Z","by":"https://example.org/agents/reviewer"}],
      "resolution": {"as":"satisfied","ts":"2026-09-01T16:02:11Z","by":"https://example.org/agents/reviewer","evidence":["https://example.org/reviews/91"]},
      "assessments": [],
      "provenance": {"as_of":"2026-09-01T16:05:00Z","ledger_ref":"sha256:<ledger-snapshot-digest>"}
    }
  }
}
```

One key, one value. The declaration fields remain at the same path, and `pinki
a2a task --ext v0` adds the same computed read fields as `show --json`: `ts`, optional
timestamp `source`, `state`, `horizons`, `resolution`, and `assessments`. An open
promise has `resolution: null`; a satisfied one carries its resolution and evidence.
The `state` here is pinki's computed promise state, **not** `TaskStatus.state`.
It is true **as of** `provenance.as_of` only. A receiver uses that UTC instant to
judge whether the computed state is stale before relying on it. `pinki a2a task
--ext v0 --now ISO ID` fixes the instant to a UTC second for reproducible output; without
`--now`, pinki uses the current UTC second. `provenance.ledger_ref` is an opaque
`sha256:` reference to the ledger snapshot that supplied the read model. It changes
when that snapshot changes, stays the same for a copied ledger, and never exposes a
local filesystem path. Compare it with a fresh reference from the authoritative
ledger when one is available. Across a trust boundary, send the record verbatim
plus `{ledger_ref, as_of}`, so that the reader can detect a stale copy.

The optional `source` on the declaration, horizons, resolution, and assessments
uses the same names and values as `show --json`. At most one promise per Task
rides under v0. After an `amend`, the declaration's `until` here is
the current horizon.

The record may also carry `meta`: an optional, opaque object that the declaring
system uses for its own provenance
([DESIGN.md § 1](DESIGN.md#1-the-record)). It rides the metadata block along with
everything else, because the declaration stays in place and `meta` is part of the
record. **A reader of this extension must not interpret it, and must not require
it** — treat an absent `meta` as "nothing was supplied", never as a conformance
failure, and treat a present one as bytes belonging to somebody else's system. It is
not a place to smuggle in extension semantics: anything this specification actually
means belongs in this specification, under this URI, where a second implementer can
read it. Resolution and assessment events may carry their own `meta` in their read
objects; those objects are opaque too.

`promise.task` and `Task.id` are the same value seen from both ends. The redundancy
is deliberate — a promise record handed to you on its own still says what it was
about, and a Task handed to you on its own still carries its obligation.

**`TaskStatus` has no `metadata` field.** Do not attempt to attach anything to a
status; per the extensions guide, status-adjacent custom data belongs on
`TaskStatus.message`, which is a `Message` and does have `metadata`.

## 2. Resolution maps onto states that already exist

No new `TaskState` is introduced. The enum is closed at nine values
(`TASK_STATE_UNSPECIFIED`, `_SUBMITTED`, `_WORKING`, `_COMPLETED`, `_FAILED`,
`_CANCELED`, `_INPUT_REQUIRED`, `_REJECTED`, `_AUTH_REQUIRED`) and pinki adds none.

| pinki | A2A task state | Also required |
|---|---|---|
| `satisfied` | `COMPLETED` | ≥1 `Artifact`. |
| `cancelled` | `REJECTED` | Non-empty `TaskStatus.message`, carrying the reason. |
| `released` | `CANCELED` | `TaskStatus.message` naming the creditor who released it. |
| `expired` | *(no mapping)* | The antecedent never held; usually no Task was created. |
| `overdue` | **no state change** | Computed by readers from `until`. See below. |
| `violated` | **never a task state** | An assessment. See [§3](#3-assessments-ride-as-attributed-history). |

Spell state names in whichever form your transport binding uses — the proto's
`TASK_STATE_COMPLETED` and the JSON binding's shorter form denote the same value.

Two rules that make this more than a renaming:

**Evidence is not optional.** Under this extension, `COMPLETED` with an empty
`artifacts` list is non-conformant. `Artifact` carries both a `metadata` struct and a
`repeated string extensions` field, so an artifact offered as evidence for a promise
should list this extension's URI in `extensions`, which lets a reader tell evidence
apart from incidental output.

**`overdue` is deliberately not a task state, and that asymmetry is load-bearing.**
The server owns its task's transitions. If "the deadline passed" were a state the
server could enter, the agent that owes the promise could transition its own way out
of owing it — declare itself lapsed and stop being asked. So a promise past `until`
produces *no* A2A state change at all. The task stays exactly as open as it was, and
`now > until` is arithmetic any reader performs for itself. Nobody has to be trusted
to report it, and nobody can suppress it.

## 3. Assessments ride as attributed history

`pinki a2a task` includes the ledger's assessments in `metadata` under the same
extension URI. Each entry keeps the `show --json` names: `ts`, optional `source`,
`state`, `observer`, optional `note` and `supersedes`, `superseded_by` (or `null`),
`after_resolution`, and optional `meta`. `violated` is an attributed judgment, never
a `TaskStatus.state` or a computed promise state. The array keeps conflicting
judgments and superseded lines in log order; the last writer does not win.

Assessments live on the assessing agent's own ledger, optionally advertised through
`params.assessments` on its card ([Surface 1](#surface-1-the-agentcard-declaration)).
Two agents may publish contradictory assessments of the same promise id. Both are
legitimate; joining them on the id is how you find the disagreement. That is the
whole of [DESIGN.md § 7](DESIGN.md#7-the-honest-limit) restated in wire terms.

## 4. Observation is a plain client

A2A has two roles: A2A Client and A2A Server. There is no observer, auditor, or
broker role, and this extension does not invent one.

An agent watching promises is an ordinary authorized client using ordinary methods —
`GetTask`, `ListTasks`, and the push-notification configuration methods
(`CreateTaskPushNotificationConfig` and its siblings) — subject to exactly the
authorization every other client faces. It sees what it has been granted and nothing
more. If that is not enough visibility to assess a promise, the honest answer is that
it cannot assess it, not that it should be given a privileged seat.

## 5. Conformance

An agent conforms if it:

1. Declares the URI in `capabilities.extensions` with `required: false`.
2. For v1, puts promises under the URI-prefixed `metadata` key as a list; each
   record carries `node` and `qid` as well as the read model and provenance.
   Should put `provenance.record_ref` beside `ledger_ref` on each v1 record;
   a publisher without it is still conformant, and a reader then has only the
   coarse `ledger_ref`.
   Carries a record's `meta` through untouched if it has one, without depending
   on it. For deprecated v0, puts at most one promise record per Task under its
   `/promise` key.
3. Attaches ≥1 `Artifact` when completing a Task that carries a promise.
4. Puts a non-empty reason in `TaskStatus.message` when rejecting or cancelling one.
5. Never enters a task state to signal that a deadline passed.
6. Keeps assessments attributed in the extension metadata, without mapping their
   judgments onto `TaskStatus.state`.

Conformance is unenforceable and deliberately so. Nothing in pinki can compel any of
this, and an agent that ignores every rule here will not be stopped — it will only be
observable, which is the most any of this was ever going to offer.

## 6. The URI is provisional

`https://github.com/azigler/pinki/ext/promise/v1` is the current URI; `v0` was the
first, and the move from v0 to v1 (a list replaced the single record) is the rule
below applied once already. Both are placeholders chosen because they sit on an
origin the project actually controls. A2A places no scheme or domain
requirement on third-party extensions — *"anyone is able to define, publish, and
implement an extension"* — but it does say the specification **should** be hosted at
the extension's URI, and encourages a permanent identifier service such as
[w3id.org](https://w3id.org). A GitHub path satisfies neither well.

Official extensions live under `https://a2a-protocol.org/extensions/`, in the
`a2aproject` organization with an `ext-` repository prefix. That is a process, not a
decision this repo can make for itself.

Also normative and worth stating early: **a breaking change MUST use a new URI.**
Hence `v0`, then `v1`, and hence the expectation that it will move again.

This is [open question 1](DESIGN.md#8-open-questions). Opinions welcome.
