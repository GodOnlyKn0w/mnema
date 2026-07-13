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

`cargo build --release && cargo test --release` is the minimum repository
discipline. The registered authoritative local gate is
`scripts/ci.ps1 -Mode Full`, which includes that release contract plus the
process, compatibility, crash and performance suites listed in
`TEST-CATALOG.md`. Behavior snapshots supplement semantic assertions; they
never replace them.

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

`scripts/ci.ps1` is the portable local entrypoint and defaults to
`-Executor Direct`, so a contributor needs only the repository toolchain.
Maintainers may opt into durable execution with `-Executor AsyncExec`; both
mechanics preserve suite semantics and emit `mnema.ci-report/v1`.

On Windows, the entrypoint uses Visual Studio's installed `vswhere` metadata
to import the x64 MSVC developer environment when available. This avoids a
common PATH collision with Git for Windows' unrelated `link.exe`. Unix-like
hosts skip this step.

AsyncExec Store resolution is explicit `-Store`, then
`MNEMA_ASYNC_EXEC_STORE`, then the current user's local application-data
directory (`mnema/async-exec`). The repository carries no machine-specific
Store path. Existing durable history can remain in place by setting the
environment variable to that Store.

Examples:

```powershell
pwsh scripts/ci.ps1 -Mode Fast
pwsh scripts/ci.ps1 -Mode Full -Executor AsyncExec
pwsh scripts/ci.ps1 -Mode Full -Executor AsyncExec -Store D:\path\to\runs
```

The durable path uses the installed consumer surfaces rather than Store layout:
`async-exec-adapter exec --ensure-host` submits structured argv with explicit
cwd, environment allowlist, wall timeout, stdin EOF, and separate canonical
capture/response budgets; Core `await` supplies the TerminalEvent; lossless
adapter `observe` supplies exact base64 log bytes for `.artifacts/ci/`. The gate
never discovers `runs/` or copies private Store files.

By default Cargo uses the current worktree's `target/`, preserving its usable
incremental toolchain state. Concurrent callers that share a worktree must set
an explicit distinct `CARGO_TARGET_DIR`; the gate preserves and allowlists it
instead of inventing a global or per-commit cold cache.

async-exec does not decide whether a semantic test passed, retry a failed suite,
understand a mnema strand, or become a workflow engine. The PowerShell wrapper
only aggregates the registered process results into the gate report.

## Recursive rere layer

The recursive black-box layer lives under `tests/recursive/`. Vendored
`rere.py` (pinned in `tests/recursive/SOURCE`) owns exact record/replay of shell
stdout, stderr, and return code. `driver.py` owns isolation, fixed presentation
environment, and stable structural reports so dynamic IDs and timestamps never
become the golden surface.

Automated gates run only `python tests/recursive/rere.py replay <list>` with
`MNEMA_RERE_REPLAY_ONLY=1`. Recording is a deliberate maintainer action and is
rejected under that environment flag. AsyncExec may durably host replay but
never interprets mnema task semantics.

## Next extensions

Implemented and planned suites are registered only in `TEST-CATALOG.md`; this policy document does not maintain a second roadmap.

