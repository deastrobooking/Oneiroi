#!/bin/sh
# Local event build; uses the FFmpeg libraries installed on this Mac.
set -eu
project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_root"
if [ "$(uname -s)" != Darwin ]; then
    echo "This packaging command requires macOS." >&2
    exit 1
fi
cargo build --release --locked -p virtual-app
release_dir="$project_root/target/release"
staging=$(mktemp -d "$release_dir/virtual-bundle.XXXXXX")
trap 'rm -rf "$staging"' EXIT HUP INT TERM
bundle="$staging/VIRTUAL.app"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp "$release_dir/virtual" "$bundle/Contents/MacOS/virtual"
cp "$project_root/packaging/macos/Info.plist" "$bundle/Contents/Info.plist"
cp -R "$project_root/effects" "$bundle/Contents/Resources/effects"
cp "$project_root/LICENSE" "$bundle/Contents/Resources/LICENSE"
cp "$project_root/docs/EVENT_SETUP.md" "$bundle/Contents/Resources/EVENT_SETUP.md"
cp "$project_root/docs/VIDEO_INPUTS.md" "$bundle/Contents/Resources/VIDEO_INPUTS.md"
cp "$project_root/docs/RELEASE_CHECKLIST.md" "$bundle/Contents/Resources/RELEASE_CHECKLIST.md"
cp "$project_root/docs/SHADER_SYSTEM.md" "$bundle/Contents/Resources/SHADER_SYSTEM.md"
plutil -lint "$bundle/Contents/Info.plist"
# Ad-hoc signing makes a valid local bundle, not a notarized distribution.
codesign --force --sign - "$bundle"
codesign --verify --strict "$bundle"
otool -L "$bundle/Contents/MacOS/virtual" > "$staging/dynamic-libraries.txt"
# Only replace our generated bundle after building and validating its successor.
rm -rf "$release_dir/VIRTUAL.app"
mv "$bundle" "$release_dir/VIRTUAL.app"
mv "$staging/dynamic-libraries.txt" "$release_dir/VIRTUAL-dynamic-libraries.txt"
(cd "$release_dir" && shasum -a 256 VIRTUAL.app/Contents/MacOS/virtual > VIRTUAL.sha256)
echo "Built $release_dir/VIRTUAL.app"
echo "Local build: keep this Mac's Homebrew FFmpeg installation available."
