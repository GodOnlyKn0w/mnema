[CmdletBinding()]
param(
    [int[]]$Sizes = @(25),
    [string]$Binary = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (-not $Binary) { $Binary = Join-Path $repo 'target\release\mnema.exe' }
$Binary = (Resolve-Path -LiteralPath $Binary).Path

function Invoke-Mnema([string]$Cwd, [string[]]$Arguments, [string]$Body = '') {
    $info = [Diagnostics.ProcessStartInfo]::new($Binary)
    $info.WorkingDirectory = $Cwd
    $info.UseShellExecute = $false
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $info.RedirectStandardInput = $true
    foreach ($arg in $Arguments) { $null = $info.ArgumentList.Add($arg) }
    $process = [Diagnostics.Process]::Start($info)
    if ($Body) { $process.StandardInput.Write($Body) }
    $process.StandardInput.Close()
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()
    if ($process.ExitCode -ne 0) { throw "mnema $Arguments failed: $stderr" }
    $stdout
}

function Percentile([long[]]$Values, [double]$P) {
    $sorted = @($Values | Sort-Object)
    if ($sorted.Count -eq 0) { return 0 }
    $index = [Math]::Min($sorted.Count - 1, [Math]::Floor(($sorted.Count - 1) * $P))
    $sorted[$index]
}

$root = Join-Path ([IO.Path]::GetTempPath()) ("mnema-bench-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $root | Out-Null
try {
    $null = Invoke-Mnema $root @('init')
    $added = Invoke-Mnema $root @('add', '--format', 'json') "[task] benchmark root`n" | ConvertFrom-Json
    $id = $added.id
    $results = @()
    $completed = 0
    foreach ($size in @($Sizes | Sort-Object)) {
        $latencies = [System.Collections.Generic.List[long]]::new()
        while ($completed -lt $size) {
            $watch = [Diagnostics.Stopwatch]::StartNew()
            $null = Invoke-Mnema $root @('append', '--id', $id) "[progress] benchmark $completed`n"
            $watch.Stop()
            $latencies.Add($watch.ElapsedMilliseconds)
            $completed++
        }
        $timelineWatch = [Diagnostics.Stopwatch]::StartNew()
        $timeline = Invoke-Mnema $root @('timeline', '--since-offset', '0', '--format', 'json') | ConvertFrom-Json
        $timelineWatch.Stop()
        $orientWatch = [Diagnostics.Stopwatch]::StartNew()
        $null = Invoke-Mnema $root @('orient', '--format', 'json')
        $orientWatch.Stop()
        $results += [ordered]@{
            entries = $size
            append_ms = [ordered]@{ p50 = Percentile $latencies.ToArray() 0.50; p95 = Percentile $latencies.ToArray() 0.95; max = ($latencies | Measure-Object -Maximum).Maximum }
            timeline_ms = $timelineWatch.ElapsedMilliseconds
            orient_ms = $orientWatch.ElapsedMilliseconds
            observed_through = $timeline.window.observed_through
        }
    }
    [ordered]@{
        schema = 'mnema.performance-smoke/v1'
        commit = (& git -C $repo rev-parse HEAD).Trim()
        os = [Environment]::OSVersion.VersionString
        logical_processors = [Environment]::ProcessorCount
        results = $results
    } | ConvertTo-Json -Depth 8
} finally {
    Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
}
