---
status: accepted
---

# Lines are written in advance, never when they are spoken

To make the buddy feel like a person, each Character has a persona and many lines per occasion. We considered having a language model write each line the moment an announcement fires and synthesizing it on the spot. We decided instead that every line is written and synthesized in advance, shipped in the Character pack, and picked at random on the box.

## Considered options

- **Generate at announcement time.** An LLM plus TTS adds one to three seconds before the box speaks, and needs-input is only worth anything if it is immediate. The UART bridge carries about 5 KB/s, so a two-second line would take longer to send than to say (ADR-0003 already ruled out streaming audio for this reason). It goes silent without a network, a key or credit, so the fixed lines would have to stay as a fallback anyway. Nobody gets to hear a line before the user does. And to say anything specific it would need session or project names, which goes against the rule that the persona never sees what the agents are working on.
- **A mix: a pool, plus live lines for a few occasions.** Possible later; nothing in this design blocks it.

## Consequences

Variety comes from pool size and from special occasions (first done, milestone, late night, daily greeting), not from live context. Lines are drafted with a local LLM during development and reviewed by ear before they are committed; the App and the daemon never call a language model.
