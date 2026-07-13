# Contributing to mnema

Thank you for helping improve mnema.

## Before changing code

- Open an issue for changes to journal identity, lifecycle, scope, JSON fields,
  or CLI grammar. These are compatibility-bearing design decisions.
- Keep entry text and semantic interpretation outside Core. Core records and
  projects structure; it does not decide task success or model meaning.
- Never commit credentials, private prompts, or a real project `.mnema/` journal.

## Build and test

Rust 1.95 or newer is required.

```powershell
cargo fmt --all -- --check
cargo build --release
cargo test --release
./scripts/ci.ps1 -Mode Fast
```

The registered full local gate is:

```powershell
./scripts/ci.ps1 -Mode Full
```

Long gates default to durable AsyncExec supervision. Use
`-Executor Direct` only for an explicit foreground comparison.

## Compatibility rules

- The journal is append-only. Projections must be reproducible from journal facts.
- Public JSON fields may be added, but existing fields are not renamed, removed,
  or redefined.
- Diagnostic codes are permanent once released.
- Help examples are parsed by tests. Keep every example syntactically valid.
- Update both `README.md` and `README.zh-CN.md` when shared product guidance changes.
- Register test-suite changes in `docs/TEST-CATALOG.md`.

## Pull requests

Keep each pull request focused. Describe the invariant being changed, include
the commands used for verification, and call out any public CLI, JSON, journal,
or migration compatibility impact.

Contributions are accepted under the repository's
`Apache-2.0 OR MIT` dual license.
