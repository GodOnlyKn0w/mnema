# Historical compatibility tests

This directory is the ownership boundary for evidence from retired journal schemas. Compatibility claims preserve old bytes and their promised migration/projection results; they do not define preferred APIs for new code.

Current locations are retained until fixtures and Rust modules can move without obscuring history:

- `tests/v2_v3_compat.rs` and `tests/fixtures/v2/` — frozen v2 bytes and v3 migration identity;
- `src/tests/manage_tests.rs` tests prefixed `cutover_v2_` — retired v1→v2 translator/certificate behavior;
- tests prefixed `legacy_`, `old_`, or `v2_` in unit modules — historical readers and effect translation.

Rules:

1. Never copy a compatibility helper or legacy event shape into current semantic tests.
2. Never rewrite a published fixture; add a versioned fixture instead.
3. Current topology contracts live in `tests/behavior_harness.rs`, `src/tests/query_tests.rs`, and `tests/recursive/`.
4. The Test Catalog registers historical compatibility separately from current semantic suites.
