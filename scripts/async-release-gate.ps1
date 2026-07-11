[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('Fast', 'Full', 'Nightly')][string]$Mode,
    [Parameter(Mandatory)][string]$RepoRoot,
    [Parameter(Mandatory)][string]$Store,
    [Parameter(Mandatory)][string]$ArtifactRoot,
    [string]$AsyncExec = 'async-exec.exe'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'test-suites.ps1')

$RepoRoot = (Resolve-Path -LiteralPath $RepoRoot).Path
New-Item -ItemType Directory -Force -Path $Store, $ArtifactRoot | Out-Null
$commit = (& git -C $RepoRoot rev-parse HEAD).Trim()
$automationVersion = 'ci-v4'
$targetDir = Join-Path $ArtifactRoot 'cargo-target'
New-Item -ItemType Directory -Force -Path $targetDir | Out-Null
$env:CARGO_TARGET_DIR = $targetDir
$env:NO_COLOR = '1'
$env:TZ = 'UTC'
if ($Mode -eq 'Nightly') {
    $env:MNEMA_DIFF_SEEDS = '256'
    $env:MNEMA_DIFF_EVENTS = '240'
    $env:MNEMA_FUZZ_CASES = '10000'
}
$runningOnWindows = [System.Environment]::OSVersion.Platform -eq [System.PlatformID]::Win32NT
if ($runningOnWindows -and -not $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER) {
    $linker = Get-ChildItem 'C:\Program Files (x86)\Microsoft Visual Studio' -Recurse `
        -Filter link.exe -ErrorAction SilentlyContinue |
        Where-Object FullName -Like '*\bin\Hostx64\x64\link.exe' |
        Sort-Object FullName -Descending | Select-Object -First 1
    if (-not $linker) { throw 'MSVC x64 linker not found; install Visual Studio C++ Build Tools' }
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = $linker.FullName
    $msvcRoot = Split-Path (Split-Path (Split-Path (Split-Path $linker.FullName)))
    $sdkLib = Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\Lib' -Directory `
        -ErrorAction SilentlyContinue | Sort-Object Name -Descending | Select-Object -First 1
    $sdkInclude = Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\Include' -Directory `
        -ErrorAction SilentlyContinue | Sort-Object Name -Descending | Select-Object -First 1
    if (-not $sdkLib -or -not $sdkInclude) { throw 'Windows 10 SDK not found' }
    $env:LIB = @(
        (Join-Path $msvcRoot 'lib\x64'),
        (Join-Path $sdkLib.FullName 'ucrt\x64'),
        (Join-Path $sdkLib.FullName 'um\x64')
    ) -join ';'
    $env:INCLUDE = @(
        (Join-Path $msvcRoot 'include'),
        (Join-Path $sdkInclude.FullName 'ucrt'),
        (Join-Path $sdkInclude.FullName 'shared'),
        (Join-Path $sdkInclude.FullName 'um'),
        (Join-Path $sdkInclude.FullName 'winrt')
    ) -join ';'
}

$inherit = @(
    'PATH', 'SystemRoot', 'windir', 'USERPROFILE', 'HOMEDRIVE', 'HOMEPATH',
    'TEMP', 'TMP', 'PATHEXT', 'ComSpec', 'APPDATA', 'LOCALAPPDATA',
    'ProgramFiles', 'ProgramFiles(x86)', 'ProgramW6432',
    'CommonProgramFiles', 'CommonProgramFiles(x86)', 'CommonProgramW6432',
    'PROCESSOR_ARCHITECTURE', 'CARGO_TARGET_DIR', 'NO_COLOR', 'TZ'
) |
    Where-Object { Test-Path "env:$_" }
foreach ($optional in @('CARGO_HOME', 'RUSTUP_HOME')) {
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
    $requestId = "mnema:${commit}:$($Mode.ToLowerInvariant()):$($Suite.Name):$automationVersion"
    $args = @('start', '--store', $Store, '--request-id', $requestId,
        '--timeout-ms', [string]$Suite.TimeoutMs, '--cwd', $RepoRoot,
        '--stdout-limit', '16777216', '--stderr-limit', '16777216')
    foreach ($name in $inherit) { $args += @('--env', $name) }
    $args += '--'
    $args += $Suite.Argv
    $raw = & $AsyncExec @args
    if ($LASTEXITCODE -ne 0) { throw "async-exec start failed for $($Suite.Name)" }
    [pscustomobject]@{ Suite = $Suite; Handle = ($raw | ConvertFrom-Json) }
}

function Find-RunDirectory([string]$RunId) {
    Get-ChildItem (Join-Path $Store 'runs') -Directory -Recurse -Filter $RunId |
        Select-Object -First 1 -ExpandProperty FullName
}

function Await-Suite([object]$Started) {
    $raw = & $AsyncExec await --store $Store --run-id $Started.Handle.run_id
    if ($LASTEXITCODE -ne 0) { throw "async-exec await failed for $($Started.Suite.Name)" }
    $terminal = $raw | ConvertFrom-Json
    $suiteDir = Join-Path $ArtifactRoot $Started.Suite.Name
    New-Item -ItemType Directory -Force -Path $suiteDir | Out-Null
    $runDir = Find-RunDirectory $Started.Handle.run_id
    $stdout = Join-Path $suiteDir 'stdout.log'
    $stderr = Join-Path $suiteDir 'stderr.log'
    if ($runDir) {
        Copy-Item (Join-Path $runDir 'stdout.log') $stdout -Force -ErrorAction SilentlyContinue
        Copy-Item (Join-Path $runDir 'stderr.log') $stderr -Force -ErrorAction SilentlyContinue
    }
    [pscustomobject]@{
        name = $Started.Suite.Name
        argv = $Started.Suite.Argv
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
    $parallel = @($suites | Where-Object Phase -eq 2 | ForEach-Object { Start-Suite $_ })
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
    executor = 'AsyncExec'
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
