[CmdletBinding()]
param(
    [ValidateSet('Fast', 'Full', 'Nightly')][string]$Mode = 'Fast',
    [ValidateSet('Direct', 'AsyncExec')][string]$Executor = 'Direct',
    [string]$Store = 'D:\harness\async-exec-runs\tasktree-core'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'test-suites.ps1')

$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$commit = (& git -C $repo rev-parse HEAD).Trim()
$artifactRoot = Join-Path $repo ".artifacts\ci\$commit\$($Mode.ToLowerInvariant())"
New-Item -ItemType Directory -Force -Path $artifactRoot | Out-Null
$env:MNEMA_RERE_REPLAY_ONLY = '1'
if ($Mode -eq 'Nightly') {
    $env:MNEMA_DIFF_SEEDS = '256'
    $env:MNEMA_DIFF_EVENTS = '240'
    $env:MNEMA_FUZZ_CASES = '10000'
}

if ($Executor -eq 'AsyncExec') {
    & (Join-Path $PSScriptRoot 'async-release-gate.ps1') -Mode $Mode -RepoRoot $repo `
        -Store $Store -ArtifactRoot $artifactRoot
    exit $LASTEXITCODE
}

$startedAt = [DateTimeOffset]::UtcNow
$results = [System.Collections.Generic.List[object]]::new()
function Invoke-DirectSuite([object]$Suite, [string]$WorkingDirectory, [string]$Stdout, [string]$Stderr) {
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $Suite.Argv[0]
    foreach ($argument in $Suite.Argv[1..($Suite.Argv.Count - 1)]) {
        $null = $info.ArgumentList.Add($argument)
    }
    $info.WorkingDirectory = $WorkingDirectory
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $info
    $null = $process.Start()
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit($Suite.TimeoutMs)) {
        $process.Kill($true)
        $process.WaitForExit()
        $outcome = 'timed-out'
        $exitCode = $null
    } else {
        $outcome = 'exited'
        $exitCode = $process.ExitCode
    }
    [Threading.Tasks.Task]::WaitAll(@($stdoutTask, $stderrTask))
    [IO.File]::WriteAllText($Stdout, $stdoutTask.Result)
    [IO.File]::WriteAllText($Stderr, $stderrTask.Result)
    [pscustomobject]@{ outcome = $outcome; exit_code = $exitCode }
}

foreach ($suite in @(Get-MnemaTestSuites $Mode | Sort-Object Phase)) {
    $suiteDir = Join-Path $artifactRoot $suite.Name
    New-Item -ItemType Directory -Force -Path $suiteDir | Out-Null
    $stdout = Join-Path $suiteDir 'stdout.log'
    $stderr = Join-Path $suiteDir 'stderr.log'
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $process = Invoke-DirectSuite $suite $repo $stdout $stderr
    $watch.Stop()
    $passed = $process.outcome -eq 'exited' -and $process.exit_code -eq 0
    $results.Add([pscustomobject]@{
        name = $suite.Name; argv = $suite.Argv; passed = $passed
        process = [ordered]@{ outcome = $process.outcome; exit_code = $process.exit_code; duration_ms = $watch.ElapsedMilliseconds }
        stdout = $stdout; stderr = $stderr
    })
    if (-not $passed) { break }
}

$report = [ordered]@{
    schema = 'mnema.ci-report/v1'; repo = $repo; commit = $commit; mode = $Mode
    executor = 'Direct'; started_at = $startedAt.ToString('O')
    finished_at = [DateTimeOffset]::UtcNow.ToString('O')
    passed = -not [bool]($results | Where-Object { -not $_.passed }); suites = @($results)
}
[System.IO.File]::WriteAllText(
    (Join-Path $artifactRoot 'gate-report.json'),
    ($report | ConvertTo-Json -Depth 12),
    [System.Text.UTF8Encoding]::new($false)
)
$report | ConvertTo-Json -Depth 12
if (-not $report.passed) { exit 1 }
