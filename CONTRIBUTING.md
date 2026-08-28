# Contributing to pinki

pinki is pre-alpha and **the design is the work right now**. The most valuable
thing you can send is not a pull request. It is an argument.

## What we want most, in order

1. **"This can't work, and here's why."** If you have run an obligation or
   commitment system in anger and you can see where this one breaks, that is the
   single most useful issue you can open. Prior art talked us out of one bad idea
   already; we would rather lose the next one to you than to production.
2. **"I built something shaped like this."** Point at it. Tell us what you learned
   and what you would not do again.
3. **An answer to one of the open questions** in
   [docs/DESIGN.md § 8](docs/DESIGN.md#8-open-questions).
4. **A worked example that does not fit the record.** A real promise between real
   agents that `docs/DESIGN.md` cannot express is a bug report against the design.
5. Code — once the design settles. See below.

## The bar for anything new

pinki is trying to be a nucleus: small enough that people build *on top of* it
rather than around it. So every addition is measured against one question:

> **Does this belong in the smallest thing that still works — or does it belong in
> the layer above?**

Most good ideas belong in the layer above. That is not a rejection of the idea; it
is the project working. If pinki grows a scheduler, a dashboard, and a policy
engine, it stops being adoptable and becomes another framework you have to buy into.

Before proposing a feature, read
[docs/DESIGN.md § 6, "What pinki is not"](docs/DESIGN.md#6-what-pinki-is-not).
Those `no`s each have a reason attached. If you think a reason is wrong, argue with
the reason — that is a much more interesting issue than the feature request.

Concretely, a change is likely to land if it:

- **removes** something, or
- makes an existing thing more precisely specified, or
- fixes a place where two implementers would build incompatible things, or
- corrects a factual claim about A2A or about the prior art.

A change is likely to be pushed up a layer if it adds a field, a verb, a state, or
a configuration knob.

## The three layers

pinki is a vocabulary, an A2A binding, and a CLI
([docs/DESIGN.md](docs/DESIGN.md)). You can contribute to one without touching the
others, and it helps to say which you mean:

- **Vocabulary** — the record, the states, the semantics. Highest stakes: everything
  else depends on it, and changes here are breaking by definition.
- **A2A binding** — how the vocabulary rides the protocol. Must stay inside the
  three surfaces A2A sanctions for extensions. If a proposal needs a fourth, it is
  not an extension anymore and we need to know that early.
- **CLI** — the reference implementation. Lowest stakes, easiest to change.

An implementation of the vocabulary in another language, or as a library rather than
a CLI, is a very welcome thing and does not need to live in this repo. Tell us and
we will link it.

## Issues before pull requests

While the design is moving, please **open an issue before writing code**. A PR
against a schema that changes next week is a waste of your evening, and we would
rather spend your goodwill on the argument than on a rebase.

Exceptions, always welcome unannounced: typos, broken links, factual corrections,
and anything in the "removes something" category.

## Working agreements

- **Be concrete.** "This won't scale" is hard to act on. "At 10k promises the fold
  is O(n²) because…" is a gift.
- **Cite things.** This design leans on 30 years of published work
  ([docs/DESIGN.md § 3, § 7](docs/DESIGN.md#3-the-states)) and it should keep doing
  that. If you know a paper that already answered one of the open questions, that is
  a complete contribution on its own.
- **Disagreement is the point.** The observer framing in this design exists because
  the first draft overclaimed and the research said so. Expect your argument to be
  taken seriously, and expect to be argued with.
- **No drive-by generated content.** Use whatever tools you like to write — but
  read what you send, check that it is true, and be able to defend it. An issue that
  is clearly unread by its own author will be closed politely and without discussion.

## Pull request mechanics

- One idea per PR. Small is easier to say yes to.
- Update `docs/DESIGN.md` in the same PR if you change behavior. The doc is the
  spec; if they disagree, the doc wins and the code is the bug.
- Explain *why* in the description, not just *what*. The diff shows the what.
- Contributions are accepted under the [MIT License](LICENSE), same as the project.
  There is no CLA and no sign-off ceremony.

## Code of conduct

Participation is governed by [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md).

## A note on the protocol

pinki is an independent, unaffiliated experiment. It is built on
[A2A](https://a2a-protocol.org), which is a hosted project of the
[Agentic AI Foundation](https://aaif.io) — pinki is not, and nothing here is
endorsed by or speaks for either. If this design is ever worth proposing upstream,
that happens through A2A's own extension process, in the open, on their terms.
