#!/usr/bin/env bash
# Builds the GitHub release assets into dist/ on Linux (the Windows app is cross-built with cargo-xwin):
#   OSCFT-for-VRC-<version>-windows-x64.exe / -linux-x64                      app with the module host built in
#   OSCFT-for-VRC-<version>-windows-x64-no-module-host.exe / -linux-x64-no-module-host    app only
#   module-host-win-x64.zip / module-host-linux-x64.zip                        the module host alone
# The host is built into the app at compile time from FACEBRIDGE_BUNDLED_HOST_ZIP (see crates/fb-core/build.rs).
# Usage: scripts/release.sh [linux-x64] [win-x64]   (no arguments: both platforms)
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dist="$root/dist"
version="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -n1)"
name="OSCFT-for-VRC-$version"

# dotnet/artifacts/module-host/ -> $1, inside a top-level module-host/ folder (Python: `zip` is not always installed).
zip_host() {
    python3 - "$root/dotnet/artifacts/module-host" "$1" <<'EOF'
import os, sys, zipfile
src, out = sys.argv[1], sys.argv[2]
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
    for f in sorted(os.listdir(src)):
        z.write(os.path.join(src, f), "module-host/" + f)
EOF
}

rids=("$@")
if [[ ${#rids[@]} -eq 0 ]]; then rids=(linux-x64 win-x64); fi
for rid in "${rids[@]}"; do
    if [[ "$rid" != linux-x64 && "$rid" != win-x64 ]]; then
        echo "unknown platform '$rid' (expected linux-x64 or win-x64)" >&2
        exit 2
    fi
done

rm -rf "$dist"
mkdir -p "$dist"
cd "$root"

for rid in "${rids[@]}"; do
    case "$rid" in
        linux-x64)
            os=linux-x64 ext=""
            build=(cargo build --release -p facebridge)
            bin="target/release/facebridge"
            ;;
        win-x64)
            os=windows-x64 ext=".exe"
            build=(cargo xwin build --release -p facebridge --target x86_64-pc-windows-msvc)
            bin="target/x86_64-pc-windows-msvc/release/facebridge.exe"
            ;;
    esac

    dotnet/publish.sh Release "$rid"
    zip="$dist/module-host-$rid.zip"
    zip_host "$zip"

    env -u FACEBRIDGE_BUNDLED_HOST_ZIP "${build[@]}"
    cp "$bin" "$dist/$name-$os-no-module-host$ext"

    FACEBRIDGE_BUNDLED_HOST_ZIP="$zip" "${build[@]}"
    cp "$bin" "$dist/$name-$os$ext"
    if [[ -z "$ext" ]]; then chmod +x "$dist/$name-$os" "$dist/$name-$os-no-module-host"; fi
done

ls -l "$dist"
