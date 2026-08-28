# pinki as an A2A extension

> **v0 draft. The extension URI is provisional — see [§6](#6-the-uri-is-provisional).**
> Read [DESIGN.md](DESIGN.md) first; this document only describes how that vocabulary
> rides [A2A](https://a2a-protocol.org).

Everything below was checked against the normative source — `specification/a2a.proto`
in [a2aproject/A2A](https://github.com/a2aproject/A2A) — and the
[extensions guide](https://a2a-protocol.org/latest/topics/extensions/), on 2026-08-28.
Where the spec says *should* rather than *must*, this document says so too.

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

On a `Task` whose work a promise is about:

```json
{
  "id": "a2a-task-9c1f0e",
  "status": { "state": "TASK_STATE_WORKING", "timestamp": "2026-08-28T20:14:03Z" },
  "metadata": {
    "https://github.com/azigler/pinki/ext/promise/v0/promise": {
      "id":      "pnk_4f3a91",
      "promise": "hand back a reviewed schema",
      "by":      "https://example.org/agents/reviewer",
      "to":      "https://example.org/agents/author",
      "on":      "pnk_0c2b77",
      "until":   "2026-09-01T17:00:00Z",
      "task":    "a2a-task-9c1f0e"
    }
  }
}
```

One key, one value: the promise record verbatim, exactly as
[DESIGN.md § 1](DESIGN.md#1-the-record) defines it. At most one promise per Task
under this extension.

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
| `violated` | **never a task state** | An assessment. See [§3](#3-assessments-do-not-ride-a2a). |

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

## 3. Assessments do not ride A2A

`violated` never appears on a Task, in `metadata`, or anywhere in the A2A message
flow. It is not a fact about the task; it is one agent's judgment about another
agent's conduct, and putting it in the shared record would let whoever writes last
win an argument that has no referee.

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
2. Puts at most one promise record per Task under the URI-prefixed `metadata` key,
   in the shape [DESIGN.md § 1](DESIGN.md#1-the-record) defines.
3. Attaches ≥1 `Artifact` when completing a Task that carries a promise.
4. Puts a non-empty reason in `TaskStatus.message` when rejecting or cancelling one.
5. Never enters a task state to signal that a deadline passed.
6. Never writes an assessment into a Task.

Conformance is unenforceable and deliberately so. Nothing in pinki can compel any of
this, and an agent that ignores every rule here will not be stopped — it will only be
observable, which is the most any of this was ever going to offer.

## 6. The URI is provisional

`https://github.com/azigler/pinki/ext/promise/v0` is a placeholder chosen because it
is an origin the project actually controls. A2A places no scheme or domain
requirement on third-party extensions — *"anyone is able to define, publish, and
implement an extension"* — but it does say the specification **should** be hosted at
the extension's URI, and encourages a permanent identifier service such as
[w3id.org](https://w3id.org). A GitHub path satisfies neither well.

Official extensions live under `https://a2a-protocol.org/extensions/`, in the
`a2aproject` organization with an `ext-` repository prefix. That is a process, not a
decision this repo can make for itself.

Also normative and worth stating early: **a breaking change MUST use a new URI.**
Hence `v0`, and hence the expectation that it will move.

This is [open question 1](DESIGN.md#8-open-questions). Opinions welcome.
