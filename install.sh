#!/bin/sh
# Consus Launcher installer for macOS.
#
#   curl -fsSL https://raw.githubusercontent.com/consusindustries/consus-launcher/main/install.sh | sh
#
# Downloads a release from GitHub, checks the disk image against the release's
# SHA256SUMS.txt, and installs Consus Launcher.app into /Applications, or
# ~/Applications when /Applications is not writable. Nothing is installed
# unless the checksum matches.
#
# Options, as environment variables:
#   CONSUS_LAUNCHER_VERSION   a release tag, e.g. v0.1.0 (default: the latest release)
#   CONSUS_LAUNCHER_DIR       the folder to install into
#   CONSUS_LAUNCHER_BASE_URL  where the release files are, for an internal mirror
#                             (default: the GitHub release for that version)
set -eu

REPO="consusindustries/consus-launcher"
APP="Consus Launcher.app"

fail() {
  printf 'Consus Launcher install: %s\n' "$1" >&2
  exit 1
}

[ "$(uname -s)" = Darwin ] || fail "macOS only for now."
for c in curl shasum hdiutil ditto codesign; do
  command -v "$c" >/dev/null 2>&1 || fail "$c is missing."
done

version="${CONSUS_LAUNCHER_VERSION:-}"
if [ -z "$version" ]; then
  # GitHub redirects releases/latest to releases/tag/<tag>.
  latest=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest") ||
    fail "could not reach GitHub."
  version=${latest##*/}
fi
case "$version" in
  v[0-9]*) ;;
  *) fail "no published release found." ;;
esac
base="${CONSUS_LAUNCHER_BASE_URL:-https://github.com/$REPO/releases/download/$version}"
dmg="Consus-Launcher-${version#v}-universal.dmg"

tmp=$(mktemp -d)
mnt="$tmp/mnt"
cleanup() {
  hdiutil detach "$mnt" -quiet >/dev/null 2>&1 || true
  rm -rf "$tmp"
}
trap cleanup EXIT INT TERM

printf 'Downloading Consus Launcher %s...\n' "$version"
curl -fsSL -o "$tmp/$dmg" "$base/$dmg" || fail "could not download $dmg."
curl -fsSL -o "$tmp/SHA256SUMS.txt" "$base/SHA256SUMS.txt" || fail "could not download SHA256SUMS.txt."
expected=$(awk -v f="$dmg" '$2 == f { print $1 }' "$tmp/SHA256SUMS.txt")
[ -n "$expected" ] || fail "$dmg is not listed in SHA256SUMS.txt."
actual=$(shasum -a 256 "$tmp/$dmg" | awk '{ print $1 }')
[ "$expected" = "$actual" ] || fail "checksum mismatch, so nothing was installed."

mkdir -p "$mnt"
hdiutil attach -nobrowse -readonly -quiet -mountpoint "$mnt" "$tmp/$dmg" || fail "could not open the disk image."
[ -d "$mnt/$APP" ] || fail "the disk image does not contain $APP."

dest="${CONSUS_LAUNCHER_DIR:-}"
if [ -z "$dest" ]; then
  if [ -w /Applications ]; then dest=/Applications; else dest="$HOME/Applications"; fi
fi
mkdir -p "$dest" || fail "could not create $dest."
if pgrep -f "$dest/$APP/Contents/MacOS/" >/dev/null 2>&1; then
  fail "Consus Launcher is running. Quit it, then run this again."
fi

# Stage the new copy, then swap. Renames either fully happen or not at all,
# so an existing install is never left half-deleted: macOS can refuse to
# modify an app another program installed.
new="$dest/.consus-launcher-new.app"
old="$dest/.consus-launcher-old.app"
rm -rf "$new" "$old" 2>/dev/null || true
ditto "$mnt/$APP" "$new" || fail "could not copy the app into $dest."
codesign --verify --deep --strict "$new" 2>/dev/null || {
  rm -rf "$new"
  fail "the app failed its signature check, so nothing was installed."
}
if [ -e "$dest/$APP" ]; then
  mv "$dest/$APP" "$old" 2>/dev/null || {
    rm -rf "$new"
    fail "macOS would not let this replace $dest/$APP. Move it to the Trash in Finder, then run this again."
  }
fi
mv "$new" "$dest/$APP" || fail "could not move the app into place."
rm -rf "$old" 2>/dev/null || true

printf 'Installed Consus Launcher %s in %s.\n' "$version" "$dest"
open "$dest/$APP"
