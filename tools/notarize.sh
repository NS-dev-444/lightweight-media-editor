#!/usr/bin/env bash
# Sign, notarize and staple Editor.app, then verify a fresh Mac would open it.
#
# Notarization is not signing. A Developer ID signature says who built the app;
# notarization is Apple having scanned it and issued a ticket. Without the
# ticket, Gatekeeper on any machine but this one shows "cannot be opened because
# Apple cannot check it for malicious software" — the signature alone is not
# enough, which is the thing most people discover from a user's screenshot.
#
# CREDENTIALS ARE NEVER STORED HERE. They come from the environment, and this
# script is committed — so nothing secret may appear in it:
#
#   DEVELOPER_ID   e.g. "Developer ID Application: NAME (TEAMID)"
#   ASC_KEY        path to the App Store Connect .p8
#   ASC_KEY_ID     the key id (the AuthKey_XXXX.p8 filename)
#   ASC_ISSUER     the issuer UUID from App Store Connect
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# The release build lives in build/release, NOT in app/macos/Editor.app.
#
# They shared a path once, and it cost a notarization: `tools/check.sh` rebuilds
# the app, and without DEVELOPER_ID in that shell it re-signed the bundle ad-hoc
# — silently replacing a Developer-ID-signed, freshly-notarized bundle with one
# Gatekeeper rejects. The ticket was still valid; the app it belonged to was
# gone.
#
# A release artefact that any routine command can overwrite is not an artefact.
DEV_APP="$ROOT/app/macos/Editor.app"
OUT="$ROOT/build/release"
APP="$OUT/Editor.app"

need() { [[ -n "${!1:-}" ]] || { echo "ERROR: $1 is not set" >&2; exit 2; }; }
need DEVELOPER_ID; need ASC_KEY; need ASC_KEY_ID; need ASC_ISSUER
[[ -f "$ASC_KEY" ]] || { echo "ERROR: no key at $ASC_KEY" >&2; exit 2; }

step() { printf '\033[1;34m==>\033[0m %s\n' "$1"; }

step "Building signed"
DEVELOPER_ID="$DEVELOPER_ID" "$ROOT/app/macos/build_app.sh" >/dev/null

# Move it out of the working directory before anything else can touch it.
mkdir -p "$OUT"
rm -rf "$APP"
/usr/bin/ditto "$DEV_APP" "$APP"

# Confirm before uploading. Submitting an unsigned or ad-hoc-signed app wastes a
# round trip to Apple and comes back with a rejection that says less than this.
team=$(codesign -dv "$APP" 2>&1 | sed -n 's/^TeamIdentifier=//p')
flags=$(codesign -dv "$APP" 2>&1 | sed -n 's/.*flags=\([^ ]*\).*/\1/p')
[[ -n "$team" && "$team" != "not set" ]] || { echo "ERROR: no Team ID — not Developer ID signed" >&2; exit 1; }
[[ "$flags" == *runtime* ]] || { echo "ERROR: hardened runtime is off; notarization will reject it" >&2; exit 1; }
step "Signed by $team, hardened runtime on"

# ditto, not zip: `zip` mangles symlinks and extended attributes inside a
# bundle, and the notary service rejects the result.
mkdir -p "$OUT"
ZIP="$OUT/Editor.zip"
rm -f "$ZIP"
step "Packaging"
/usr/bin/ditto -c -k --keepParent --sequesterRsrc "$APP" "$ZIP"
printf '    %s\n' "$(du -h "$ZIP" | cut -f1)"

step "Submitting to Apple (this takes a few minutes)"
if ! xcrun notarytool submit "$ZIP" --wait \
      --key "$ASC_KEY" --key-id "$ASC_KEY_ID" --issuer "$ASC_ISSUER" \
      2>&1 | tee "$OUT/notarize.log"; then
  echo "ERROR: submission failed — see $OUT/notarize.log" >&2
  exit 1
fi

id=$(sed -n 's/^ *id: *//p' "$OUT/notarize.log" | head -1)
status=$(sed -n 's/^ *status: *//p' "$OUT/notarize.log" | tail -1)
if [[ "$status" != "Accepted" ]]; then
  echo "ERROR: notarization $status" >&2
  # The log is where the actual reason lives; a bare "Invalid" says nothing.
  [[ -n "$id" ]] && xcrun notarytool log "$id" \
    --key "$ASC_KEY" --key-id "$ASC_KEY_ID" --issuer "$ASC_ISSUER" 2>&1 | head -40
  exit 1
fi

# Staple the ticket INTO the app, so it validates without a network round trip.
# An un-stapled app fails to open on a machine that is offline or behind a
# firewall — which looks exactly like an unnotarized one to the user.
step "Stapling the ticket"
xcrun stapler staple "$APP"

step "Verifying as a fresh Mac would see it"
xcrun stapler validate "$APP"
spctl --assess --type execute --verbose=4 "$APP" 2>&1 | sed 's/^/    /'
codesign --verify --deep --strict --verbose=2 "$APP" 2>&1 | tail -2 | sed 's/^/    /'

# Re-zip AFTER stapling: the ticket is inside the bundle now, and the zip made
# before stapling does not contain it.
rm -f "$ZIP"
/usr/bin/ditto -c -k --keepParent --sequesterRsrc "$APP" "$ZIP"

step "Done"
printf '    app: %s\n' "$APP"
printf '    zip: %s  (%s)\n' "$ZIP" "$(du -h "$ZIP" | cut -f1)"
printf '    \033[2mapp/macos/Editor.app remains the DEV build and may be rebuilt freely.\033[0m\n' 
