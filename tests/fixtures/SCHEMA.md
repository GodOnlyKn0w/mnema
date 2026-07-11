# Historical fixture contract

Directories under `v2/` are immutable compatibility evidence. Each version
contains exact historical journal bytes, a fixed journal identity, an expected
identity/projection contract, and a short purpose. Once merged, never update a
version in place: add a new directory and catalog entry.

The gate verifies source SHA-256 before migration, independently recomputes the
migration-id domain hash, pins deterministic v3 identities and target bytes,
parses raw v3 records without the production v3→Event adapter, and finally
compares public pre/post-cutover projections.
