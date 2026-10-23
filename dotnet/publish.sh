#!/usr/bin/env bash
# Publishes the FaceBridge module host (framework-dependent, linux-x64) to dotnet/artifacts/module-host/.
# Linux counterpart of publish.ps1: same output layout, FaceBridge.ModuleHost (no extension) plus the
# VRCFaceTracking.Core / VRCFaceTracking.SDK compat assemblies and FaceBridge.Sdk.dll next to it.
# The target machine needs the .NET 10 runtime (DOTNET_ROOT must point at it without a system-wide install).
#
# dotnet: $FACEBRIDGE_DOTNET, then the .NET 10 SDK under ${XDG_DATA_HOME:-~/.local/share}/dotnet10, then `dotnet`
# on PATH. Configuration: first argument, default Release.
set -euo pipefail

configuration="${1:-Release}"

dotnet="${FACEBRIDGE_DOTNET:-}"
if [[ -z "$dotnet" ]]; then
    local_sdk="${XDG_DATA_HOME:-$HOME/.local/share}/dotnet10/dotnet"
    if [[ -x "$local_sdk" ]]; then dotnet="$local_sdk"; else dotnet=dotnet; fi
fi

root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project="$root/FaceBridge.ModuleHost/FaceBridge.ModuleHost.csproj"
out="$root/artifacts/module-host"

rm -rf "$out"

"$dotnet" publish "$project" \
    --configuration "$configuration" \
    --runtime linux-x64 \
    --self-contained false \
    --output "$out" \
    -p:DebugType=embedded \
    -nologo

exe="$out/FaceBridge.ModuleHost"
[[ -x "$exe" ]] || { echo "Publish finished but $exe is missing." >&2; exit 1; }
for name in VRCFaceTracking.Core.dll VRCFaceTracking.SDK.dll FaceBridge.Sdk.dll; do
    [[ -f "$out/$name" ]] || { echo "Publish output is missing $name." >&2; exit 1; }
done

echo "Published: $exe"
