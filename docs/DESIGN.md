# pinki — design

> Status: **v0 draft, nothing is stable.** This document is the nucleus. If a
> feature is not in here, it is deliberately not in pinki.

pinki is three separable layers. Each one is adoptable without the ones below it.

1. **A vocabulary** — what a promise is, and the states it can be in.
2. **An A2A binding** — how that vocabulary rides [A2A](https://a2a-protocol.org)
   without changing it, using only the three surfaces A2A sanctions for extensions:
   the `capabilities.extensions` declaration on the AgentCard, the `A2A-Extensions`
   request header, and URI-prefixed keys in `metadata`. Full spec:
   [A2A-EXTENSION.md](A2A-EXTENSION.md).
3. **A CLI over an append-only log** — the reference implementation.

You can adopt (1) alone and write your own tooling. You can adopt (1)+(2) and
interoperate over A2A without ever running pinki. The third layer is the smallest
thing that makes the first two usable on a Tuesday.

---

## 1. The record

One record type. A promise.

```json
{
  "id":      "pnk_4f3a91",
  "promise": "hand back a reviewed schema",
  "by":      "https://example.org/agents/reviewer",
  "to":      "https://example.org/agents/author",
  "on":      "pnk_0c2b77",
  "until":   "2026-09-01T17:00:00Z",
  "task":    "a2a-task-9c1f0e"
}
```

| Field | Required | Meaning |
|---|---|---|
| `id` | yes | Stable identifier. Opaque. |
| `promise` | yes | Free text. The thing owed. pinki never parses it. |
| `by` | yes | The **debtor** — who owes. An A2A AgentCard identity. |
| `to` | yes | The **creditor** — who is owed. |
| `on` | no | Antecedent: the `id` of another promise. Absent means born owed. |
| `until` | yes | Deadline, ISO-8601. |
| `task` | no | The A2A `Task.id` this promise is *about*, when there is one. |
| `meta` | no | An opaque object, carried and never interpreted. See below. |

`task` is the one place the vocabulary reaches toward A2A, and it is deliberately
optional and one-directional. A promise may be about a Task, but it does not live
inside one: pinki works with no A2A anywhere, and an obligation can outlive the Task
that occasioned it. When `task` is set, the same record should also appear in that
Task's `metadata` — see [A2A-EXTENSION.md](A2A-EXTENSION.md).

Three deliberate absences:

- **No status field.** State is computed from the log, never stored. A record you
  are handed is enough to know what it says, never enough to know where it stands.
- **No `created` field.** The `promise` event's own `ts` already carries it. Storing
  it twice invites the two copies to disagree.
- **No priority, no tags, no assignee, no project.** Those are your orchestrator's
  business. pinki holds the obligation and nothing else — and `meta`, below, is where
  they cross the seam without pinki acquiring an opinion about them.

### `meta` is the one open door

Everything above it is closed. `meta` is not.

```json
{
  "id":      "pnk_4f3a91",
  "promise": "hand back a reviewed schema",
  "by":      "https://example.org/agents/reviewer",
  "to":      "https://example.org/agents/author",
  "until":   "2026-09-01T17:00:00Z",
  "meta":    { "by": "scheduler", "kind": "nudge", "ref": "run-4131" }
}
```

`meta` is an optional JSON **object**, and pinki does exactly three things with it:
stores it, hands it back, and never reads it. Not one key inside it is interpreted,
by the fold or by anything else.

> **The arithmetic fields stay closed; `meta` is the one open door; the fold never
> reads it.**

Both halves are load-bearing. A system adopting pinki already has rows carrying
provenance — which component declared this, on whose behalf, under which policy,
citing what — and the seven fields above have nowhere to put any of it. With no door,
adoption means projecting all of that away at the seam, and §4's "the log is the whole
system" quietly becomes *most* of it. With the door in the wrong place — a field the
fold consults — §3's "any two implementations produce the same answer" stops being
true, because the second implementation would have to agree about a field it has never
heard of.

The rules are all shape and no content:

| Rule | Why |
|---|---|
| It must be an object | A consumer has to be able to take the one key it understands and leave the rest alone. That is what makes `meta` safe to ignore. |
| It may not be empty | `"meta": {}` is provenance offered and left blank. Omit it instead: an absent `meta` writes no key at all. |
| It is capped at 8 KiB of JSON | A ledger line is a line. Provenance is a handful of short keys; a payload belongs behind a reference *in* `meta`, not inside it. |
| Nothing else | Nesting, arrays, nulls, keys pinki has never heard of — all fine. Opaque means opaque. |

A `meta` that breaks one of the first three is malformed input: exit 2, nothing
appended, like every other refusal at the edge (§5).

One honest limit. `meta` is held as a JSON object and re-emitted with its keys
**sorted**, so key order is canonicalised on the first write. Values, types and
nesting survive exactly, and JSON objects are unordered by definition, so nothing is
lost that the format ever promised to keep — but if you are diffing bytes, diff them
against the first write rather than against your input.

Every event may carry a `meta`, not only the record — see §4.

### `until` is required, and that is opinionated

It is the one place pinki insists. A promise without a deadline cannot be checked
by anyone, which makes it a wish. A2A's core has no deadline anywhere — `TaskStatus.timestamp`
records when a status *happened*, never when anything is *due* — so this is the
single field the extension exists to add.

---

## 2. The graph

`on` is the only edge, and it is the whole point.

```
  pnk_0c2b77   author  →  reviewer   "send the draft schema"      until 08-30
       │
       └── on ── pnk_4f3a91   reviewer  →  author   "hand back a review"  until 09-01
                      │
                      └── on ── pnk_88de10   author → publisher  "ship it"  until 09-03
```

A promise with `on: X` is not owed until X is satisfied. When X is satisfied, the
dependent **detaches** and the clock that matters starts mattering.

This is what separates a promise graph from a task graph, and it is worth being
precise about, because the two look identical in a diagram:

> **A task graph's edges are ordering. A promise graph's edges are obligation
> between named parties.**

A task DAG tells you what must happen before what. A promise graph tells you *who
owes what to whom*, and therefore who is left holding something when a node goes
quiet. The shape of a project versus the shape of a collaboration.

There is exactly one kind of edge because everything that can be depended on is
itself a promise. If you need to condition on something outside the graph — a build
going green, a human replying — someone promises it. That keeps the graph closed,
keeps the record count at one, and means pinki never needs an event bus, a webhook
receiver, or a condition language.

---

## 3. The states

The vocabulary comes from the **commitment** literature in multiagent systems, which
worked this out across the 2000s. We borrow it rather than invent a worse one:

- Singh, *A Social Semantics for Agent Communication Languages* (IJCAI-99 workshop) —
  the argument for grounding agent communication in public commitments rather than in
  each agent's private beliefs.
  [PDF](https://www.csc2.ncsu.edu/faculty/mpsingh/papers/mas/ijcai-99-acl.pdf)
- Yolum & Singh, *Commitment Machines* (ATAL 2001, LNCS 2333, pp. 235–247) — a
  commitment as `C(debtor, creditor, antecedent, consequent)`, and protocols expressed
  as the lifecycle of those commitments rather than as legal message orderings.
  [doi:10.1007/3-540-45448-9_17](https://doi.org/10.1007/3-540-45448-9_17)
- Chopra & Singh, *Cupid: Commitments in Relational Algebra* (AAAI 2015) — commitments
  **with deadlines**, and "which are violated?" compiled to a relational query.
  [doi:10.1609/aaai.v29i1.9443](https://doi.org/10.1609/aaai.v29i1.9443)

The state names below (`conditional`, `detached`, `satisfied`, `violated`, `expired`)
are that literature's, used with its meanings. We make no claim about which paper
coined which term — the lifecycle is refined across the line of work above and its
successors, and pinki is a consumer of it, not a historian of it.

pinki splits those states into two kinds, and **the split is the design**:

### Computed states — arithmetic, everyone agrees

Given the record and the log, any two implementations produce the same answer.

| State | When |
|---|---|
| `conditional` | `on` is set and the antecedent is not yet satisfied |
| `detached` | Owed now. Either no `on`, or the antecedent is satisfied |
| `overdue` | `detached`, unresolved, and `now > until` |
| `satisfied` | Resolved by the debtor, with evidence |
| `cancelled` | Resolved by the debtor, with a reason |
| `released` | Resolved by the creditor letting the debtor off |
| `expired` | The antecedent ended without being satisfied — never became owed |

### Asserted states — speech acts, attributed to whoever said them

| State | Who says it |
|---|---|
| `violated` | Someone. Never pinki-the-arithmetic. |

**`overdue` is a fact. `violated` is a judgment.** The gap between them is where
every honest design in this space either succeeds or lies.

`now > until` is arithmetic; two implementations that disagree about it have a bug.
"This promise was broken" is an assessment: it depends on what counts as done, on
context the record does not carry, and on who is doing the assessing.

Mark Burgess's [Promise Theory](http://markburgess.org/PromiseMethod.pdf) is the
sharpest statement of the constraint. Assessment of whether a promise was kept is
made by an *agent* — the promiser, the promisee, or a third party — and every one of
those assessments is subjective, made from that agent's own vantage and its own
imperfect information. No agent's assessment is automatically the true one, and an
intermediary that positions itself as the arbiter has, by construction, acquired the
ability to distort the very thing it claims to certify. (See also Burgess,
[*Cooperation in Human and Machine Agents*](https://arxiv.org/abs/2604.10505), 2026.)

So pinki computes the first column and **publishes, never computes, the second**.
An assessment is a record with an author, a timestamp, and a vantage point. It sits
beside the promise. It does not overwrite it, and it does not win.

Two observers may publish contradicting assessments of the same promise. pinki will
show you both. That is not a bug in pinki; it is the actual state of the world, and
a tool that hid it would be lying to you.

---

## 4. The log

Append-only JSONL. Three event types. State is a fold over the log.

```jsonl
{"ts":"2026-08-28T20:14:03Z","type":"promise","id":"pnk_4f3a91","promise":"hand back a reviewed schema","by":"…/reviewer","to":"…/author","on":"pnk_0c2b77","until":"2026-09-01T17:00:00Z"}
{"ts":"2026-09-01T16:02:11Z","type":"resolve","promise":"pnk_4f3a91","as":"satisfied","by":"…/reviewer","evidence":["https://example.org/reviews/91"]}
{"ts":"2026-09-02T09:00:00Z","type":"assess","promise":"pnk_88de10","state":"violated","observer":"…/author","note":"nothing shipped, no reason given"}
```

**`promise`** — speaks a promise into existence.

**`resolve`** — ends one. Three outcomes:

| `as` | Issued by | Requires |
|---|---|---|
| `satisfied` | debtor | `evidence`: one or more **references**. **Non-optional.** |
| `cancelled` | debtor | `reason`: non-empty text. |
| `released` | creditor | nothing. Reason encouraged. |

A **reference** is anything that points outward at a thing another party could go and
look at: a URL, a file path, a commit SHA, an A2A `Artifact.artifact_id`. pinki does
not fetch it, parse it, or validate that it exists — that would require network
access it does not have, and judgment it does not claim. It requires only that the
field is non-empty and that its contents point somewhere rather than describe
something.

That is the one rule pinki is strict about, and it is strict on purpose: "done" that
points at nothing is the exact failure this whole category of tool exists to catch.
A prose summary is welcome in `note`; it is not evidence. A resolve event with an
empty `evidence` list is malformed, and pinki says so.

Note what pinki does *not* do: it does not verify that `by` on the resolve event
matches `by` on the promise. It records who claimed what. Enforcement needs
authority pinki does not have and should not pretend to — see §6.

**`assess`** — publishes an assessment. Author, state, timestamp, optional note.
Never terminal: an assessed promise stays exactly as open as it was.

**`meta` on any of the three.** §1's open door is not only the record's. Resolving and
assessing carry their own provenance in an adopting system — which component resolved
this, on whose behalf, citing what — and that provenance belongs to the *act*, not to
the promise, so it rides the event that performed it:

```jsonl
{"ts":"2026-09-01T16:02:11Z","type":"resolve","promise":"pnk_4f3a91","as":"satisfied","by":"…/reviewer","evidence":["https://example.org/reviews/91"],"meta":{"by":"expected-gap-watchdog","cites_ref":"sha:9c1f0e"}}
```

Always last on the line, always optional, always opaque — §1's three shape rules, and
no interpretation anywhere. The fold does not read `meta` on any event, which is a
property you can check rather than a promise you have to take: mutate every `meta` in
a ledger and every computed state is identical.

Because state is a fold, the log is the whole system. Copy the file and you have
copied the state. Concatenate two ledgers and you have a joined view. Truncate it
and you have lied to yourself, which is a property, not a feature.

That claim is also why `meta` exists. "The log is the whole system" is only true for
an adopter if their row can go *into* the log whole; a seam that silently drops the
half pinki has no field for makes it "the log is most of the system", which is a much
weaker thing to build on.

---

## 5. The CLI

Six verbs. It should feel like a small issue tracker: push JSON in, pull JSON out,
no daemon, no server, no database.

```
pinki promise "hand back a reviewed schema" \
      --by reviewer --to author --until 2026-09-01T17:00Z [--on pnk_0c2b77] \
      [--meta '{"by":"scheduler","ref":"run-4131"}']

pinki resolve pnk_4f3a91 --satisfied --evidence https://example.org/reviews/91
pinki resolve pnk_4f3a91 --cancelled --reason "upstream schema was withdrawn"
pinki resolve pnk_4f3a91 --released  --by author

pinki assess  pnk_88de10 --violated --observer author --note "nothing shipped"

pinki ls    [--open|--overdue|--by X|--to Y|--json]
pinki show  pnk_4f3a91 [--json]

pinki a2a card              # emit the AgentCard extension block  -> stdout
pinki a2a task  pnk_4f3a91  # emit the Task metadata block        -> stdout
```

`ls` filters over computed state: `--open` is everything not in a terminal state
(`conditional` + `detached` + `overdue`); `--overdue` is the `overdue` subset alone.
Terminal states (`satisfied`, `cancelled`, `released`, `expired`) are excluded from
both and shown by `--all`.

`--meta` takes one JSON object and is accepted by `promise`, `resolve` and `assess`
(§1, §4). On the stdin form of `promise` it is refused rather than ignored — the
record you piped in carries its own `meta`, and guessing between two answers is how
provenance goes missing at exactly the seam this field exists to keep whole.

The two `a2a` verbs **only write JSON to stdout**. They do not call anything. You
pipe them into whatever A2A client you already run — that is the whole integration
story, and it is why there is no `a2a import`: getting data *out* of A2A is your
client's job, and `pinki promise` reads JSON on stdin like everything else.

The ledger is a file. `PINKI_LEDGER`, default `./.pinki/ledger.jsonl`. Every command
reads and writes JSON, and touches nothing but that file. There is no state anywhere
else, no daemon, and no network.

---

## 6. What pinki is not

Each of these is a real thing people will ask for. Each is a no, with a reason.

- **Not a scheduler.** pinki never runs your work, retries it, or nudges anyone.
  It has no clock of its own — it reads `until` and compares it to `now` when you
  ask. Your orchestrator schedules; pinki records.
- **Not an agent registry.** A2A AgentCards already are one. `by` and `to` are
  card identities and pinki resolves nothing.
- **Not an SLO engine.** [OpenSLO](https://openslo.com/) exists and is good at
  statistical targets over metric streams. A promise is a single obligation with a
  named debtor. Different object.
- **Not a dashboard.** `pinki ls --json` and your own tooling.
- **Not escrow, arbitration, or enforcement.** There is no penalty, no stake, no
  dispute resolution, and no adjudicator. A pinky promise never had one either.
- **Not the truth.** See below.

## 7. The honest limit

Two known problems, neither of which pinki solves.

**Commitment alignment.** Chopra & Singh formalized this in *Multiagent Commitment
Alignment* (AAMAS 2009, pp. 937–944,
[ACM DL](https://dl.acm.org/doi/10.5555/1558109.1558143)): a debtor and a creditor observing different histories can
legitimately infer different states for the same commitment, with neither party
mistaken. pinki does not fix that and cannot. What it offers is narrower and
actually achievable: because state is a fold over an append-only log keyed by
promise id, two parties' ledgers can be **joined**, and the diff *is* the
misalignment — visible, enumerable, and small enough to talk about. Making a
disagreement computable is not the same as resolving it, and pinki claims only the
first.

**`meta` is not part of the join key, and must not become one.** The join above is on
promise id and nothing else. Two parties holding the same promise will routinely
disagree about its `meta` — each side's provenance describes its own system, which is
the point of the field — and a join that keyed on it would report a misalignment where
there is none. Read the other side's `meta` if it helps you understand what you are
looking at; never diff it to decide whether the promises are the same promise.

**The intermediary problem.** Burgess's objection to any third party in an
obligation is structural, not fixable by good intentions: an intermediary that sits
in the path between two agents is *able* to distort what passes through it, and that
capability alone undermines its standing to certify anything — regardless of how it
behaves. pinki's answer is
positional, and it is enforced by an invariant rather than by a promise of good
behavior:

> **pinki makes no network calls.**

It is a log and a CLI. It is not a proxy, a server, a daemon, or a message bus, and
it is never in the request path between two agents. It cannot distort a message it
never carries. Observation is somebody else's job: whatever A2A client you already
have reads tasks and pipes JSON into `pinki promise` / `pinki assess`, and pinki
records what it is told. When it publishes an assessment, that assessment is signed
as one agent's opinion among the possible ones — which, per Burgess, is the only
honest thing any assessment can be.

---

## 8. Open questions

Genuinely open. Opinions wanted — see [CONTRIBUTING.md](../CONTRIBUTING.md).

1. **The extension URI needs a permanent home.** It is provisional today
   (§ [A2A-EXTENSION.md](A2A-EXTENSION.md)). Ideally it ends up under
   `a2a-protocol.org/extensions/`, which is a process, not a decision we can make.
2. **Moving a deadline.** Re-declare with a later `until`, or a distinct `amend`
   event, or a new promise superseding the old one? All three are defensible. v0
   ships none of them until someone hits the problem for real.
3. **Identity granularity.** `by` is an AgentCard identity, so an obligation binds
   the card's subject and outlives any one session. Is a card the right grain, or
   do fleets need something finer?
4. **Multi-party promises.** `to` is one creditor. Is a promise to a group one
   record or N?
5. **Signing.** Assessments are attributed but not cryptographically signed.
   Where's the line between useful and ceremony?
6. **Is `expired` worth keeping?** It is Singh-correct and it may be a distinction
   nobody in practice needs.
