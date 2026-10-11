#!/usr/bin/env sh
# Installs CrabyTube for the current user on Linux: the launcher entry and the
# app icon in every size. Wayland compositors take a window's icon from the
# .desktop file that matches its app id, so both files carry that id as their name.
#
#   scripts/install-linux.sh [path/to/crabytube]   install (default: target/release/crabytube)
#   scripts/install-linux.sh --uninstall            remove what this script installed
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
data=${XDG_DATA_HOME:-$HOME/.local/share}
app_id=dev.holypanda.CrabyTube
desktop="$data/applications/$app_id.desktop"
# Launcher and icon names used before the app id became the file name.
legacy_desktop="$data/applications/crabytube.desktop"

refresh() {
    command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$data/icons/hicolor" 2>/dev/null || true
    command -v update-desktop-database >/dev/null && update-desktop-database -q "$data/applications" 2>/dev/null || true
    command -v kbuildsycoca6 >/dev/null && kbuildsycoca6 --noincremental >/dev/null 2>&1 || true
}

remove_legacy() {
    rm -f "$legacy_desktop"
    for icon in "$data"/icons/hicolor/*/apps/crabytube.png; do
        rm -f "$icon"
    done
}

if [ "${1:-}" = "--uninstall" ]; then
    rm -f "$desktop"
    for icon in "$data"/icons/hicolor/*/apps/$app_id.png; do
        rm -f "$icon"
    done
    remove_legacy
    refresh
    echo "CrabyTube launcher and icons removed."
    exit 0
fi

binary=$(realpath "${1:-$root/target/release/crabytube}")
if [ ! -x "$binary" ]; then
    echo "No CrabyTube binary at $binary. Build it first: cargo build --release" >&2
    exit 1
fi

remove_legacy
for dir in "$root"/packaging/icons/hicolor/*; do
    size=$(basename "$dir")
    mkdir -p "$data/icons/hicolor/$size/apps"
    cp "$dir/apps/$app_id.png" "$data/icons/hicolor/$size/apps/$app_id.png"
done
mkdir -p "$data/applications"
sed "s|^Exec=.*|Exec=\"$binary\"|" "$root/packaging/linux/$app_id.desktop" > "$desktop"
refresh
echo "Installed: $desktop (Exec=$binary) and icons in $data/icons/hicolor."
