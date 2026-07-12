Set-StrictMode -Version Latest

function Get-MnemaTestSuites {
    param([ValidateSet('Fast', 'Full', 'Nightly')][string]$Mode)

    $all = @(
        [pscustomobject]@{ Name = 'format'; Phase = 0; TimeoutMs = 60000; Argv = @('cargo', 'fmt', '--check') },
        [pscustomobject]@{ Name = 'build-release'; Phase = 1; TimeoutMs = 600000; Argv = @('cargo', 'build', '--release') },
        [pscustomobject]@{ Name = 'compile-release'; Phase = 1; TimeoutMs = 600000; Argv = @('cargo', 'test', '--release', '--no-run') },
        [pscustomobject]@{ Name = 'compile-failpoints'; Phase = 1; TimeoutMs = 300000; Argv = @('cargo', 'test', '--release', '--features', 'test-failpoints', '--test', 'crash_atomicity', '--no-run') },
        [pscustomobject]@{ Name = 'unit'; Phase = 2; TimeoutMs = 600000; Argv = @('cargo', 'test', '--release', '--bin', 'mnema') },
        [pscustomobject]@{ Name = 'behavior'; Phase = 2; TimeoutMs = 180000; Argv = @('cargo', 'test', '--release', '--test', 'behavior_harness') },
        [pscustomobject]@{ Name = 'cli-recovery'; Phase = 2; TimeoutMs = 120000; Argv = @('cargo', 'test', '--release', '--test', 'cli_recovery') },
        [pscustomobject]@{ Name = 'compat-v2-v3'; Phase = 2; TimeoutMs = 120000; Argv = @('cargo', 'test', '--release', '--test', 'v2_v3_compat') },
        [pscustomobject]@{ Name = 'v3-runtime'; Phase = 2; TimeoutMs = 180000; Argv = @('cargo', 'test', '--release', '--test', 'v3_runtime') },
        [pscustomobject]@{ Name = 'crash-atomicity'; Phase = 2; TimeoutMs = 300000; Argv = @('cargo', 'test', '--release', '--features', 'test-failpoints', '--test', 'crash_atomicity') },
        [pscustomobject]@{ Name = 'recursive-rere-smoke'; Phase = 2; TimeoutMs = 180000; Argv = @('python', 'tests/recursive/rere.py', 'replay', 'tests/recursive/smoke.list') },
        [pscustomobject]@{ Name = 'recursive-rere-full'; Phase = 2; TimeoutMs = 300000; Argv = @('python', 'tests/recursive/rere.py', 'replay', 'tests/recursive/full.list') },
        [pscustomobject]@{ Name = 'performance-smoke'; Phase = 3; TimeoutMs = 180000; Argv = @('pwsh.exe', '-NoProfile', '-File', 'scripts/benchmark.ps1', '-Sizes', '25') },
        [pscustomobject]@{ Name = 'differential-expanded'; Phase = 4; TimeoutMs = 600000; Argv = @('cargo', 'test', '--release', 'generated_scope_model_matches_full_and_incremental_replay') },
        [pscustomobject]@{ Name = 'fuzz-strict-input'; Phase = 4; TimeoutMs = 300000; Argv = @('cargo', 'test', '--release', 'deterministic_hostile_ascii_corpus_never_panics') },
        [pscustomobject]@{ Name = 'recursive-rere-crash'; Phase = 4; TimeoutMs = 180000; Argv = @('python', 'tests/recursive/rere.py', 'replay', 'tests/recursive/crash.list') }
    )

    switch ($Mode) {
        'Fast' { return @($all | Where-Object Name -in @('format', 'build-release', 'compile-release', 'behavior', 'cli-recovery', 'recursive-rere-smoke')) }
        'Full' { return @($all | Where-Object Phase -lt 4) }
        'Nightly' { return $all }
    }
}
