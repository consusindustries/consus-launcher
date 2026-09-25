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
# The checksum catches a corrupted or mismatched download. It does not by
# itself prove who built the release: whoever can change the release can
# change both files. Once releases are Developer ID signed, this script will
# also require Consus's signing identity.
#
# Options, as environment variables:
#   CONSUS_LAUNCHER_VERSION   a release tag such as v0.1.0 (default: the latest release)
#   CONSUS_LAUNCHER_DIR       the folder to install into
#   CONSUS_LAUNCHER_BASE_URL  where the release files are, for an internal mirror;
#                             set CONSUS_LAUNCHER_VERSION too, or GitHub is asked
#                             for the latest version
set -eu

REPO="consusindustries/consus-launcher"
APP="Consus Launcher.app"

tmp=""
mnt=""
dest=""
stage=""

fail() {
  printf 'Consus Launcher install: %s\n' "$1" >&2
  exit 1
}

cleanup() {
  if [ -n "$mnt" ]; then
    hdiutil detach "$mnt" -force -quiet >/dev/null 2>&1 || true
  fi
  if [ -n "$stage" ] && [ -d "$stage" ]; then
    if [ -e "$stage/previous.app" ] && [ ! -e "$dest/$APP" ]; then
      # Never delete the only copy of the app.
      printf 'Consus Launcher install: your previous copy is at %s/previous.app\n' "$stage" >&2
    elif ! rm -rf "$stage" 2>/dev/null; then
      printf 'Consus Launcher install: could not remove %s; you can delete it.\n' "$stage" >&2
    fi
  fi
  if [ -n "$tmp" ]; then
    rm -rf "$tmp"
  fi
}

# Everything runs from here, so a download cut short cannot run half a script.
main() {
  [ "$(uname -s)" = Darwin ] || fail "macOS only for now."
  for c in curl shasum hdiutil ditto codesign; do
    command -v "$c" >/dev/null 2>&1 || fail "$c is missing."
  done

  version="${CONSUS_LAUNCHER_VERSION:-}"
  if [ -n "$version" ]; then
    case "$version" in
      v[0-9]*) ;;
      *) fail "CONSUS_LAUNCHER_VERSION must be a release tag such as v0.1.0." ;;
    esac
  else
    # GitHub redirects releases/latest to releases/tag/<tag>.
    latest=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest") ||
      fail "could not reach GitHub."
    version=${latest##*/}
    case "$version" in
      v[0-9]*) ;;
      *) fail "no published release found." ;;
    esac
  fi
  base="${CONSUS_LAUNCHER_BASE_URL:-https://github.com/$REPO/releases/download/$version}"
  dmg="Consus-Launcher-${version#v}-universal.dmg"

  tmp=$(mktemp -d)
  mnt="$tmp/mnt"
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
  dest=${dest%/}
  mkdir -p "$dest" || fail "could not create $dest."
  if ps -axo comm= | grep -Fqx "$dest/$APP/Contents/MacOS/consus-launcher"; then
    fail "Consus Launcher is running. Quit it, then run this again."
  fi

  # Stage in a fresh private folder next to the app, so the swap is two
  # renames on one disk and no name can be planted in advance. An existing
  # copy is renamed aside, never deleted in place: macOS can refuse to modify
  # an app another program installed.
  stage=$(mktemp -d "$dest/.consus-launcher-install.XXXXXX") || fail "could not write to $dest."
  ditto "$mnt/$APP" "$stage/$APP" || fail "could not copy the app into $dest."
  codesign --verify --deep --strict "$stage/$APP" 2>/dev/null ||
    fail "the app failed its signature check, so nothing was installed."
  if [ -e "$dest/$APP" ] || [ -L "$dest/$APP" ]; then
    mv "$dest/$APP" "$stage/previous.app" ||
      fail "macOS would not let this replace $dest/$APP (see the error above). Move it to the Trash in Finder, then run this again."
  fi
  if ! mv "$stage/$APP" "$dest/$APP"; then
    if [ -e "$stage/previous.app" ] && mv "$stage/previous.app" "$dest/$APP"; then
      fail "could not move the new app into place, so the previous copy was put back."
    fi
    fail "could not move the new app into place."
  fi

  printf 'Installed Consus Launcher %s in %s.\n' "$version" "$dest"
  open "$dest/$APP"
}

main "$@"
