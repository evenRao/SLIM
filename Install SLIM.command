#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"
if [[ -d "SLIM.app" ]]; then
    app_arches="$(/usr/bin/lipo -archs "SLIM.app/Contents/MacOS/SLIM" 2>/dev/null || true)"
    machine_arch="$(uname -m)"
    if [[ " $app_arches " != *" $machine_arch "* ]]; then
        echo "This SLIM build supports '${app_arches:-unknown}', but this Mac requires '$machine_arch'."
        echo "Download the matching macOS release or build SLIM from source on this Mac."
        read -r -p "Press Return to close." _
        exit 1
    fi
    mkdir -p "$HOME/Applications"
    /usr/bin/ditto "SLIM.app" "$HOME/Applications/SLIM.app"
    /usr/bin/xattr -cr "$HOME/Applications/SLIM.app"
else
    cargo_bin="${CARGO_HOME:-$HOME/.cargo}/bin/cargo"
    if command -v cargo >/dev/null 2>&1; then
        cargo_bin="$(command -v cargo)"
    elif [[ -x "/opt/homebrew/opt/rustup/bin/cargo" ]]; then
        cargo_bin="/opt/homebrew/opt/rustup/bin/cargo"
    elif [[ -x "/usr/local/opt/rustup/bin/cargo" ]]; then
        cargo_bin="/usr/local/opt/rustup/bin/cargo"
    fi
    if [[ ! -x "$cargo_bin" ]]; then
        echo "Install Rust 1.89+ and Xcode command line tools to build this source checkout, or use the prebuilt SLIM macOS release."
        read -r -p "Press Return to close." _
        exit 1
    fi
    "$cargo_bin" build --release --locked --package slim-cli --bins
    /bin/bash scripts/package-macos.sh
    mkdir -p "$HOME/Applications"
    /usr/bin/ditto "dist/SLIM.app" "$HOME/Applications/SLIM.app"
    /usr/bin/xattr -cr "$HOME/Applications/SLIM.app"
fi
cp 'Start SLIM.command' "$HOME/Applications/Start SLIM.command"
chmod +x "$HOME/Applications/Start SLIM.command"
exec /bin/bash "$HOME/Applications/Start SLIM.command"
