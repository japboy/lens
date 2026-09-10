[CmdletBinding()]
param([Parameter(Mandatory = $true)][string]$OutputDirectory)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows -or [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne 'X64' -or $env:VSCMD_ARG_TGT_ARCH -ne 'x64') {
    throw 'Requires an existing native Windows x64 MSVC developer environment.'
}
if (-not $env:WindowsSDKVersion -or -not $env:VCToolsVersion -or $env:CL -or $env:_CL_ -or $env:LINK) {
    throw 'Explicit SDK/toolset and no compiler/linker overrides are required.'
}
$compiler = (Get-Command cl.exe -CommandType Application -ErrorAction Stop).Source
$source = Join-Path $PSScriptRoot 'notification-probe.cpp'
$runner = Join-Path $PSScriptRoot 'notification-supervisor.mjs'
$inputs = @($source, $runner, $PSCommandPath, $compiler)
$hashes = @($inputs | ForEach-Object { (Get-FileHash -Algorithm SHA256 -LiteralPath $_).Hash.ToLowerInvariant() })
$destination = [System.IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $destination) { throw 'OutputDirectory must not exist.' }
$head = & git -C $PSScriptRoot rev-parse HEAD
if ($LASTEXITCODE -ne 0 -or $head -notmatch '^[0-9a-f]{40}$') { throw 'Cannot resolve source SHA.' }
$status = & git -C $PSScriptRoot status --porcelain
if ($LASTEXITCODE -ne 0) { throw 'Cannot resolve dirty state.' }
$null = New-Item -ItemType Directory -Path $destination
$binary = Join-Path $destination 'notification-probe.exe'
$object = Join-Path $destination 'notification-probe.obj'
$arguments = @('/nologo', '/std:c++20', '/EHsc', '/W4', '/WX', '/Od', '/DUNICODE', '/D_UNICODE',
    '/DWINVER=0x0A00', '/D_WIN32_WINNT=0x0A00', $source, "/Fo$object", "/Fe$binary",
    '/link', '/WX', '/MACHINE:X64', 'user32.lib', 'ole32.lib', 'oleaut32.lib', 'uuid.lib', 'runtimeobject.lib')
& $compiler @arguments
if ($LASTEXITCODE -ne 0) { throw 'Notification probe compile/link failed; output retained.' }
for ($i = 0; $i -lt $inputs.Count; $i++) {
    if ((Get-FileHash -Algorithm SHA256 -LiteralPath $inputs[$i]).Hash.ToLowerInvariant() -ne $hashes[$i]) {
        throw 'Build input changed; successful provenance is not emitted.'
    }
}
$os = Get-CimInstance Win32_OperatingSystem
$record = [ordered]@{
    version = 1
    scope = 'notification-probe-build-only'
    source_sha = $head
    worktree_dirty = [bool]$status
    source_sha256 = $hashes[0]
    supervisor_sha256 = $hashes[1]
    script_sha256 = $hashes[2]
    compiler_sha256 = $hashes[3]
    binary_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $binary).Hash.ToLowerInvariant()
    sdk_version = $env:WindowsSDKVersion
    toolset_version = $env:VCToolsVersion
    os_version = $os.Version
    os_build = $os.BuildNumber
    architecture = 'x64'
    probe_executed = $false
    product_admission_granted = $false
}
$json = $record | ConvertTo-Json -Depth 5
$json | Set-Content -LiteralPath (Join-Path $destination 'notification-build-result.json') -Encoding utf8
Write-Output $json
