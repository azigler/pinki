# Prior art

Very little about pinki is new, and the parts that aren't should be credited
properly. This document exists so you can decide for yourself whether the narrow
thing pinki claims is actually narrow, and so nobody has to reverse-engineer where
the ideas came from.

Every link below was checked on 2026-08-28.

## The lineage we borrow from

The state machine is not ours. Multiagent-systems research worked out the semantics
of commitments across the 2000s, and pinki is a consumer of that work.

- **Castelfranchi**, *Commitments: From Individual Intentions to Groups and
  Organizations* (ICMAS-95).
  [PDF](https://cdn.aaai.org/ICMAS/1995/ICMAS95-006.pdf) — social commitments are
  *external* to any single agent. The argument for holding obligations in a shared
  artifact rather than inside each agent's head, made thirty years ago.
- **Singh**, *A Social Semantics for Agent Communication Languages* (IJCAI-99
  workshop).
  [PDF](https://www.csc2.ncsu.edu/faculty/mpsingh/papers/mas/ijcai-99-acl.pdf) —
  ground agent communication in public commitments rather than private beliefs,
  because heterogeneous agents' beliefs "cannot be uniformly determined."
- **Yolum & Singh**, *Commitment Machines* (ATAL 2001, LNCS 2333, pp. 235–247).
  [doi:10.1007/3-540-45448-9_17](https://doi.org/10.1007/3-540-45448-9_17) —
  `C(debtor, creditor, antecedent, consequent)`, and protocols expressed as the
  lifecycle of commitments instead of as legal message orderings. pinki's
  `by` / `to` / `on` are this, renamed for people who have not read the paper.
- **Chopra & Singh**, *Cupid: Commitments in Relational Algebra* (AAAI 2015).
  [doi:10.1609/aaai.v29i1.9443](https://doi.org/10.1609/aaai.v29i1.9443) —
  commitments **with deadlines**, and "which ones are violated?" compiled into a
  relational query. The closest ancestor of what `pinki ls --overdue` does.
- **Chopra & Singh**, *Multiagent Commitment Alignment* (AAMAS 2009, pp. 937–944).
  [ACM DL](https://dl.acm.org/doi/10.5555/1558109.1558143) — debtor and creditor,
  observing asynchronously, can infer *different states for the same commitment
  with neither being wrong*. pinki does not solve this. See
  [DESIGN.md § 7](DESIGN.md#7-the-honest-limit).
- **Burgess**, *Promise Theory*
  ([method paper](http://markburgess.org/PromiseMethod.pdf); see also
  [*Cooperation in Human and Machine Agents*](https://arxiv.org/abs/2604.10505),
  2026) — no agent can make a promise on another's behalf, and assessment of whether
  a promise was kept is subjective regardless of which agent makes it. This is the
  source of pinki's hardest constraint, and of the redesign described at the bottom
  of this page.

## Contemporary neighbours

Two projects landed on adjacent ground recently and independently. Both are worth
your time, and if either fits your problem better, use it.

**[mentu-ai/protocol](https://github.com/mentu-ai/protocol)** (MIT) — a
specification for an append-only, **hash-chained** ledger of agent commitments,
whose governing rule is *"observations become obligations; obligations require
evidence."* One record type (`EpistemicSignal`) in JSON Lines, nine operations
(`capture`, `commit`, `claim`, `release`, `close`, `submit`, `approve`, `reopen`,
`annotate`), and state recomputed by replaying the log rather than stored. It goes
considerably further than pinki does on integrity (SHA-256 chaining with a precisely
specified canonicalisation) and on derived trust scoring.

*Overlap:* the ledger shape, evidence-gated closure, and state-as-a-fold. pinki
arrived at all three independently and would have been better off reading this first.
*Difference:* it mints its own actor and workspace identities and is not an A2A
extension — there is no agent-to-agent protocol binding.

**[proof-gate](https://github.com/1000larrylobster-source/proof-gate)** (MIT) — a
dependency-free Python utility whose premise is that an agent's self-report is
worthless: *"'Done' is a green check, never a sentence."* Every commitment carries a
**proof command** — a read-only shell command, restricted to a non-mutating
allow-list so an agent cannot author a proof that manufactures its own artifact —
plus a due date, swept every morning. Items end **done** (proof exited 0) or
**dropped** with a recorded reason. Repeated failures increment a visible miss count.

*Overlap:* deadline plus evidence-on-close plus reason-on-abandon is exactly pinki's
resolution model. *Difference:* it is single-owner and single-fleet by design, with
paths hard-coded for your own layout, and it verifies by *executing* proofs — a
stronger guarantee than pinki's, and one that requires the authority to run commands
that pinki deliberately does not take.

An honest note on that last point: proof-gate's executable proof is a better answer
to "is it really done?" than pinki's inert reference. pinki's evidence merely points;
it does not check. The trade is that pinki can hold a promise between two
organisations that would never let each other run shell commands.

## Adjacent, but a different object

- **[FIPA Contract Net Interaction Protocol](http://www.fipa.org/specs/fipa00029/SC00029H.html)**
  (SC00029) — had the deadline in the 1990s: a call-for-proposals carries a
  `reply-by`, late proposals are auto-rejected, and an accepted proposal means the
  participant "acquires a commitment," ending in `inform-done` or `failure`. A
  negotiation protocol rather than an obligation ledger — commitments are formed by
  the exchange and do not outlive it.
- **[OpenSLO](https://openslo.com/)** — the mature "objective as YAML, validated in
  CI" pattern. But an SLO is a statistical target over a metric stream; a promise is
  one obligation with a named debtor, a deadline, and an artifact. Different object,
  frequently confused.
- **Durable agent runtimes** (checkpointing, pause/resume, replayable history) —
  increasingly common, and orthogonal. Durability means a *process* can survive
  interruption. It says nothing about whether anyone still owes anything, to whom, or
  by when. **Durability is not obligation**, and having one does not give you the
  other.

## What is actually new here

A narrow claim: **binding obligation semantics to a cross-organisational agent
identity.**

The ledgers above assume one owner's fleet and mint their own actor IDs. A2A supplies
durable, discoverable, signed identity in AgentCards, and a sanctioned extension slot
— but no vocabulary for "I will, by when, and here's what will prove it." A2A has no
deadline field anywhere, which is
[issue #857](https://github.com/a2aproject/A2A/issues/857), open since July 2025 and
where a project contributor suggested an extension as the right vehicle.

That join is the only part of pinki that isn't borrowed. Everything else on this page
got there first.

## What we already got wrong

Worth recording, since the repo is meant to be a public account of the work and not
just its conclusion.

The first draft of this project described itself as a **broker that detects lapses**.
Both halves were wrong, and the prior-art scan above is what caught it:

- *"Broker"* implied a party in the middle with authority. Burgess's objection is
  structural rather than a suspicion of bad intent: anything sitting in the path
  between two agents is *able* to distort what passes through, and that capability
  alone costs it the standing to certify. pinki answered by removing itself from the
  path entirely — it makes no network calls at all.
- *"Detects lapses"* implied a computed verdict. Whether a promise was broken is a
  judgment made from somebody's vantage. pinki now splits `overdue` (arithmetic,
  computed) from `violated` (a speech act, published and attributed, never computed).

The reframe — from adjudicator to **observer that publishes assessments** — turned
the two objections into the design's most interesting property. If you spot the next
one, [open an issue](https://github.com/azigler/pinki/issues/new/choose).
