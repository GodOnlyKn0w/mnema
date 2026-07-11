# Testing mnema

mnema's tests protect an append-only journal and its reproducible projections.
No single runner is the test architecture: each layer below owns a different
claim, and a failure must remain attributable to that layer.

## Layers

| Layer | Claim | Primary mechanism | Default gate |
|---|---|---|---|
| Unit and contract | Parsers, event rules, diagnostics, and public JSON contracts behave exactly as specified | Rust tests under `src/tests/` | Every change |
| Runtime integration | The release CLI reads, writes, migrates, and recovers real on-disk journals | Rust tests under `tests/` | Every change |
| Model and differential | Replay agrees with a deliberately simple reference model; cached and uncached/full and incremental paths agree | Generated event sequences plus an in-test oracle | Every change at bounded size |
| Concurrency and crash safety | Concurrent processes and interruption expose only valid old or new durable states | Release-binary process tests and test-only failpoints | CI; expanded scheduled run |
| Historical compatibility | Checked-in historical journals retain their promised projections and migration identities | Immutable, versioned fixtures | Every change touching persistence |
| CLI behavior | Exit status, stdout, and stderr of selected user journeys do not drift accidentally | Normalized record/replay snapshots | Every change touching CLI output |
| Scale and fuzzing | Valid large journals remain practical; hostile input does not panic or hang | Benchmarks and fuzz targets | Scheduled or explicit |

`cargo build --release && cargo test --release` remains the repository's
authoritative local gate. Behavior snapshots supplement it; they never replace
semantic assertions.

## Required invariants

New harness work should converge on these cross-cutting checks:

1. Replaying the same journal is deterministic.
2. A disposable cache cannot change a projection and can be deleted or rebuilt.
3. Full replay and incremental replay agree at the same journal offset.
4. A scoped query under strand `X` agrees with applying the same scope oracle to
   the corresponding global facts.
5. `belongs-to` determines subtree membership; refs aid evidence and discovery
   but do not silently add members to a subtree.
6. A successful atomic command exposes all of its journal facts, while an
   interrupted command exposes either the prior state or the complete new state.
7. Historical fixtures are immutable. A new expected interpretation requires a
   new fixture version, not editing the old evidence.
8. Public JSON fields may be added but existing fields cannot be renamed,
   removed, or change meaning.

## Isolation

Every process-level scenario gets a fresh temporary project directory and an
explicit `MNEMA_HOME`. Tests must not discover or mutate the repository's own
`.mnema/`. Parallel worktrees should use distinct `CARGO_TARGET_DIR` values when
they run release builds concurrently.

Tests fix locale, timezone, color mode, and other presentation inputs where the
platform permits. Platform-specific expectations are split explicitly; they are
not hidden by a normalizer.

## Behavior snapshots

The initial behavior harness lives in `tests/behavior/`. Its scenario manifest
is reviewable source; recorded stdout/stderr is generated evidence.

Before adopting `rere.py`, add a thin scenario driver that:

- creates an isolated directory per scenario;
- executes setup separately from the command being asserted;
- canonicalizes JSON structurally;
- replaces only declared dynamic values such as temporary paths, timestamps,
  generated IDs, hashes, and offsets;
- rejects undeclared dynamic output instead of silently scrubbing it;
- records the release binary's status, stdout, and stderr.

Snapshot recording is a deliberate maintainer action. CI only replays. Snapshot
changes must be reviewed like public API changes.

## async-exec boundary

`scripts/ci.ps1` is the local entrypoint. `-Executor Direct` and
`-Executor AsyncExec` select execution mechanics without changing suite
semantics; both emit `mnema.ci-report/v1`. The async adapter supplies explicit
suite, cwd, environment allowlist, timeout and output budget, then preserves the
canonical Handle, TerminalEvent and logs under `.artifacts/ci/`.

async-exec does not decide whether a semantic test passed, retry a failed suite,
understand a mnema strand, or become a workflow engine. The PowerShell wrapper
only aggregates the registered process results into the gate report.

## Planned order

1. Extend the executable behavior driver from structural assertions to reviewed snapshots.
2. Extend the first immutable v2→v3 fixture with typed-unlink and retired-why versions when those historical shapes change.
3. Expand the deterministic scope/replay generator beyond the bounded CI seeds when a nightly lane exists.
4. Extend the initial multi-process parent+refs case with test-only persistence failpoints.
5. Vendor or pin `rere.py` only after normalization policy has reviewed examples.
6. Add scheduled fuzz, large-journal, and async-exec-hosted suites.

