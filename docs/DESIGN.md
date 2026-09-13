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
  business. pinki holds the obligation and nothing else.

### The id is opaque, and that cuts both ways

`id` says *opaque* in the table above, and pinki means it about its own ids too.
There is exactly one id field, and it holds one of two things:

- **A minted id**, when you do not say otherwise: `pnk_` plus exactly six lowercase
  hex digits. Short and typeable, and minted by asking the ledger which ids are
  already spoken for rather than by trusting the size of the space.
- **An id you brought with you**, via `pinki promise --id` (or `"id"` in the record
  on stdin): any non-blank string with no whitespace, no control characters and no
  invisible characters, up to 128 characters.

The permissiveness is the point. Anyone arriving with an existing obligation ledger
has an id space that other systems already reference, and an `--id` that refuses it
turns adoption into a permanent side table mapping their ids to ours — a second copy
of the identity of every promise, maintained forever, for no gain. The join in § 7
wants the same thing from the other direction: it is keyed by promise id, so two
parties who did not both mint here can only key it if pinki will hold the key they
share.

Four rules, and they are the only ones:

- **No whitespace, no control characters, and not blank.** The id is a column in
  `pinki ls`, the first field of the tab-separated line every mutating verb prints,
  and the key two ledgers are joined on. Those are the uses; these are the rules that
  keep them working. Nothing else about the shape is pinki's business.
- **No invisible characters.** A character with the Unicode
  **`Default_Ignorable_Code_Point`** property — zero-width spaces and joiners, the
  bidi controls, variation selectors, tag characters, soft hyphen, the Hangul fillers
  — is refused. This is its own rule because neither of the two above catches it:
  U+200B ZERO WIDTH SPACE is `White_Space=No` despite the name, and none of these are
  control characters. Without it, two ids that are **not equal** can be
  **indistinguishable** — in `pinki ls`, in a terminal, in a code review, and to
  whoever is deciding whether the join in § 7 matched. An identifier whose equality
  the eye cannot check is not a handle. Non-ASCII is otherwise entirely fine: this is
  a rule about invisibility, not about script. (The set is Unicode 16.0.0's, carried
  as a 17-range table in `src/id.rs`; a later Unicode version that adds to the
  property needs that table updated.)
- **At most 128 characters.** A bound rather than a taste — a UUID is 36, a URN
  comfortably under 100 — chosen so a ledger line stays something a human reads with
  `less`. It is not a claim that 129 would be meaningless; it is a refusal to let the
  field grow without anyone deciding it should.
- **Unique within the ledger.** Declaring an id the ledger already declares is
  refused, minted or supplied alike: that is § 4's first-declaration-wins guard, and
  it does not care which space the id came from. The six-hex space never had a
  collision check worth the name; this is it.

`pnk_` is reserved for the ids pinki mints, so a supplied id wearing that prefix
without being six lowercase hex digits is refused. That reservation is small and it
buys something specific: "was this minted here?" stays an answerable question from
the id alone, which is what lets an unknown-id error tell a typo (`pnk_bogus` could
never have been declared) apart from a plain lookup miss (`pr-2026…` simply is not in
this file). An adopter loses nothing by it — an id space that already begins `pnk_`
is pinki's own.

There is deliberately **no `external_id` field**. Two id fields is two ways to name
one promise, and every reader then has to decide which one the `on` edge, the join,
and the A2A `metadata` block key on. One field, opaque, is the smaller thing that
works.

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

Append-only JSONL. Four event types. State is a fold over the log.

```jsonl
{"ts":"2026-08-28T20:14:03Z","type":"promise","id":"pnk_4f3a91","promise":"hand back a reviewed schema","by":"…/reviewer","to":"…/author","on":"pnk_0c2b77","until":"2026-09-01T17:00:00Z"}
{"ts":"2026-09-01T16:40:00Z","type":"amend","promise":"pnk_4f3a91","by":"…/reviewer","until":"2026-09-01T18:00:00Z","reason":"the draft schema landed late"}
{"ts":"2026-09-01T16:02:11Z","type":"resolve","promise":"pnk_4f3a91","as":"satisfied","by":"…/reviewer","evidence":["https://example.org/reviews/91"]}
{"ts":"2026-09-02T09:00:00Z","type":"assess","promise":"pnk_88de10","state":"violated","observer":"…/author","note":"nothing shipped, no reason given"}
```

**`promise`** — speaks a promise into existence.

**`amend`** — moves a promise's deadline, and nothing else. This is §8's second open
question, settled: see §8.2 for why this shape and not the other two.

The fold takes the **latest** amend's `until` as the current horizon — the one
`overdue` is computed against, the one `ls` prints, the one that rides the A2A
`metadata` block — and keeps **every earlier horizon**, which `show` renders in order.
"This deadline moved twice, says who, when, and why" is one read of the record rather
than an archaeology problem.

Four properties make this small enough to belong here:

- **The declaration is never rewritten.** First declaration still wins; an amend is a
  new line beside it, not an edit to it. A re-declaration is the *silent* version of
  this and stays refused.
- **A reader that skips unknown event types degrades to first-wins** — the horizon the
  promise was born with — rather than to something broken. **v0.1.0 is not such a
  reader**: its parser refuses a ledger containing an `amend` line outright, so a
  ledger with amends in it needs this version or later. That refusal is the reason this
  version is the first *forward-tolerant* one: an event whose `type` it does not know
  is read, warned about on stderr with its line number, and ignored by the fold, so the
  next event type this format grows will not cost anyone the rest of their file. A line
  that is not an event at all is still fatal.
- **Only the debtor may amend.** A promise is the debtor's own declaration, so moving
  its horizon is a new act by the same party. A *creditor* or third party who thinks a
  deadline should move is making an assessment, which is a different speech act and
  already has a verb. Like every rule here, this is enforced where input arrives and
  not in the fold: pinki cannot authenticate anyone, and a joined ledger may carry an
  amend written by something looser. The fold reads it and attributes it; the CLI
  refuses to write it.
- **Deadlines move both ways.** Bringing one forward is as legitimate as pushing it
  out, and pinki has no opinion about the direction — only about the move being
  visible.

A resolved promise cannot be amended: the resolution ended it and there is no horizon
left to move. And "latest" means last in the log, not the largest `ts` — the fold never
compares event timestamps, because a ledger whose lines are out of order is not one
whose clock can be trusted to sort them.

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

**Unknown event types.** A reader that meets a `type` it does not know keeps the line,
says so on stderr with its line number, and folds without it. That is not the same
tolerance as skipping a line it could not parse at all — that one stays fatal, because
§4's own point is that quietly dropping a line you cannot read is truncation told one
line at a time. The distinction is "an event I have not heard of" versus "not an
event."

Because state is a fold, the log is the whole system. Copy the file and you have
copied the state. Concatenate two ledgers and you have a joined view. Truncate it
and you have lied to yourself, which is a property, not a feature.

---

## 5. The CLI

Seven verbs. It should feel like a small issue tracker: push JSON in, pull JSON out,
no daemon, no server, no database.

```
pinki promise "hand back a reviewed schema" \
      --by reviewer --to author --until 2026-09-01T17:00Z [--on pnk_0c2b77] [--id ID]

pinki amend   pnk_4f3a91 --until 2026-09-01T18:00Z [--reason "the draft landed late"]

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
both and shown by `--all`. Both `ls` and `show` print the **current** horizon in
`until`; `show` additionally lists every horizon the promise has had, declaration
first, once there is more than one.

`amend` defaults `--by` to the debtor — the only party §8.2 admits — and refuses an
explicit one that disagrees. `--reason` is encouraged, not required: an escalation
ladder amends on a clock and has one reason for every rung.

`promise` mints an id unless `--id` gives it one, and an id you give it is opaque —
§ 1's id policy, enforced where the input arrives. Every other verb takes whatever id
the ledger holds; none of them re-checks its shape, because an id that could be
declared has to be resolvable.

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

That join is only as available as the key is. It works between two parties who both
run pinki because both ledgers carry ids from the same space; it works between a
pinki ledger and a party who never heard of pinki **only if pinki will hold their
id** — which is why § 1's id is opaque and `--id` takes whatever you already call the
thing. A tool that insisted on minting the key would have offered a join that
requires the disagreement to be between two copies of itself.

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

Genuinely open, except where a number says otherwise — a settled one keeps its number
so the answer stays where the question was. Opinions wanted — see
[CONTRIBUTING.md](../CONTRIBUTING.md).

1. **The extension URI needs a permanent home.** It is provisional today
   (§ [A2A-EXTENSION.md](A2A-EXTENSION.md)). Ideally it ends up under
   `a2a-protocol.org/extensions/`, which is a process, not a decision we can make.
2. **Moving a deadline — SETTLED: the `amend` event** (§4), on the evidence in
   [issue #9](https://github.com/azigler/pinki/issues/9). v0 shipped none of the three
   candidates "until someone hits the problem for real"; someone did, with
   measurements, so here is the answer and the reasoning.

   The case: a fleet's watchdog escalates a lapsed promise by re-declaring **the same
   id** on a short clock — `until = T`, then `T+15m`, then `T+30m` — because minting a
   fresh id per nudge leaves a permanent phantom promise behind for every rung. Against
   the v0.1.0 fold that ladder reports `until = T` forever, and an external observer
   scored a live promise overdue **32 minutes before the system that owned it did**. An
   observer that disagrees with the owner about *when* a deadline fell is publishing a
   different fact, not a second opinion on the same one.

   - **Not re-declare-with-a-later-`until`.** The first-declaration-wins guard exists
     precisely because a silent move is the problem; keeping it and adding an explicit
     event keeps both halves — silent moves stay impossible, honest ones become
     visible.
   - **Not supersede-with-lineage.** It re-imposes the cost the ladder was built to
     avoid: a routine three-step escalation manufactures three dead records plus one
     live one, and a fleet whose escalation runs through its promise store would litter
     its own ledger at escalation frequency. Any graph rendering then shows four nodes
     where one obligation exists.
   - **Amend, debtor-only.** Moving a horizon is a new act by the same party who spoke
     the promise. The creditor sees the move — attributed, timestamped, with every
     prior horizon still in the record — and can `assess` it, never edit it. An
     observer's view of someone else's deadline is an assessment; that is the same
     line §3 draws between arithmetic and judgment, and it is why an observer amend is
     deliberately not allowed in this version.

   What it buys is the property that motivated the report: with amends in the log, an
   outside observer folds the **same current horizon** as the system that owns the
   promise. The divergence disappears structurally instead of being reconciled after
   the fact. What stays open underneath it is §8.3 — whether a card is the right grain
   for `by` — since "the debtor" is only as sharp as the identity that names it.
3. **Identity granularity.** `by` is an AgentCard identity, so an obligation binds
   the card's subject and outlives any one session. Is a card the right grain, or
   do fleets need something finer?
4. **Multi-party promises.** `to` is one creditor. Is a promise to a group one
   record or N?
5. **Signing.** Assessments are attributed but not cryptographically signed.
   Where's the line between useful and ceremony?
6. **Is `expired` worth keeping?** It is Singh-correct and it may be a distinction
   nobody in practice needs.
