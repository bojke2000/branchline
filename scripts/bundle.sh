#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
if [ "$(uname -s)" != Darwin ]; then
    printf '%s\n' 'The app-bundle script requires macOS.' >&2
    exit 1
fi
cargo build --release --locked
app=target/bundle/BMM.app
marker="$app/.branchline-local-build"
if [ -e "$app" ] && { [ ! -f "$marker" ] || [ "$(cat "$marker")" != branchline-local-build-v1 ]; }; then
    printf '%s\n' "Refusing to replace an unowned bundle: $app" >&2
    exit 1
fi
mkdir -p "$app/Contents/MacOS"
printf '%s\n' branchline-local-build-v1 > "$marker"
install -m 755 target/release/branchline "$app/Contents/MacOS/BMM"
cp packaging/Info.plist "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist"
printf 'Bundle ready: %s/%s\n' "$(pwd)" "$app"
