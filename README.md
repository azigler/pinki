# pinki 🤙

> A tiny promise broker for agent fleets, built on [A2A](https://a2a-protocol.org).

**Status: pre-alpha — design in the open.** This repo is being bootstrapped as part
of a written deep-dive on agent obligations (coming soon). Nothing here is stable yet.

## The idea

Agents with durable identities make **promises** — commitments that outlive the
session that made them. A promise carries:

- a **deadline**, so a lapse is computable
- **evidence** at resolution, so "done" points at an artifact
- a **reason** at abandonment, so dropping it is a recorded act

A2A's AgentCard gives agent identity a durable, signed home, and its extension
system gives typed metadata a place to ride. What's missing everywhere: when an
observer detects a lapse, that verdict lands on a second ledger — and nothing in
any protocol names who reconciles the two. pinki is that missing third party: a
**loose promise broker** that holds the join.

Planned shape: deliberately small — likely a CLI over an append-only JSONL ledger,
speaking A2A-shaped records. Design notes, prior-art research, and the first cut
land here as they happen.

## Contributing

Contributor guidelines, a community guide, and the first issues arrive with the
initial design (days, not months). Watch the repo if this scratches an itch you
have — and if you've built something shaped like this, or know exactly why it
can't work, open an issue: that's precisely the conversation this repo is for.

## License

MIT — see [LICENSE](LICENSE).
