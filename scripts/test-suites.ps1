Set-StrictMode -Version Latest

function Get-MnemaTestSuites {
    param([ValidateSet('Fast', 'Full', 'Nightly')][string]$Mode)

    $all = @(
        [pscustomobject]@{ Name = 'format'; Phase = 0; TimeoutMs = 60000; Argv = @('cargo', 'fmt', '--check') },
        [pscustomobject]@{ Name = 'compile-release'; Phase = 1; TimeoutMs = 600000; Argv = @('cargo', 'test', '--release', '--no-run') },
        [pscustomobject]@{ Name = 'compile-failpoints'; Phase = 1; TimeoutMs = 300000; Argv = @('cargo', 'test', '--release', '--features', 'test-failpoints', '--test', 'crash_atomicity', '--no-run') },
        [pscustomobject]@{ Name = 'unit'; Phase = 2; TimeoutMs = 600000; Argv = @('cargo', 'test', '--release', '--bin', 'mnema') },
        [pscustomobject]@{ Name = 'behavior'; Phase = 2; TimeoutMs = 180000; Argv = @('cargo', 'test', '--release', '--test', 'behavior_harness') },
        [pscustomobject]@{ Name = 'cli-recovery'; Phase = 2; TimeoutMs = 120000; Argv = @('cargo', 'test', '--release', '--test', 'cli_recovery') },
        [pscustomobject]@{ Name = 'v2-v3-compat'; Phase = 2; TimeoutMs = 120000; Argv = @('cargo', 'test', '--release', '--test', 'v2_v3_compat') },
        [pscustomobject]@{ Name = 'v3-runtime'; Phase = 2; TimeoutMs = 180000; Argv = @('cargo', 'test', '--release', '--test', 'v3_runtime') },
        [pscustomobject]@{ Name = 'crash-atomicity'; Phase = 2; TimeoutMs = 300000; Argv = @('cargo', 'test', '--release', '--features', 'test-failpoints', '--test', 'crash_atomicity') },
        [pscustomobject]@{ Name = 'performance-smoke'; Phase = 3; TimeoutMs = 180000; Argv = @('pwsh.exe', '-NoProfile', '-File', 'scripts/benchmark.ps1', '-Sizes', '25') },
        [pscustomobject]@{ Name = 'differential-expanded'; Phase = 4; TimeoutMs = 600000; Argv = @('cargo', 'test', '--release', 'generated_scope_model_matches_full_and_incremental_replay') },
        [pscustomobject]@{ Name = 'fuzz-strict-input'; Phase = 4; TimeoutMs = 300000; Argv = @('cargo', 'test', '--release', 'deterministic_hostile_ascii_corpus_never_panics') }
    )

    switch ($Mode) {
        'Fast' { return @($all | Where-Object Name -in @('format', 'compile-release', 'behavior', 'cli-recovery')) }
        'Full' { return @($all | Where-Object Phase -lt 4) }
        'Nightly' { return $all } # extended suites are registered before being added here
    }
}
