# mnema documentation map

[简体中文](README.md) | English

Each normative fact has one owning document. Other documents link to that
owner instead of copying rules into a second source of truth.

| Document | Owns | Does not own |
|---|---|---|
| [README](../README.md) | Product entry, shortest work loop, common commands | Complete domain or module specification |
| [ARCHITECTURE](ARCHITECTURE.md) | Target modules, boundaries, invariants, load-bearing decisions | Command tutorial or diagnostic catalog |
| [CORPUS](CORPUS.md) | Entry/strand semantics, virtual Journal root, relations, scope, lifecycle, delegation, federation | Source layout or vendor launch commands |
| [DIAGNOSTICS](DIAGNOSTICS.md) | Errors, warnings, notices, Doctor, and code registry | Task verdicts or scheduling policy |
| [TESTING](TESTING.md) | Test layers, invariants, and isolation policy | Live suite inventory |
| [TEST-CATALOG](TEST-CATALOG.md) | Current/planned suites, lanes, timeouts, and evidence | Testing philosophy |
| [agent-roster](agent-roster.md) | Maintainer-tested model invocation notes | Core semantics |
| [MIGRATION-v2-to-v3](MIGRATION-v2-to-v3.md) | Current v2→v3 migration procedure | Repeated domain identity rules |
| [MIGRATION-v1-to-v2](MIGRATION-v1-to-v2.md) | Retired historical migration record | Current v3 onboarding |

The detailed documents are currently Chinese-first and are being translated in
pairs. CLI syntax, public JSON, and `mnema explain <topic|CODE>` are the current
English machine/consumer contracts.

## One-sentence architecture

mnema is a semantic-topology substrate for multi-agent collaboration: Journal
is a virtual projection root, any strand can become a local root, the default
view is the root's downward closure, and parent/refs/depends-on remain explicit
unexpanded exits; Core records semantic facts without interpreting process or
scheduling facts.

## Maintenance rules

1. Assign every new concept to one owning document.
2. Replace obsolete design text instead of keeping current and superseded rules side by side.
3. Mark historical migrations and keep them out of the current onboarding path.
4. Help examples are parsed; public JSON fields are additive-only.
5. Register suites in TEST-CATALOG; change TESTING only when policy changes.
