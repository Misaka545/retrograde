# Builds the installer: publishes a Release build, then packs it with NSIS.
#   powershell -ExecutionPolicy Bypass -File installer\build.ps1 [-Version 2.0.0]
# Output: native\release\Retrograde Setup <version>.exe
# makensis is taken from $env:MAKENSIS, the PATH, an NSIS install, or electron-builder's cache.
param([string]$Version = "2.0.0")
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
$out = Join-Path $root "out"
$release = Join-Path $root "release"

$makensis = $env:MAKENSIS
if (-not $makensis) { $makensis = (Get-Command makensis -ErrorAction SilentlyContinue).Source }
if (-not $makensis) {
  $makensis = @(
    "${env:ProgramFiles(x86)}\NSIS\makensis.exe",
    "$env:ProgramFiles\NSIS\makensis.exe"
  ) + @(Get-ChildItem "$env:LOCALAPPDATA\electron-builder\Cache\nsis" -Recurse -Filter makensis.exe -ErrorAction SilentlyContinue |
        Where-Object { $_.Directory.Name -ne "Bin" } | ForEach-Object FullName) |
    Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
}
if (-not $makensis) { throw "makensis.exe not found: install NSIS or set MAKENSIS" }

if (Test-Path $out) { Remove-Item -Recurse -Force $out }
dotnet publish (Join-Path $root "src\Retrograde.App") -c Release -o $out "-p:Version=$Version" -nologo -v q
if ($LASTEXITCODE -ne 0) { throw "dotnet publish failed" }
if (-not (Test-Path (Join-Path $out "bin\mpv.exe"))) { throw "bin\mpv.exe missing from the publish output" }

New-Item -ItemType Directory -Force $release | Out-Null
$setup = Join-Path $release "Retrograde Setup $Version.exe"
& $makensis /V2 "/DVERSION=$Version" "/DSRC=$out" "/DOUTFILE=$setup" (Join-Path $PSScriptRoot "Retrograde.nsi")
if ($LASTEXITCODE -ne 0) { throw "makensis failed" }
Get-Item $setup | Select-Object FullName, @{ n = "MB"; e = { [math]::Round($_.Length / 1MB, 1) } }
