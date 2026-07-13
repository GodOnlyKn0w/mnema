[CmdletBinding()]
param(
    [ValidateSet('Fast', 'Full', 'Nightly')][string]$Mode = 'Fast',
    [ValidateSet('Direct', 'AsyncExec')][string]$Executor = 'Direct',
    [string]$Store
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'test-suites.ps1')

function Initialize-WindowsMsvcEnvironment {
    if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { return }

    $installerRoot = [Environment]::GetFolderPath([Environment+SpecialFolder]::ProgramFilesX86)
    $vswhere = Join-Path $installerRoot 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path -LiteralPath $vswhere)) { return }

    $installPath = & $vswhere -latest -products '*' `
        -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
        -property installationPath | Select-Object -First 1
    if (-not $installPath) { return }
    $installPath = $installPath.Trim()

    $vcvars = Join-Path $installPath 'VC\Auxiliary\Build\vcvars64.bat'
    if (-not (Test-Path -LiteralPath $vcvars)) { return }

    $developerEnvironment = '"{0}" >nul && set' -f $vcvars
    foreach ($line in & cmd.exe /d /s /c $developerEnvironment) {
        $separator = $line.IndexOf('=')
        if ($separator -le 0) { continue }
        $name = $line.Substring(0, $separator)
        $value = $line.Substring($separator + 1)
        Set-Item -LiteralPath "env:$name" -Value $value
    }
}

Initialize-WindowsMsvcEnvironment

$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$commit = (& git -C $repo rev-parse HEAD).Trim()
$invocationId = [Guid]::NewGuid().ToString('N')
$artifactRoot = Join-Path $repo ".artifacts\ci\$commit\$($Mode.ToLowerInvariant())"
New-Item -ItemType Directory -Force -Path $artifactRoot | Out-Null
$env:MNEMA_RERE_REPLAY_ONLY = '1'
if ($Mode -eq 'Nightly') {
    $env:MNEMA_DIFF_SEEDS = '256'
    $env:MNEMA_DIFF_EVENTS = '240'
    $env:MNEMA_FUZZ_CASES = '10000'
}

if ($Executor -eq 'AsyncExec') {
    if (-not $Store) { $Store = $env:MNEMA_ASYNC_EXEC_STORE }
    if (-not $Store) {
        $localData = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
        if (-not $localData) { $localData = [IO.Path]::GetTempPath() }
        $Store = Join-Path $localData 'mnema\async-exec'
    }
    & (Join-Path $PSScriptRoot 'async-release-gate.ps1') -Mode $Mode -RepoRoot $repo `
        -Store $Store -ArtifactRoot $artifactRoot -InvocationId $invocationId
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
        name = $suite.Name; argv = $suite.Argv; parallel_safe = $suite.ParallelSafe
        passed = $passed
        process = [ordered]@{ outcome = $process.outcome; exit_code = $process.exit_code; duration_ms = $watch.ElapsedMilliseconds }
        stdout = $stdout; stderr = $stderr
    })
    if (-not $passed) { break }
}

$report = [ordered]@{
    schema = 'mnema.ci-report/v1'; repo = $repo; commit = $commit; mode = $Mode
    invocation_id = $invocationId
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
