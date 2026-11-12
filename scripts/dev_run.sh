#!/bin/sh
# Cargo runner for start_dev.sh: signs the fresh dev build with a stable certificate before it
# starts. macOS ties "Always Allow" for worki's Keychain item to the app's signature, and an
# unsigned build counts as a new app after every rebuild, so it would ask on every start.
set -eu

identity=${WORKI_DEV_SIGNING_IDENTITY:-worki dev}
if security find-certificate -c "$identity" >/dev/null 2>&1; then
  codesign --force --sign "$identity" --identifier io.fibe.worki.dev "$1" 2>/dev/null ||
    echo "worki: couldn't sign the dev build with \"$identity\"; macOS will ask for Keychain access." >&2
fi
exec "$@"
