#!/bin/sh
# Cargo `runner` for local macOS dev builds.
#
# `npm run tauri dev` rebuilds the binary on every change, and an unsigned binary gets a new code
# identity each rebuild, so the macOS Keychain never remembers your "Always Allow" and re-prompts
# for token access. Signing the binary with a STABLE local identity before running it fixes that:
# approve the prompt once with "Always Allow" and it sticks across rebuilds.
#
# One-time setup:
#   1. Create a self-signed code-signing certificate named "CIMon Dev" (see CONTRIBUTING.md).
#   2. Run the app once and approve the Keychain prompt with "Always Allow".
# Override the identity name with CIMON_SIGN_IDENTITY if you named the certificate differently.
#
# Safe by default: if the certificate is missing, or this is a test/bench binary rather than the
# app, the binary runs unsigned exactly as before. Non-macOS targets never use this runner.
set -eu

bin="$1"
shift

# Only the app binary needs a stable identity for the Keychain; leave test/bench binaries alone.
case "${bin##*/}" in
  cimon | cimon.exe)
    identity="${CIMON_SIGN_IDENTITY:-CIMon Dev}"
    # `find-identity` WITHOUT `-v`: a self-signed "CIMon Dev" certificate is untrusted, so `-v`
    # (valid identities only) would not list it and the binary would silently run unsigned.
    # codesign signs with an untrusted identity fine, and a stable signature is all the Keychain
    # needs for "Always Allow" to persist.
    if security find-identity -p codesigning 2>/dev/null | grep -qF "$identity"; then
      codesign --force --sign "$identity" --identifier io.github.fuitad.cimon "$bin" >/dev/null 2>&1 ||
        echo "dev-codesign: signing with '$identity' failed; running unsigned" >&2
    fi
    ;;
esac

# Test hook: when set, report what would run and exit without launching (used to verify wiring).
if [ -n "${CIMON_SIGN_DRYRUN:-}" ]; then
  printf 'dev-codesign: ran for %s (cwd=%s)\n' "$bin" "$PWD" >&2
  exit 0
fi

exec "$bin" "$@"
