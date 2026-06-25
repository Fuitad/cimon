#!/bin/sh
# Build CIMon as a packaged .app, install it into /Applications, and code-sign it with a stable
# local identity so macOS shows notification BANNERS (not just Notification Center entries).
#
# Why this exists: `npm run tauri build` ad-hoc signs the app, and macOS will not present banners
# for an ad-hoc signed app (the notifications still reach Notification Center, just without a
# banner). Signing the bundle with a stable identity, even an untrusted self-signed one, lets
# `usernoted` present banners. This is the packaged-build companion to scripts/dev-codesign.sh,
# which does the same for the `npm run tauri dev` binary.
#
# One-time setup: create a self-signed "CIMon Dev" code-signing certificate (see CONTRIBUTING.md).
# It does NOT need to be trusted; a stable signature is enough. Override the name with
# CIMON_SIGN_IDENTITY if you called the certificate something else.
#
# Local and macOS only: the identity lives only in your keychain, so this never affects CI or the
# app other people download. Run it with `npm run build:mac`.
set -eu

[ "$(uname)" = "Darwin" ] || { echo "build-local: macOS only" >&2; exit 1; }

identity="${CIMON_SIGN_IDENTITY:-CIMon Dev}"
app="CIMon.app"
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
bundle="$repo_root/src-tauri/target/release/bundle/macos/$app"
dest="/Applications/$app"

# Require the identity up front, before the slow build. `find-identity` WITHOUT `-v` matches an
# untrusted self-signed certificate too (the documented setup produces one); codesign signs with it
# regardless of trust.
if ! security find-identity -p codesigning 2>/dev/null | grep -qF "$identity"; then
  echo "build-local: code-signing identity '$identity' not found in your keychain." >&2
  echo "Create a self-signed 'CIMon Dev' certificate (see CONTRIBUTING.md), or set CIMON_SIGN_IDENTITY." >&2
  exit 1
fi

echo "build-local: building the release bundle"
( cd "$repo_root" && npm run tauri build )

# Quit a running copy so the bundle can be replaced cleanly.
if pgrep -f "$dest/Contents/MacOS" >/dev/null 2>&1; then
  echo "build-local: quitting the running CIMon"
  osascript -e 'quit app "CIMon"' >/dev/null 2>&1 || true
  sleep 2
fi

echo "build-local: installing into /Applications"
rm -rf "$dest"
ditto "$bundle" "$dest"

echo "build-local: signing with '$identity'"
codesign --force --deep --sign "$identity" "$dest"

echo "build-local: verifying the signature"
codesign -dvv "$dest" 2>&1 | grep -E 'Identifier=|Authority=' || true

echo "build-local: done. Launch /Applications/$app to pick up the new build."
