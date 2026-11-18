#!/usr/bin/env bash
# The launcher's tests, plus the opt-in ones against the real Claude Code
# installed on each machine, on this Mac and on a Parallels Windows VM.
#
# Run it on demand: before a release, after a change to connectors, the
# Windows run code or a tool writer, and after a big Claude Code update. It
# installs nothing: a missing tool stops it with a note saying what to add.
# It does not cover the launcher's window, the portal, Claude Desktop's UI or
# OAuth sign-in; those are the release checklist's.
#
#   scripts/real-tools-test.sh             # this Mac, then the Windows VM
#   scripts/real-tools-test.sh --mac
#   scripts/real-tools-test.sh --windows
#
# The Windows VM's settings come from scripts/real-tools.local, which git
# ignores (*.local):
#   VM_NAME="Windows 11"
#   VM_REPO='C:\Users\you\Desktop\consus-launcher'
# The VM needs Git, Node, Rust with the x86_64-pc-windows-msvc target (the
# ARM64 build needs clang for a dependency, x64 runs under emulation),
# Visual Studio's C++ build tools, and Claude Code. Its checkout is moved to
# this Mac's commit, which must be pushed.

set -uo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/.." && pwd)"
logs="$(mktemp -d -t consus-real-tools)"
results=()

note() { printf '%s\n' "$*"; }
stop() { note "Stopped: $*"; exit 1; }
record() { results+=("$1: $2"); }

# The test result lines of a cargo log, and whether every one passed.
summarize() {
  local log="$1" name="$2"
  grep -E '^test result:' "$log" | sed 's/^/  /'
  # A test that found a Claude Code lock says so and passes: say so here.
  grep -E '^SKIPPED' "$log" | sed 's/^/  /'
  if grep -q '^SKIPPED' "$log"; then
    record "$name" "SKIPPED (a Claude Code lock is on this machine; see above)"
    return
  fi
  if grep -qE '^test result: FAILED|^error(\[|:)' "$log" || ! grep -q '^test result:' "$log"; then
    record "$name" "FAILED (log: $log)"
    grep -E '^error|panicked|^---- ' "$log" | head -20 | sed 's/^/  /'
  else
    record "$name" "passed"
  fi
}

run_mac() {
  note "== This Mac"
  local cargo
  cargo="$(command -v cargo || rustup which cargo 2>/dev/null)" || stop "no cargo; install Rust (rustup)."
  command -v claude >/dev/null || stop "no claude on PATH; install Claude Code to run the real-tool tests."
  note "Claude Code $(claude --version 2>/dev/null) at $(command -v claude)"
  ( cd "$repo/src-tauri" && "$cargo" test ) >"$logs/mac-unit.log" 2>&1
  note "Unit tests:"; summarize "$logs/mac-unit.log" "Mac unit tests"
  ( cd "$repo/src-tauri" && "$cargo" test -- --ignored real_claude --test-threads=1 --nocapture ) >"$logs/mac-real.log" 2>&1
  note "Real Claude Code:"; summarize "$logs/mac-real.log" "Mac real Claude Code"
}

run_windows() {
  note "== Windows VM"
  [ -f "$here/real-tools.local" ] || stop "no scripts/real-tools.local; create it with VM_NAME and VM_REPO (see this script's header)."
  # shellcheck source=/dev/null
  . "$here/real-tools.local"
  [ -n "${VM_NAME:-}" ] && [ -n "${VM_REPO:-}" ] || stop "scripts/real-tools.local needs VM_NAME and VM_REPO."
  command -v prlctl >/dev/null || stop "no prlctl; this runs the VM through Parallels Desktop."
  prlctl status "$VM_NAME" 2>/dev/null | grep -q running || stop "the VM \"$VM_NAME\" is not running; start it and sign in."

  local sha
  sha="$(git -C "$repo" rev-parse HEAD)"
  git -C "$repo" fetch -q origin
  [ -n "$(git -C "$repo" branch -r --contains "$sha")" ] || stop "commit ${sha:0:7} is not pushed; the VM checks it out from GitHub."

  vm() { prlctl exec "$VM_NAME" --current-user cmd /c "$1"; }
  vm "rustup target list --installed" | grep -q x86_64-pc-windows-msvc \
    || stop "the VM's Rust has no x64 target; in the VM run: rustup target add x86_64-pc-windows-msvc"
  local claude_at
  claude_at="$(vm "where claude" 2>/dev/null | tr -d '\r' | head -1)"
  [ -n "$claude_at" ] || stop "no claude in the VM; install Claude Code there (npm or the native installer)."
  note "Claude Code $(vm "claude --version" 2>/dev/null | tr -d '\r') at $claude_at"
  case "$claude_at" in
    *.cmd) note "  npm install: the org's servers go through node.exe, the built-in ones through cmd.exe" ;;
    *) note "  native claude.exe: no shell in either run" ;;
  esac

  if ! vm "cd /d $VM_REPO && git fetch -q origin && git checkout -q --detach $sha" >"$logs/win-checkout.log" 2>&1; then
    sed 's/^/  /' "$logs/win-checkout.log"
    stop "the VM's checkout could not move to ${sha:0:7} (above). A PrlJob error is Parallels failing the call: run the script again."
  fi
  note "VM checkout at ${sha:0:7} (detached)"

  vm "cd /d $VM_REPO && npm.cmd ci && npm.cmd run build" >"$logs/win-ui.log" 2>&1 \
    && record "Windows UI build" "passed" \
    || record "Windows UI build" "FAILED (log: $logs/win-ui.log)"

  local target="--manifest-path src-tauri\\Cargo.toml --target x86_64-pc-windows-msvc"
  vm "cd /d $VM_REPO && cargo test $target 2>&1" >"$logs/win-unit.log" 2>&1
  note "Unit tests:"; summarize "$logs/win-unit.log" "Windows unit tests"
  vm "cd /d $VM_REPO && cargo test $target -- --ignored real_claude --test-threads=1 --nocapture 2>&1" >"$logs/win-real.log" 2>&1
  note "Real Claude Code:"; summarize "$logs/win-real.log" "Windows real Claude Code"
}

case "${1:-}" in
  --mac) run_mac ;;
  --windows) run_windows ;;
  "") run_mac; run_windows ;;
  *) stop "unknown option $1 (use --mac, --windows, or nothing for both)" ;;
esac

note ""
note "== Summary (logs in $logs)"
printf '  %s\n' "${results[@]}"
printf '%s\n' "${results[@]}" | grep -q FAILED && exit 1
exit 0
