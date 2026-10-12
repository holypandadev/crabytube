#!/usr/bin/env sh
# Builds CrabyTube.app for macOS and zips it: universal binaries (arm64 and x86_64 joined with
# lipo), the two helper programs next to the executable, an Info.plist, the icon and an ad hoc
# signature. The app is not notarized and not built with the hardened runtime. The zip also holds
# the first-launch note, the license and the third-party notices.
#
#   packaging/macos/build-app.sh
#
# Needs both Rust targets (rustup target add aarch64-apple-darwin x86_64-apple-darwin) and
# Xcode's command line tools. The result is in target/macos/:
#   CrabyTube.app
#   CrabyTube-<version>-macos-universal.zip
#
# TARGETS overrides the Rust targets to build, for a quick local check on one architecture.
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
here="$root/packaging/macos"
out="$root/target/macos"
app="$out/CrabyTube.app"
app_id=dev.holypanda.CrabyTube
minimum_macos=11.0
targets=${TARGETS:-"aarch64-apple-darwin x86_64-apple-darwin"}
# The three programs of the workspace. The first is the application.
executable=CrabyTube
helpers="botguard-helper js-challenge-helper"

version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -n 1)
[ -n "$version" ] || { echo "Could not read the version from Cargo.toml." >&2; exit 1; }

for target in $targets; do
    if ! rustup target list --installed | grep -qx "$target"; then
        echo "The Rust target $target is not installed. Install it with:" >&2
        echo "  rustup target add $target" >&2
        exit 1
    fi
done

# Every binary, and the C and C++ code built into them, targets the same macOS.
export MACOSX_DEPLOYMENT_TARGET=$minimum_macos

for target in $targets; do
    echo "Building for $target..."
    (cd "$root" && cargo build --release --locked --workspace --target "$target")
done

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"

# Joins the release binary of every target into one universal file in the bundle.
join_binary() {
    source_name=$1
    bundle_name=$2
    inputs=""
    for target in $targets; do
        inputs="$inputs $root/target/$target/release/$source_name"
    done
    # shellcheck disable=SC2086
    lipo -create -output "$app/Contents/MacOS/$bundle_name" $inputs
}

join_binary crabytube "$executable"
for helper in $helpers; do
    join_binary "$helper" "$helper"
done

cp "$here/crabytube.icns" "$app/Contents/Resources/crabytube.icns"

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleExecutable</key>
    <string>$executable</string>
    <key>CFBundleIconFile</key>
    <string>crabytube</string>
    <key>CFBundleIdentifier</key>
    <string>$app_id</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>CrabyTube</string>
    <key>CFBundleDisplayName</key>
    <string>CrabyTube</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>$version</string>
    <key>CFBundleVersion</key>
    <string>$version</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.music</string>
    <key>LSMinimumSystemVersion</key>
    <string>$minimum_macos</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSPrincipalClass</key>
    <string>NSApplication</string>
</dict>
</plist>
EOF
plutil -lint "$app/Contents/Info.plist" >/dev/null

# Ad hoc signature (no identity, no certificate): the helpers first, then the bundle, which seals
# them. The hardened runtime is left off.
for helper in $helpers; do
    codesign --force --sign - --identifier "$app_id.$helper" "$app/Contents/MacOS/$helper"
done
codesign --force --sign - --identifier "$app_id" "$app"
codesign --verify --strict --verbose=2 "$app"

# What goes into the zip, at its top level: the app and the files a user reads first. The notices
# are docs/licenses.md; its links to files of the repository point to the repository on GitHub,
# because those files are not in the zip, and the one to the license points to the copy beside it.
stage="$out/zip-contents"
rm -rf "$stage"
mkdir -p "$stage"
ditto "$app" "$stage/CrabyTube.app"
cp "$here/README-first-launch.txt" "$stage/README-first-launch.txt"
cp "$root/LICENSE" "$stage/LICENSE.txt"
sed -e 's#](\.\./LICENSE)#](LICENSE.txt)#' \
    -e 's#](\.\./#](https://github.com/holypandadev/crabytube/blob/main/#' \
    "$root/docs/licenses.md" > "$stage/THIRD-PARTY-LICENSES.md"

zip="$out/CrabyTube-$version-macos-universal.zip"
rm -f "$zip"
ditto -c -k --norsrc --noextattr --noacl "$stage" "$zip"

echo
echo "Architectures:"
for program in "$executable" $helpers; do
    printf '  %s: %s\n' "$program" "$(lipo -archs "$app/Contents/MacOS/$program")"
done
echo "In the zip:"
unzip -Z1 "$zip" | grep -v '/.' | sed 's/^/  /'
echo "App: $app"
echo "Zip: $zip"
echo "SHA-256: $(shasum -a 256 "$zip" | cut -d ' ' -f 1)"
