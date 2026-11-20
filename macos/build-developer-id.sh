#!/bin/bash
# Universal build of the site runner, signed Developer ID for direct distribution. Notarization is in release.sh.
# The feature `devices` (Python sidecar, card signing) is left out.
#
#   SOFTWELL_LINK_PUBLIC_KEY  base64 of the 32-byte Ed25519 public key that signs connection files;
#                             default ~/.tauri/genroapp-link.pub.b64
set -euo pipefail
cd "$(dirname "$0")/.."
team=HXS58UUB2N
identity=$(security find-identity -v -p codesigning | awk -v team="$team" 'index($0," \"Developer ID Application: ") && index($0,"(" team ")\"") {print $2; exit}')
[[ -n "$identity" ]] || { echo "Developer ID Application ($team) with its private key is required in the keychain." >&2; exit 1; }
if [[ -z "${SOFTWELL_LINK_PUBLIC_KEY:-}" ]]; then
    SOFTWELL_LINK_PUBLIC_KEY=$(tr -d '\n' < ~/.tauri/genroapp-link.pub.b64)
fi
export SOFTWELL_LINK_PUBLIC_KEY
# Camera and microphone (Meetlot windows) and the keychain group of the device keys.
entitlements=src-tauri/GenroApp.entitlements
cd src-tauri
cargo tauri build --target universal-apple-darwin --bundles app \
    --config '{"bundle":{"macOS":{"minimumSystemVersion":"10.15"}}}' -- --no-default-features
cd ..
app=src-tauri/target/universal-apple-darwin/release/bundle/macos/GenroApp.app
codesign --force --timestamp --options runtime --sign "$identity" --identifier it.softwell.genropy.runner \
    --entitlements "$entitlements" "$app"
codesign --verify --deep --strict --all-architectures "$app"
codesign --verify -R="anchor apple generic and certificate leaf[field.1.2.840.113635.100.6.1.13] exists and certificate leaf[subject.OU] = \"$team\"" "$app"
lipo -archs "$app/Contents/MacOS/genropy-runner"
echo "Built $app"
