# Recursive rere black-box layer

Record/replay harness for **recursive scope** claims: virtual Journal root and
any strand root share the same downward semantics; refs never silently expand
subtree membership.

## Layout

| Path | Role |
|---|---|
| `rere.py` | Vendored record/replay tool (see `SOURCE`) |
| `driver.py` | Isolation + stable structural reports |
| `smoke.list` / `full.list` / `crash.list` | Lane-specific shell lists |
| `tests.list` | Maintainer union of all scenarios |
| `*.list.bi` | Reviewed binary snapshots (evidence) |

## Policy

- **CI / `scripts/ci.ps1` only replays.** Suite argv is always
  `python tests/recursive/rere.py replay <list>` with
  `MNEMA_RERE_REPLAY_ONLY=1`.
- **Record is a deliberate maintainer action** after reviewing driver output:

```powershell
cargo build --release
$env:MNEMA_BIN = (Resolve-Path .\target\release\mnema.exe).Path
python tests/recursive/rere.py record tests/recursive/smoke.list
python tests/recursive/rere.py record tests/recursive/full.list
python tests/recursive/rere.py record tests/recursive/crash.list
# optional union:
python tests/recursive/rere.py record tests/recursive/tests.list
```

- Snapshot diffs are public-contract reviews. Do not rewrite `.bi` casually.
- AsyncExec only hosts the replay process; it does not interpret strand semantics.

## Isolation

`driver.py` creates a fresh temp directory per scenario, runs `mnema -C <tmp>`,
and never discovers the repository `.mnema/`. Presentation env is fixed
(`NO_COLOR=1`, `TZ=UTC`). Output is slug/membership-oriented so IDs, timestamps,
and offsets are not part of the golden surface.

## Lanes

| Suite id | List | Gate |
|---|---|---|
| `recursive-rere-smoke` | `smoke.list` | Fast, Full |
| `recursive-rere-full` | `full.list` | Full, Nightly |
| `recursive-rere-crash` | `crash.list` | Nightly |
