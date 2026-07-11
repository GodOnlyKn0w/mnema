# CLI behavior tests

This directory is the black-box behavior layer described in
`docs/TESTING.md`. It intentionally starts with a manifest rather than recorded
snapshots: normalization and isolation must be stable before golden output is
committed.

`scenarios.json` describes user-visible journeys. Each scenario will run in a
fresh temporary project and must name which dynamic fields it permits. Setup
commands prepare state but are not themselves snapshot assertions.

Rules for future fixtures:

- never use the repository `.mnema/`;
- prefer `--format json` for machine contracts and text output for genuinely
  human-facing contracts;
- keep one behavioral reason per scenario;
- do not normalize wording, field names, exit codes, or diagnostics;
- keep platform variants explicit;
- do not record snapshots automatically in CI.

The eventual record/replay integration may use `rere.py`, but the scenario
driver owns isolation and normalization. This prevents the snapshot tool from
becoming an implicit source of mnema semantics.

