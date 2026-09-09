# Run in an existing x64 MSVC Developer PowerShell. Never installs tools.
[CmdletBinding()]
param([Parameter(Mandatory = $true)][string]$OutputDirectory)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows -or [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne 'X64') {
    throw 'This experiment requires a native Windows x64 host.'
}
if ($env:VSCMD_ARG_TGT_ARCH -ne 'x64') { throw 'Enter an x64 MSVC developer environment first.' }
if (-not $env:WindowsSDKVersion -or -not $env:VCToolsVersion) { throw 'SDK/toolset versions must be explicit in the developer environment.' }
if ($env:CL -or $env:_CL_ -or $env:LINK) { throw 'Compiler/linker override environment variables are not admitted.' }
$compiler = (Get-Command cl.exe -CommandType Application -ErrorAction Stop).Source
$source = Join-Path $PSScriptRoot 'build-smoke.cpp'
$sourceHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $source).Hash.ToLowerInvariant()
$fixtureSource = Join-Path $PSScriptRoot 'controlled-fixture.cpp'
$fixtureHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $fixtureSource).Hash.ToLowerInvariant()
$probeSource = Join-Path $PSScriptRoot 'acquisition-probe.cpp'
$probeHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $probeSource).Hash.ToLowerInvariant()
$scriptHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $PSCommandPath).Hash.ToLowerInvariant()
$compilerHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $compiler).Hash.ToLowerInvariant()
$destination = [System.IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $destination) { throw 'OutputDirectory must not exist; no existing output is overwritten.' }
$head = & git -C $PSScriptRoot rev-parse HEAD
if ($LASTEXITCODE -ne 0 -or $head -notmatch '^[0-9a-f]{40}$') { throw 'Cannot resolve source commit.' }
$status = & git -C $PSScriptRoot status --porcelain
if ($LASTEXITCODE -ne 0) { throw 'Cannot resolve source dirty state.' }
$os = Get-CimInstance Win32_OperatingSystem
$null = New-Item -ItemType Directory -Path $destination
$binary = Join-Path $destination 'build-smoke.exe'
$object = Join-Path $destination 'build-smoke.obj'
$arguments = @('/nologo', '/std:c++20', '/EHsc', '/W4', '/WX', '/Od', '/DUNICODE', '/D_UNICODE',
    '/DWINVER=0x0A00', '/D_WIN32_WINNT=0x0A00', $source, "/Fo$object", "/Fe$binary",
    '/link', '/WX', '/MACHINE:X64', '/INCLUDE:LensSdkLinkSmoke', 'ole32.lib', 'runtimeobject.lib', 'uuid.lib')
& $compiler @arguments
if ($LASTEXITCODE -ne 0) { throw 'Windows SDK compile/link failed; output directory retained for inspection.' }
$fixtureBinary = Join-Path $destination 'controlled-fixture.exe'
$fixtureObject = Join-Path $destination 'controlled-fixture.obj'
$fixtureArguments = @('/nologo', '/std:c++20', '/EHsc', '/W4', '/WX', '/Od', '/DUNICODE', '/D_UNICODE',
    '/DWINVER=0x0A00', '/D_WIN32_WINNT=0x0A00', $fixtureSource, "/Fo$fixtureObject", "/Fe$fixtureBinary",
    '/link', '/WX', '/MACHINE:X64', 'user32.lib', 'gdi32.lib')
& $compiler @fixtureArguments
if ($LASTEXITCODE -ne 0) { throw 'Controlled fixture compile/link failed; output retained for inspection.' }
$probeBinary = Join-Path $destination 'acquisition-probe.exe'
$probeObject = Join-Path $destination 'acquisition-probe.obj'
$probeArguments = @('/nologo', '/std:c++20', '/EHsc', '/W4', '/WX', '/Od', '/DUNICODE', '/D_UNICODE',
    '/DWINVER=0x0A00', '/D_WIN32_WINNT=0x0A00', $probeSource, "/Fo$probeObject", "/Fe$probeBinary",
    '/link', '/WX', '/MACHINE:X64', 'user32.lib', 'ole32.lib', 'oleaut32.lib', 'uuid.lib',
    'runtimeobject.lib', 'windowsapp.lib', 'd3d11.lib', 'dxgi.lib')
& $compiler @probeArguments
if ($LASTEXITCODE -ne 0) { throw 'Acquisition probe compile/link failed; output retained for inspection.' }
if ((Get-FileHash -Algorithm SHA256 -LiteralPath $source).Hash.ToLowerInvariant() -ne $sourceHash -or
    (Get-FileHash -Algorithm SHA256 -LiteralPath $probeSource).Hash.ToLowerInvariant() -ne $probeHash -or
    (Get-FileHash -Algorithm SHA256 -LiteralPath $fixtureSource).Hash.ToLowerInvariant() -ne $fixtureHash -or
    (Get-FileHash -Algorithm SHA256 -LiteralPath $PSCommandPath).Hash.ToLowerInvariant() -ne $scriptHash -or
    (Get-FileHash -Algorithm SHA256 -LiteralPath $compiler).Hash.ToLowerInvariant() -ne $compilerHash) {
    throw 'Build inputs changed during compile/link; no successful provenance record is emitted.'
}
$metadataText = & $binary --metadata
if ($LASTEXITCODE -ne 0) { throw 'Metadata executable failed.' }
$metadata = $metadataText | ConvertFrom-Json
if ($metadata.version -ne 1 -or $metadata.architecture -ne 'x64' -or $metadata.native_acquisition_executed -ne $false -or $metadata.product_admission_granted -ne $false) {
    throw 'Unexpected executable metadata.'
}
$record = [ordered]@{
    version = 3
    scope = 'sdk-smoke-fixture-and-acquisition-probe-build-only'
    source_sha = $head
    worktree_dirty = [bool]$status
    source_sha256 = $sourceHash
    fixture_source_sha256 = $fixtureHash
    fixture_binary_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $fixtureBinary).Hash.ToLowerInvariant()
    fixture_executed = $false
    probe_source_sha256 = $probeHash
    probe_binary_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $probeBinary).Hash.ToLowerInvariant()
    probe_executed = $false
    script_sha256 = $scriptHash
    binary_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $binary).Hash.ToLowerInvariant()
    compiler_sha256 = $compilerHash
    sdk_version = $env:WindowsSDKVersion
    toolset_version = $env:VCToolsVersion
    os_version = $os.Version
    os_build = $os.BuildNumber
    executable = $metadata
}
$json = $record | ConvertTo-Json -Depth 5
$json | Set-Content -LiteralPath (Join-Path $destination 'build-result.json') -Encoding utf8
Write-Output $json
