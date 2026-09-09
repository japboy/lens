# Compile only, in an existing native Windows x64 MSVC Developer PowerShell.
[CmdletBinding()]
param([Parameter(Mandatory = $true)][string]$OutputDirectory)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows -or [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne 'X64') { throw 'Native Windows x64 required.' }
if ($env:VSCMD_ARG_TGT_ARCH -ne 'x64' -or -not $env:WindowsSDKVersion -or -not $env:VCToolsVersion) { throw 'Explicit x64 SDK/toolset environment required.' }
if ($env:CL -or $env:_CL_ -or $env:LINK) { throw 'Compiler/linker overrides rejected.' }
$compiler = (Get-Command cl.exe -CommandType Application).Source
$compilerHash = (Get-FileHash -LiteralPath $compiler -Algorithm SHA256).Hash
$scriptHash = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash
$head = & git -C $PSScriptRoot rev-parse HEAD
if ($LASTEXITCODE -ne 0 -or $head -notmatch '^[0-9a-f]{40}$') { throw 'Missing source SHA.' }
$dirty = & git -C $PSScriptRoot status --porcelain
if ($LASTEXITCODE -ne 0) { throw 'Missing source status.' }
$destination = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $destination) { throw 'Output directory must be new.' }
$null = New-Item -ItemType Directory -Path $destination
$records = @()
foreach ($name in @('provider-stall-fixture', 'provider-stall-probe')) {
    $source = Join-Path $PSScriptRoot "$name.cpp"
    $sourceHash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
    $binary = Join-Path $destination "$name.exe"
    $object = Join-Path $destination "$name.obj"
    & $compiler /nologo /std:c++20 /EHsc /W4 /WX /Od /DUNICODE /D_UNICODE /DWINVER=0x0A00 /D_WIN32_WINNT=0x0A00 $source "/Fo$object" "/Fe$binary" /link /WX /MACHINE:X64 user32.lib ole32.lib oleaut32.lib uuid.lib uiautomationcore.lib
    if ($LASTEXITCODE -ne 0) { throw "Compile/link failed: $name; partial output retained." }
    if ((Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash -ne $sourceHash) { throw 'Source changed during compilation.' }
    $records += [ordered]@{ source_name = "$name.cpp"; source_sha256 = $sourceHash; binary_name = "$name.exe"; binary_sha256 = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash }
}
foreach ($record in $records) {
    if ((Get-FileHash -LiteralPath (Join-Path $PSScriptRoot $record.source_name) -Algorithm SHA256).Hash -ne $record.source_sha256) { throw 'Source changed during build.' }
}
if ((Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash -ne $scriptHash -or (Get-FileHash -LiteralPath $compiler -Algorithm SHA256).Hash -ne $compilerHash) { throw 'Build inputs changed.' }
$os = Get-CimInstance Win32_OperatingSystem
$result = [ordered]@{ version = 1; scope = 'controlled-provider-stall-build-only'; source_sha = $head; worktree_dirty = [bool]$dirty; compiler_sha256 = $compilerHash; script_sha256 = $scriptHash; sdk_version = $env:WindowsSDKVersion; toolset_version = $env:VCToolsVersion; os_version = $os.Version; os_build = $os.BuildNumber; files = $records; native_execution = $false; product_admission = $false }
$json = $result | ConvertTo-Json -Depth 5
$json | Set-Content -LiteralPath (Join-Path $destination 'build-result.json') -Encoding utf8
Write-Output $json
