#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"
if [[ -x "SLIM.app/Contents/MacOS/slim" ]]; then
    slim_bin="$PWD/SLIM.app/Contents/MacOS/slim"
elif [[ -x "$HOME/Applications/SLIM.app/Contents/MacOS/slim" ]]; then
    slim_bin="$HOME/Applications/SLIM.app/Contents/MacOS/slim"
elif [[ -x "target/release/slim" ]]; then
    slim_bin="$PWD/target/release/slim"
else
    echo "Install SLIM first using Install SLIM.command."
    read -r -p "Press Return to close." _
    exit 1
fi
if [[ -f "my-inventory.db" ]]; then
    exec "$slim_bin" --db "$PWD/my-inventory.db" tui
fi
exec "$slim_bin" tui
