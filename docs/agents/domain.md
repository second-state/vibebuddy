# Domain docs

How engineering skills should consume this repo's domain docs when exploring the codebase.

## Read these before exploring

- **`CONTEXT.md`** at the repo root; or
- **`CONTEXT-MAP.md`** at the repo root (if it exists): it points to each context's own `CONTEXT.md`. Read every one that is relevant to the topic.
- **`docs/adr/`**: read the ADRs relevant to the area you are about to touch. In a multi-context repo, also check `src/<context>/docs/adr/` for decisions scoped to that context.

If these files don't exist, **carry on silently**. Don't point out that they are missing, and don't suggest creating them up front. The `/domain-modeling` skill (reached via `/grill-with-docs` and `/improve-codebase-architecture`) creates them on demand once a term or decision is actually settled.

## File structure

Single-context repo (most repos, including this one):

```
/
├── CONTEXT.md
├── docs/adr/
│   ├── 0001-claude-adapter-does-not-parse-transcript.md
│   └── 0002-both-adapters-share-the-waiting-heuristic.md
└── daemon/  firmware/  protocol/
```

Multi-context repo (a `CONTEXT-MAP.md` exists at the root):

```
/
├── CONTEXT-MAP.md
├── docs/adr/                          ← system-wide decisions
└── src/
    ├── ordering/
    │   ├── CONTEXT.md
    │   └── docs/adr/                  ← decisions scoped to this context
    └── billing/
        ├── CONTEXT.md
        └── docs/adr/
```

## Use the glossary's vocabulary

When your output mentions a domain concept (issue titles, refactoring proposals, hypotheses, test names), use the term defined in `CONTEXT.md`. Don't drift into synonyms the glossary explicitly avoids.

If a concept you need isn't in the glossary yet, that is a signal in itself: either you are inventing language the project doesn't use (reconsider), or there is a real gap (note it and hand it to `/domain-modeling`).

## Flag conflicts with ADRs

When your output contradicts an existing ADR, say so explicitly instead of silently overriding it:

> _Contradicts ADR-0007 (event-sourced orders), but worth reopening because…_
