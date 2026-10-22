<#
.SYNOPSIS
  Publishes the FaceBridge module host (framework-dependent, win-x64) to dotnet/artifacts/module-host/.

.DESCRIPTION
  The output contains FaceBridge.ModuleHost.exe plus the VRCFaceTracking.Core / VRCFaceTracking.SDK compat
  assemblies and FaceBridge.Sdk.dll next to it. The target machine needs the .NET 10 runtime
  (DOTNET_ROOT must point at it when the host is launched without a machine-wide install).

.PARAMETER Dotnet
  Path to dotnet.exe. Defaults to $env:FACEBRIDGE_DOTNET, then the .NET 10 SDK under %LOCALAPPDATA%\dotnet10, then `dotnet` on PATH.
#>
[CmdletBinding()]
param(
    [string]$Dotnet,
    [string]$Configuration = 'Release'
)

$ErrorActionPreference = 'Stop'

if (-not $Dotnet) { $Dotnet = $env:FACEBRIDGE_DOTNET }
if (-not $Dotnet) {
    $local = Join-Path $env:LOCALAPPDATA 'dotnet10\dotnet.exe'
    if (Test-Path $local) { $Dotnet = $local } else { $Dotnet = 'dotnet' }
}

$root = $PSScriptRoot
$project = Join-Path $root 'FaceBridge.ModuleHost\FaceBridge.ModuleHost.csproj'
$out = Join-Path $root 'artifacts\module-host'

if (Test-Path $out) { Remove-Item -Recurse -Force $out }

& $Dotnet publish $project `
    --configuration $Configuration `
    --runtime win-x64 `
    --self-contained false `
    --output $out `
    -p:DebugType=embedded `
    -nologo
if ($LASTEXITCODE -ne 0) { throw "dotnet publish failed with exit code $LASTEXITCODE" }

$exe = Join-Path $out 'FaceBridge.ModuleHost.exe'
if (-not (Test-Path $exe)) { throw "Publish finished but $exe is missing." }
foreach ($name in 'VRCFaceTracking.Core.dll', 'VRCFaceTracking.SDK.dll', 'FaceBridge.Sdk.dll') {
    if (-not (Test-Path (Join-Path $out $name))) { throw "Publish output is missing $name." }
}

Write-Host "Published: $exe"
