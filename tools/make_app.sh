#!/bin/sh
# Builds the desktop app (release) and wraps it in a macOS application
# bundle that opens with a double click.
#
#   tools/make_app.sh [path/To.app]        (default: target/EdgeSlicerRS.app)
#
# macOS only: it uses sips, iconutil, plutil and codesign, which come with
# the system. Elsewhere run the app with `cargo run --release` in
# apps/desktop.
#
# The bundle holds the binary and nothing else: the page is built into it,
# and it finds the profiles in this source tree (reference/EdgeSlicer, by
# the path it was built from), so it works only while that tree is where it
# was. Its settings are the app's own (~/Library/Application Support/
# EdgeSlicerRS), as with `cargo run`. CFBundleVersion is the commit it was
# built from ("-dirty" after it when there were changes not committed).
set -eu

case "$(uname -s)" in
  Darwin) ;;
  *) echo "make_app.sh: macOS only" >&2; exit 1 ;;
esac

root=$(cd "$(dirname "$0")/.." && pwd)
desktop="$root/apps/desktop"
out=${1:-"$root/target/EdgeSlicerRS.app"}
case "$out" in
  *.app) ;;
  *) echo "make_app.sh: the bundle's name must end in .app: $out" >&2; exit 1 ;;
esac

# Homebrew's rustup, where cargo is not on the path already.
if ! command -v cargo >/dev/null 2>&1 && [ -x /opt/homebrew/opt/rustup/bin/cargo ]; then
  PATH="/opt/homebrew/opt/rustup/bin:$PATH"
fi

(cd "$desktop" && cargo build --release)
binary="$desktop/target/release/edge-desktop"

conf="$desktop/tauri.conf.json"
identifier=$(plutil -extract identifier raw -o - "$conf")
name=$(plutil -extract productName raw -o - "$conf")
version=$(plutil -extract version raw -o - "$conf")
commit=$(git -C "$root" rev-parse --short=12 HEAD)
if [ -n "$(git -C "$root" status --porcelain --untracked-files=no)" ]; then
  commit="$commit-dirty"
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# The icon: every size macOS asks for, from the one picture there is.
iconset="$work/$name.iconset"
mkdir "$iconset"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$desktop/icons/icon.png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" "$desktop/icons/icon.png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done

rm -rf "$out"
mkdir -p "$out/Contents/MacOS" "$out/Contents/Resources"
cp "$binary" "$out/Contents/MacOS/edge-desktop"
iconutil -c icns "$iconset" -o "$out/Contents/Resources/$name.icns"

cat > "$out/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleDisplayName</key><string>$name</string>
  <key>CFBundleExecutable</key><string>edge-desktop</string>
  <key>CFBundleIconFile</key><string>$name</string>
  <key>CFBundleIdentifier</key><string>$identifier</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>$name</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$commit</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.graphics-design</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF
plutil -lint "$out/Contents/Info.plist" >/dev/null

# Signed for this machine only (no identity), so that the system takes the
# bundle as one whole.
codesign --force --sign - "$out" >/dev/null 2>&1 || echo "make_app.sh: not signed (codesign failed); it still opens here" >&2

echo "$out ($identifier $version, $commit)"
