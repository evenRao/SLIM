#!/bin/bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

version="$(awk '
    { sub(/\r$/, "") }
    /^\[workspace.package\]$/ { in_package=1; next }
    /^\[/ { in_package=0 }
    in_package && /^version[[:space:]]*=/ {
        sub(/^[^"]*"/, "", $0)
        sub(/".*$/, "", $0)
        print
        exit
    }
' Cargo.toml)"
if [[ -z "$version" ]]; then
    echo "Could not read the workspace version from Cargo.toml." >&2
    exit 1
fi

for binary in target/release/slim-app target/release/slim; do
    if [[ ! -x "$binary" ]]; then
        echo "Missing $binary. Run cargo build --release --locked --package slim-cli --bins first." >&2
        exit 1
    fi
done

arch="$(uname -m)"
case "$arch" in
    arm64|x86_64) ;;
    *) echo "Unsupported macOS architecture: $arch" >&2; exit 1 ;;
esac

stage="$(mktemp -d /tmp/slim-macos-package.XXXXXX)"
trap 'rm -rf "$stage"' EXIT
app="$stage/SLIM.app"
release="$stage/SLIM-macOS-$arch"
archive="dist/SLIM-macOS-$arch.zip"
rm -rf "dist/SLIM.app" "dist/SLIM-macOS-$arch" "$archive"
mkdir -p dist
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/slim-app "$app/Contents/MacOS/SLIM"
cp target/release/slim "$app/Contents/MacOS/slim"
chmod +x "$app/Contents/MacOS/SLIM" "$app/Contents/MacOS/slim"
sed "s/__SLIM_VERSION__/$version/g" > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>SLIM</string>
<key>CFBundleDisplayName</key><string>SLIM</string>
<key>CFBundleIdentifier</key><string>com.slim.inventory</string>
<key>CFBundleExecutable</key><string>SLIM</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>__SLIM_VERSION__</string>
<key>CFBundleVersion</key><string>__SLIM_VERSION__</string>
<key>LSMinimumSystemVersion</key><string>11.0</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
/usr/bin/plutil -lint "$app/Contents/Info.plist"
# Ad-hoc signing makes the bundle structurally valid. Public distribution should
# use an Apple Developer ID and notarization in the release environment.
/usr/bin/xattr -cr "$app"
/usr/bin/codesign --force --deep --sign - "$app"
/usr/bin/codesign --verify --deep --strict "$app"
mkdir -p "$release"
/usr/bin/ditto "$app" "$release/SLIM.app"
cp 'Install SLIM.command' 'Start SLIM.command' README.md LICENSE "$release/"
chmod +x "$release/Install SLIM.command" "$release/Start SLIM.command"
/usr/bin/xattr -cr "$release"
/usr/bin/ditto -c -k --sequesterRsrc --keepParent "$release" "$archive"

# Keep unpacked copies for source-checkout installation and local inspection.
# The ZIP above is created from the clean temporary staging area so Desktop
# file-provider metadata cannot contaminate the distributed signature.
/usr/bin/ditto "$app" "dist/SLIM.app"
/usr/bin/ditto "$release" "dist/SLIM-macOS-$arch"

echo "Created $archive (SLIM $version for $arch)"
