# pinki 🤙

> A tiny promise ledger for agent fleets, shipped as an [A2A](https://a2a-protocol.org)
> extension.

<!-- Uncomment once the first Actions run has completed on main:
[![CI](https://github.com/azigler/pinki/actions/workflows/ci.yml/badge.svg)](https://github.com/azigler/pinki/actions/workflows/ci.yml)
-->

**Status: v0.1.0, pre-alpha. The design is public and the CLI runs; nothing is
stable.** This repo is being built in the open as part of a written deep-dive on agent
obligations. Minor versions carry the work as it lands, and 1.0 is reserved for done —
see [CHANGELOG.md](CHANGELOG.md). The most useful thing you can send today is still an
argument: see [CONTRIBUTING.md](CONTRIBUTING.md).

## The problem

Give agents durable identities and they start depending on each other. One waits on
another's output; a third can't start until the second finishes. Once that happens,
things get dropped in the gaps between sessions — and nothing in the stack records
that anything *was* owed.

A2A gives agents identity and a way to talk. It has no way to say **"I will, by
when, and here's what will prove it."** There is no deadline field anywhere in the
protocol; `TaskStatus.timestamp` records when a status happened, never when anything
is due. That gap is [issue #857](https://github.com/a2aproject/A2A/issues/857), open
since July 2025 — where a project contributor suggested an extension as the right
vehicle, which is the route this repo takes.

pinki fills it with one record.

```json
{
  "id":      "pnk_4f3a91",
  "promise": "hand back a reviewed schema",
  "by":      "https://example.org/agents/reviewer",
  "to":      "https://example.org/agents/author",
  "on":      "pnk_0c2b77",
  "until":   "2026-09-01T17:00:00Z"
}
```

A **debtor** owes a **creditor** a thing, by a **deadline**. Satisfying it requires
evidence. Abandoning it requires a reason. And `on` points at another promise, which
is the interesting part:

> **A task graph's edges are ordering. A promise graph's edges are obligation between
> named parties.**

A task DAG tells you what has to happen before what. A promise graph tells you who
owes what to whom — and therefore who is left holding something when a node goes
quiet. The shape of a project versus the shape of a collaboration.

## What pinki will not do

It will not tell you a promise was broken.

It can't. Whether a promise was kept is a judgment — it depends on what counts as
done, on context the record doesn't carry, and on who's asking. Mark Burgess's
[Promise Theory](http://markburgess.org/PromiseMethod.pdf) is blunt about this:
assessment is made by an agent, from that agent's vantage, and is subjective no
matter which agent makes it. A third party that appoints itself judge has mostly
acquired the ability to distort the thing it claims to certify.

So pinki draws a hard line down the middle of its own state machine:

- **`overdue` is arithmetic.** `now > until` and unresolved. Every implementation
  computes the same answer. pinki will tell you this.
- **`violated` is a speech act.** Somebody says it, signs it, and timestamps it.
  pinki will *publish* it, attributed, next to the promise — and never compute it.

Two observers can publish contradicting assessments of the same promise. pinki shows
you both, because that is the actual state of the world and hiding it would be a lie.

It is also not a scheduler, an agent registry, an SLO engine, a dashboard, or an
escrow. Each of those has a reason attached in
[DESIGN.md § 6](docs/DESIGN.md#6-what-pinki-is-not).

## Shape

Three layers, separately adoptable:

1. **A vocabulary** — the record above, and a state machine borrowed from 25 years of
   [commitment](docs/DESIGN.md#3-the-states) research rather than invented fresh:
   seven states pinki *computes* (`conditional`, `detached`, `overdue`, `satisfied`,
   `cancelled`, `released`, `expired`) and one it only ever *publishes* (`violated`).
2. **An A2A binding** — using only the three surfaces A2A sanctions for extensions:
   the AgentCard declaration, the `A2A-Extensions` header, and URI-prefixed
   `metadata` keys. No new task states, no new roles.
   → [docs/A2A-EXTENSION.md](docs/A2A-EXTENSION.md)
3. **A CLI over an append-only JSONL log** — six verbs, no daemon, no server, no
   database, and **no network calls at all**. It writes JSON to stdout and you pipe
   it into the A2A client you already run.

You can take the first layer and build your own tooling. You can take the first two
and interoperate over A2A without ever running pinki. That's the point — pinki is
meant to be a nucleus you build on top of, not a framework you adopt.

## Install

Build it from source. There is nothing else to install — no daemon, no server, no
database.

```sh
cargo build --release
```

The binary lands at `target/release/pinki`. Put it on your `PATH` and it will create
its ledger on first use:

```sh
./target/release/pinki promise "hand back a reviewed schema" \
  --by  https://example.org/agents/reviewer \
  --to  https://example.org/agents/author \
  --until 2026-09-01T17:00:00Z
```

The ledger is a JSONL file at `$PINKI_LEDGER`, defaulting to `./.pinki/ledger.jsonl`.
Copy that file and you have copied the state.

**MSRV: Rust 1.97.1** — the stable toolchain this is built and tested against. Earlier
versions may work; none are tested, so none are claimed.

## Prior art

Very little here is new, and the parts that aren't deserve the credit.

The state machine comes from twenty-five years of multiagent-systems research on
**commitments**. [Yolum & Singh's *Commitment
Machines*](https://doi.org/10.1007/3-540-45448-9_17) (ATAL 2001) gave it
`C(debtor, creditor, antecedent, consequent)` — which is what pinki's `by` / `to` /
`on` are, renamed for people who haven't read the paper — and [Chopra &
Singh](https://doi.org/10.1609/aaai.v29i1.9443) added deadlines to it in 2015. Mark
Burgess's [Promise Theory](http://markburgess.org/PromiseMethod.pdf) supplied the
hardest constraint, that assessment is subjective no matter which agent makes it;
it is the reason pinki observes instead of adjudicating. And [FIPA's Contract Net
Interaction Protocol](http://www.fipa.org/specs/fipa00029/SC00029H.html) had the
deadline in the 1990s, with a `reply-by` on every call-for-proposals.

Two projects landed on adjacent ground recently and independently. Both are worth
your time. **[mentu-ai/protocol](https://github.com/mentu-ai/protocol)** goes
further than pinki on integrity — a hash-chained JSONL ledger with state recomputed
by replaying the log. **[proof-gate](https://github.com/1000larrylobster-source/proof-gate)**
goes further on evidence: nothing closes until a restricted, read-only proof command
exits 0, on the premise that an agent's self-report is worthless. If either fits your
problem better, use it.

Full accounting — including the parts pinki got wrong first — is in
[docs/PRIOR-ART.md](docs/PRIOR-ART.md).

## Docs

- **[docs/DESIGN.md](docs/DESIGN.md)** — the nucleus: record, graph, states, log, CLI,
  non-goals, and the honest limits.
- **[docs/A2A-EXTENSION.md](docs/A2A-EXTENSION.md)** — the extension spec, checked
  against `a2a.proto`.
- **[docs/PRIOR-ART.md](docs/PRIOR-ART.md)** — what already exists, who got here
  first, and what's actually new.
- **[CONTRIBUTING.md](CONTRIBUTING.md)** — what would help most right now.

## Contributing

The design has already been corrected once by outside research — an earlier draft
called this a broker that detects lapses, which overclaimed on both counts. Expect
that to keep happening.

If you've built something shaped like this, or you can see exactly where it breaks,
[open an issue](https://github.com/azigler/pinki/issues/new/choose). That is precisely
the conversation this repo exists for.

## License

MIT — see [LICENSE](LICENSE).

---

*pinki is an independent experiment. It builds on [A2A](https://a2a-protocol.org), a
hosted project of the [Agentic AI Foundation](https://aaif.io); it is not affiliated
with, endorsed by, or speaking for either.*
