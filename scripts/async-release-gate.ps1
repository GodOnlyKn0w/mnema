[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('Fast', 'Full', 'Nightly')][string]$Mode,
    [Parameter(Mandatory)][string]$RepoRoot,
    [Parameter(Mandatory)][string]$Store,
    [Parameter(Mandatory)][string]$ArtifactRoot,
    [Parameter(Mandatory)][string]$InvocationId,
    [string]$AsyncExec = 'async-exec',
    [string]$AsyncExecAdapter = 'async-exec-adapter'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'test-suites.ps1')

$RepoRoot = (Resolve-Path -LiteralPath $RepoRoot).Path
New-Item -ItemType Directory -Force -Path $Store, $ArtifactRoot | Out-Null
$commit = (& git -C $RepoRoot rev-parse HEAD).Trim()
$automationVersion = 'ci-v6'
$runManifest = Join-Path $ArtifactRoot 'runs.jsonl'
[IO.File]::WriteAllText($runManifest, '', [Text.UTF8Encoding]::new($false))
$targetDir = if ($env:CARGO_TARGET_DIR) {
    [IO.Path]::GetFullPath($env:CARGO_TARGET_DIR, $RepoRoot)
} else {
    Join-Path $RepoRoot 'target'
}
New-Item -ItemType Directory -Force -Path $targetDir | Out-Null
$env:CARGO_TARGET_DIR = $targetDir
$env:NO_COLOR = '1'
$env:TZ = 'UTC'
$env:MNEMA_RERE_REPLAY_ONLY = '1'
if ($Mode -eq 'Nightly') {
    $env:MNEMA_DIFF_SEEDS = '256'
    $env:MNEMA_DIFF_EVENTS = '240'
    $env:MNEMA_FUZZ_CASES = '10000'
}
$inherit = @(
    'PATH', 'SystemRoot', 'windir', 'USERPROFILE', 'HOMEDRIVE', 'HOMEPATH',
    'TEMP', 'TMP', 'PATHEXT', 'ComSpec', 'APPDATA', 'LOCALAPPDATA',
    'ProgramFiles', 'ProgramFiles(x86)', 'ProgramW6432',
    'CommonProgramFiles', 'CommonProgramFiles(x86)', 'CommonProgramW6432',
    'PROCESSOR_ARCHITECTURE', 'CARGO_TARGET_DIR', 'NO_COLOR', 'TZ',
    'MNEMA_RERE_REPLAY_ONLY'
) |
    Where-Object { Test-Path "env:$_" }
foreach ($optional in @('CARGO_HOME', 'RUSTUP_HOME', 'MNEMA_BIN')) {
    if (Test-Path "env:$optional") { $inherit += $optional }
}
if (Test-Path env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER) {
    $inherit += 'CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER'
}
foreach ($nightlyVariable in @('MNEMA_DIFF_SEEDS', 'MNEMA_DIFF_EVENTS', 'MNEMA_FUZZ_CASES')) {
    if (Test-Path "env:$nightlyVariable") { $inherit += $nightlyVariable }
}
foreach ($buildVariable in @('LIB', 'INCLUDE', 'LIBPATH')) {
    if (Test-Path "env:$buildVariable") { $inherit += $buildVariable }
}

function Start-Suite([object]$Suite) {
    $requestId = "mnema:${commit}:$($Mode.ToLowerInvariant()):$($Suite.Name):${automationVersion}:${InvocationId}"
    $args = @('exec', '--store', $Store, '--request-id', $requestId,
        '--wall-timeout-ms', [string]$Suite.TimeoutMs, '--inline-wait-ms', '1',
        '--ensure-host', '--cwd', $RepoRoot,
        '--stdout-capture-max-bytes', '16777216',
        '--stderr-capture-max-bytes', '16777216',
        '--stdout-response-read-max-bytes', '1',
        '--stderr-response-read-max-bytes', '1')
    foreach ($name in $inherit) { $args += @('--env', $name) }
    $args += '--'
    $args += $Suite.Argv
    $raw = & $AsyncExecAdapter @args
    if ($LASTEXITCODE -ne 0) { throw "async-exec-adapter exec failed for $($Suite.Name)" }
    $result = $raw | ConvertFrom-Json
    $record = [ordered]@{
        schema = 'mnema.ci-run/v1'
        name = $Suite.Name
        phase = $Suite.Phase
        parallel_safe = $Suite.ParallelSafe
        handle = $result.handle
    }
    [IO.File]::AppendAllText(
        $runManifest,
        (($record | ConvertTo-Json -Depth 8 -Compress) + [Environment]::NewLine),
        [Text.UTF8Encoding]::new($false)
    )
    [pscustomobject]@{ Suite = $Suite; Handle = $result.handle }
}

function Read-SuiteStream([object]$Handle, [string]$Stream, [string]$CursorDir, [string]$Destination) {
    $handleJson = $Handle | ConvertTo-Json -Depth 8 -Compress
    $raw = $handleJson | & $AsyncExecAdapter observe --handle - --cursor-dir $CursorDir `
        --stream $Stream --max-bytes 16777216
    if ($LASTEXITCODE -ne 0) { throw "async-exec-adapter observe failed for $Stream" }
    $observation = $raw | ConvertFrom-Json
    [IO.File]::WriteAllBytes($Destination, [Convert]::FromBase64String($observation.bytes_base64))
}

function Await-Suite([object]$Started) {
    $raw = & $AsyncExec await --store $Store --run-id $Started.Handle.run_id
    if ($LASTEXITCODE -ne 0) { throw "async-exec await failed for $($Started.Suite.Name)" }
    $terminal = $raw | ConvertFrom-Json
    $suiteDir = Join-Path $ArtifactRoot $Started.Suite.Name
    New-Item -ItemType Directory -Force -Path $suiteDir | Out-Null
    $stdout = Join-Path $suiteDir 'stdout.log'
    $stderr = Join-Path $suiteDir 'stderr.log'
    $cursorDir = Join-Path $suiteDir 'cursors'
    Read-SuiteStream $Started.Handle 'stdout' $cursorDir $stdout
    Read-SuiteStream $Started.Handle 'stderr' $cursorDir $stderr
    [pscustomobject]@{
        name = $Started.Suite.Name
        argv = $Started.Suite.Argv
        parallel_safe = $Started.Suite.ParallelSafe
        passed = ($terminal.outcome -eq 'exited' -and $terminal.exit_code -eq 0)
        handle = $Started.Handle
        terminal = $terminal
        stdout = $stdout
        stderr = $stderr
    }
}

$startedAt = [DateTimeOffset]::UtcNow
$results = [System.Collections.Generic.List[object]]::new()
$suites = @(Get-MnemaTestSuites $Mode)

foreach ($suite in @($suites | Where-Object Phase -lt 2 | Sort-Object Phase)) {
    $result = Await-Suite (Start-Suite $suite)
    $results.Add($result)
    if (-not $result.passed) { break }
}

if (-not ($results | Where-Object { -not $_.passed })) {
    $phaseTwo = @($suites | Where-Object Phase -eq 2)
    $parallel = @($phaseTwo | Where-Object ParallelSafe | ForEach-Object { Start-Suite $_ })
    foreach ($suite in @($phaseTwo | Where-Object { -not $_.ParallelSafe })) {
        $results.Add((Await-Suite (Start-Suite $suite)))
    }
    foreach ($started in $parallel) { $results.Add((Await-Suite $started)) }
}
if (-not ($results | Where-Object { -not $_.passed })) {
    foreach ($suite in @($suites | Where-Object Phase -gt 2 | Sort-Object Phase)) {
        $results.Add((Await-Suite (Start-Suite $suite)))
        if (-not $results[$results.Count - 1].passed) { break }
    }
}

$report = [ordered]@{
    schema = 'mnema.ci-report/v1'
    repo = $RepoRoot
    commit = $commit
    mode = $Mode
    invocation_id = $InvocationId
    executor = 'AsyncExec'
    run_manifest = $runManifest
    started_at = $startedAt.ToString('O')
    finished_at = [DateTimeOffset]::UtcNow.ToString('O')
    passed = -not [bool]($results | Where-Object { -not $_.passed })
    suites = @($results)
}
$reportPath = Join-Path $ArtifactRoot 'gate-report.json'
[System.IO.File]::WriteAllText(
    $reportPath,
    ($report | ConvertTo-Json -Depth 12),
    [System.Text.UTF8Encoding]::new($false)
)
$report | ConvertTo-Json -Depth 12
if (-not $report.passed) { exit 1 }
