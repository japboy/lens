param([Parameter(Mandatory)][string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
$env:PATHEXT = '.COM;.EXE;.BAT;.CMD'
$vswhere = Join-Path "${env:ProgramFiles(x86)}" 'Microsoft Visual Studio/Installer/vswhere.exe'
$vs = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (!$vs) { throw 'Existing MSVC required' }
& "$vs/Common7/Tools/Launch-VsDevShell.ps1" -Arch amd64 -HostArch amd64 -SkipAutomaticLocation
$out = New-Item -ItemType Directory -Path $OutputDirectory
foreach ($name in 'fixture', 'worker', 'selector') {
    $libraries = @('user32.lib', 'gdi32.lib')
    if ($name -eq 'worker') {
        $libraries += @('ole32.lib','oleaut32.lib','runtimeobject.lib','windowsapp.lib','d3d11.lib','dxgi.lib','uuid.lib','windowscodecs.lib','dwmapi.lib')
    }
    & cl.exe /nologo /std:c++20 /EHsc /W4 /WX /DUNICODE /D_UNICODE "$PSScriptRoot/$name.cpp" "/Fo$($out.FullName)/$name.obj" "/Fe$($out.FullName)/$name.exe" /link @libraries
    if ($LASTEXITCODE) { throw "Build failed: $name" }
}
