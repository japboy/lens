# Run only in an existing Windows 11 x64 MSVC Developer PowerShell 7.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidatePattern('^[0-9a-f]{40}$')][string]$ExpectedSourceSha,
    [Parameter(Mandatory = $true)][string]$OutputDirectory
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if ($PSVersionTable.PSVersion.Major -lt 7 -or -not $IsWindows -or
    [Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne 'X64' -or
    [Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture -ne 'X64') { throw 'Windows x64 PowerShell 7 required.' }
$os = Get-CimInstance Win32_OperatingSystem
if ([int]$os.BuildNumber -lt 22000 -or $os.ProductType -ne 1) { throw 'Windows 11 workstation required.' }
if ($env:VSCMD_ARG_TGT_ARCH -ne 'x64' -or -not $env:WindowsSDKVersion -or -not $env:VCToolsVersion) { throw 'Enter the installed x64 MSVC developer environment first.' }
if ($env:CL -or $env:_CL_ -or $env:LINK) { throw 'Compiler/linker overrides are not admitted.' }
$null = Get-Command cl.exe -CommandType Application -ErrorAction Stop
$node = (Get-Command node -CommandType Application -ErrorAction Stop).Source
$pwsh = (Get-Command pwsh -CommandType Application -ErrorAction Stop).Source
$repo = (& git -C $PSScriptRoot rev-parse --show-toplevel)
if ($LASTEXITCODE -ne 0) { throw 'Cannot resolve repository.' }
function Assert-Source {
    $head = & git -C $repo rev-parse HEAD
    if ($LASTEXITCODE -ne 0 -or $head -ne $ExpectedSourceSha) { throw 'Checkout is not the exact requested commit.' }
    $dirty = & git -C $repo status --porcelain --untracked-files=all
    if ($LASTEXITCODE -ne 0 -or $dirty) { throw 'A clean checkout is required; do not discard local changes to run this script.' }
}
Assert-Source
$destination = [IO.Path]::GetFullPath($OutputDirectory)
$repoPath = [IO.Path]::GetFullPath($repo).TrimEnd([IO.Path]::DirectorySeparatorChar)
if ($destination.Equals($repoPath, [StringComparison]::OrdinalIgnoreCase) -or
    $destination.StartsWith($repoPath + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Evidence must be outside the checkout.' }
if (Test-Path -LiteralPath $destination) { throw 'Output directory must not exist; existing evidence is never overwritten.' }
$null = New-Item -ItemType Directory -Path $destination
$commands = [Collections.Generic.List[object]]::new()
function Save-Json($Value, [string]$Name) {
    $Value | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath (Join-Path $destination $Name) -Encoding utf8
}
function Invoke-Recorded([string]$Name, [string]$Executable, [string[]]$CommandArguments) {
    $started = [DateTimeOffset]::UtcNow
    $timer = [Diagnostics.Stopwatch]::StartNew()
    $start = [Diagnostics.ProcessStartInfo]::new($Executable)
    $start.UseShellExecute = $false
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.WorkingDirectory = $repo
    foreach ($argument in $CommandArguments) { $start.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    $streams = @()
    $code = $null
    $stopReason = $null
    $cleanup = 'not-started'
    $deadlineMs = 180000
    $outputCap = 16777216
    try {
        if (-not $process.Start()) { throw 'Process did not start.' }
        $cleanup = 'termination-unconfirmed'
        foreach ($pair in @(@('stdout', $process.StandardOutput.BaseStream), @('stderr', $process.StandardError.BaseStream))) {
            $buffer = [byte[]]::new(8192)
            $streams += @{ input = $pair[1]; output = [IO.File]::Open((Join-Path $destination "$Name.$($pair[0]).txt"), [IO.FileMode]::CreateNew); buffer = $buffer; task = $pair[1].ReadAsync($buffer, 0, $buffer.Length); ended = $false; bytes = 0 }
        }
        $killAt = $null
        while ($true) {
            foreach ($stream in $streams) {
                if (-not $stream.ended -and $stream.task.IsCompleted) {
                    $read = $stream.task.GetAwaiter().GetResult()
                    if ($read -eq 0) { $stream.ended = $true; continue }
                    $remaining = $outputCap - $stream.bytes
                    $retained = [Math]::Min($read, $remaining)
                    if ($retained -gt 0) { $stream.output.Write($stream.buffer, 0, $retained); $stream.bytes += $retained }
                    if ($read -gt $remaining) { $stopReason = 'output-limit' }
                    $stream.task = $stream.input.ReadAsync($stream.buffer, 0, $stream.buffer.Length)
                }
            }
            if ($null -eq $killAt -and ($stopReason -or $timer.ElapsedMilliseconds -ge $deadlineMs)) {
                if (-not $stopReason) { $stopReason = 'external-command-deadline' }
                $killAt = $timer.ElapsedMilliseconds
                if (-not $process.HasExited) { $process.Kill($true) }
            }
            if ($process.HasExited -and @($streams | Where-Object { -not $_.ended }).Count -eq 0) { break }
            if ($null -ne $killAt -and $timer.ElapsedMilliseconds - $killAt -ge 5000) { break }
            Start-Sleep -Milliseconds 10
        }
        if ($process.HasExited) { $code = $process.ExitCode; $cleanup = 'closed' }
    } catch {
        $stopReason = "launch-or-stream-error: $($_.Exception.Message)"
        if ($cleanup -ne 'not-started') {
            try { if (-not $process.HasExited) { $process.Kill($true) }; if ($process.WaitForExit(5000)) { $code = $process.ExitCode; $cleanup = 'closed' } } catch { $cleanup = 'termination-unconfirmed' }
        }
    } finally {
        foreach ($stream in $streams) { $stream.input.Dispose(); $stream.output.Dispose() }
        $process.Dispose()
        $timer.Stop()
    }
    $commands.Add([ordered]@{ name = $Name; executable = $Executable; arguments = $CommandArguments; started_utc = $started.ToString('o'); elapsed_ms = $timer.ElapsedMilliseconds; exit_code = $code; external_stop_reason = $stopReason; direct_child_cleanup = $cleanup; stream_limit_bytes = $outputCap; deadline_ms = $deadlineMs; process_tree_cleanup = 'Kill(entireProcessTree) requested on abnormal live child; descendant closure not independently proven' })
    Save-Json @($commands.ToArray()) 'commands.json'
    if ($stopReason -or $cleanup -ne 'closed') { return 1 }
    return $code
}
function Save-Processes([string]$Name) {
    # Only these experiment images are inspected. No unrelated command lines are collected.
    $names = @('controlled-fixture.exe', 'acquisition-probe.exe', 'provider-stall-fixture.exe', 'provider-stall-probe.exe', 'notification-probe.exe')
    $rows = @(Get-CimInstance Win32_Process | Where-Object { $_.Name -in $names } | Select-Object Name, ProcessId, ParentProcessId, ExecutablePath, CreationDate)
    Save-Json ([ordered]@{ observed_utc = [DateTimeOffset]::UtcNow.ToString('o'); processes = $rows; scope = 'named experiment images only; snapshot is not proof of historical process absence' }) $Name
}
function Get-SourceHashes {
    @(Get-ChildItem -LiteralPath $experiment -File -Recurse | Where-Object { $_.Extension -in @('.mjs', '.cpp', '.ps1') } | Sort-Object FullName | ForEach-Object { [ordered]@{ path = [IO.Path]::GetRelativePath($repo, $_.FullName); sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant() } })
}
$native = Join-Path $repo 'experiments/windows-target-authority/native'
$experiment = Join-Path $repo 'experiments/windows-target-authority'
$runner = Join-Path $PSScriptRoot 'runner.mjs'
$failure = $null
$initialSourceHashes = @(Get-SourceHashes)
Save-Json $initialSourceHashes 'source-hashes-before.json'
try {
    Save-Json ([ordered]@{ source_sha = $ExpectedSourceSha; worktree_dirty = $false; os_version = $os.Version; os_build = $os.BuildNumber; architecture = 'x64'; powershell = $PSVersionTable.PSVersion.ToString(); sdk_version = $env:WindowsSDKVersion; toolset_version = $env:VCToolsVersion; product_support = 'Unsupported'; started_utc = [DateTimeOffset]::UtcNow.ToString('o') }) 'environment.json'
    Save-Processes 'processes-before.json'
    if ((Invoke-Recorded 'node-version' $node @('--version')) -ne 0) { throw 'Node version check failed.' }
    $tests = @(Get-ChildItem -LiteralPath $experiment -Recurse -Filter '*.test.mjs' | Sort-Object FullName | ForEach-Object FullName)
    if ((Invoke-Recorded 'synthetic-tests' $node (@('--test') + $tests)) -ne 0) { throw 'Synthetic tests failed; native matrix not started.' }
    $builds = @(
        @{ name = 'acquisition'; script = (Join-Path $native 'build.ps1') },
        @{ name = 'notifications'; script = (Join-Path $native 'build-notifications.ps1') },
        @{ name = 'provider'; script = (Join-Path $native 'provider-stall/build.ps1') }
    )
    foreach ($build in $builds) {
        if ((Invoke-Recorded "build-$($build.name)" $pwsh @('-NoProfile', '-File', $build.script, '-OutputDirectory', (Join-Path $destination $build.name))) -ne 0) { throw "Build failed: $($build.name). Partial evidence retained." }
    }
    Assert-Source
    foreach ($scenario in @('ordinary', 'stop-before-root', 'stop-before-capture', 'stop-before-probe', 'stop-before-commit', 'replace-before-root', 'replace-before-capture', 'replace-before-probe', 'replace-before-commit', 'mismatched-pid')) {
        $null = Invoke-Recorded "acquisition-$scenario" $node @($runner, 'acquisition', (Join-Path $destination 'acquisition/controlled-fixture.exe'), (Join-Path $destination 'acquisition/acquisition-probe.exe'), $scenario)
    }
    foreach ($scenario in @('release', 'provider-deadline', 'stop-inflight')) {
        $null = Invoke-Recorded "provider-$scenario" $node @($runner, 'provider', (Join-Path $destination 'provider/provider-stall-fixture.exe'), (Join-Path $destination 'provider/provider-stall-probe.exe'), $scenario)
    }
    Assert-Source
    if (($initialSourceHashes | ConvertTo-Json -Depth 5 -Compress) -ne (@(Get-SourceHashes) | ConvertTo-Json -Depth 5 -Compress)) { throw 'Experiment source inputs changed during collection.' }
} catch {
    $failure = $_.Exception.Message
} finally {
    try { Save-Processes 'processes-after.json' } catch { Save-Json @{ status = 'unavailable'; reason = $_.Exception.Message } 'processes-after.json' }
    $hashes = @(Get-SourceHashes)
    Save-Json $hashes 'source-hashes.json'
    Save-Json ([ordered]@{ collection_error = $failure; native_success_inferred = $false; review_required = $true; commands = $commands.Count; product_support = 'Unsupported'; completed_utc = [DateTimeOffset]::UtcNow.ToString('o') }) 'collection-result.json'
    $files = @(Get-ChildItem -LiteralPath $destination -Recurse -File | Sort-Object FullName | ForEach-Object { [ordered]@{ path = [IO.Path]::GetRelativePath($destination, $_.FullName); sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant() } })
    Save-Json $files 'checksums.json'
}
if ($failure) { throw $failure }
if (@($commands | Where-Object { $_.exit_code -ne 0 -or $_.external_stop_reason -or $_.direct_child_cleanup -ne 'closed' }).Count -gt 0) { throw 'One or more cases failed or required external termination; retain and review the evidence.' }
Write-Output "Evidence retained at $destination. Exit zero does not certify product support or native target continuity."
