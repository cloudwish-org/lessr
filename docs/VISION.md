# Vision

Coding agents pay rent on every token that enters their context. Lessr is the layer that collects less rent.

## The problem, precisely

An agent session re-sends its whole history on every turn. Tool output (tests, git, file reads) enters that history and is paid for again on each later turn at cache-read price; when the provider cache breaks, it is paid for again at write price. A 30k-token file read at turn 10 of a 60-turn session costs about six times its face value. Almost nothing in the toolchain shows this, and the popular remedies (terse personas, "write less code" prompts) attack the small slice, output tokens, while measuring far below their claims.

## What Lessr is

A local, no-LLM layer between agents and model APIs that:

1. keeps the request prefix byte-stable so the provider cache serves it,
2. shrinks tool output before it enters history, without ever hiding an error,
3. returns unchanged files as a hash and changed files as a diff,
4. traps the reads that should never enter history raw,
5. records every provider usage field and prints an exact receipt.

## What Lessr is not

- Not a model, not a router, not a prompt. It never changes what the model is asked to do.
- Not a cloud proxy. It runs on the developer's machine; a network hop would break the 10 ms budget.
- Not a persona. Nothing in Lessr tells the model to be brief.

## Principles for anyone extending it

- **Only save what you can count.** A mechanism ships when its saving is an accounting identity: bytes removed, or cache-read versus cache-write tokens from the provider's own usage fields. Behavioural savings are someone else's product.
- **Loop safety over savings.** A mechanism that makes the agent retry is a loss, however many tokens it strips. See LOOP_SAFETY.md.
- **Local first.** Prompts never leave the machine. The engine works offline forever.
- **Invisible.** Under 10 ms, streaming untouched, zero configuration, self-updating.
- **Honest numbers.** Every release publishes the paired benchmark, red rows included.

## Where it goes

The engine becomes a dependency of agent harnesses rather than a proxy users install: `lessr-core` and `lessr-gate` are libraries first, a binary second. When a harness ships Lessr inside, the rent is collected before anyone has to think about it.
