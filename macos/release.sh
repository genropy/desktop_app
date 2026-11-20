#!/bin/bash
# Release the app built by build-developer-id.sh: notarize and staple the app, make the notarized disk image,
# sign the updater archive and write latest.json in dist/release. publish.sh uploads them to the bucket.
#
#   GENROAPP_NOTARY_PROFILE  notarytool keychain profile of team HXS58UUB2N; default gp-notary-tool
#   GENROAPP_UPDATER_KEY     private key file of the updater; default ~/.tauri/genroapp-updater.key
#                            (1Password, vault "Comuni Softwell", document "GenroApp updater key")
#   GENROAPP_RELEASE_NOTES   optional: what changed, kept in latest.json (English)
set -euo pipefail
cd "$(dirname "$0")/.."
profile=${GENROAPP_NOTARY_PROFILE:-gp-notary-tool}
updater_key=${GENROAPP_UPDATER_KEY:-$HOME/.tauri/genroapp-updater.key}
base=https://genroapp-downloads.fsn1.your-objectstorage.com
team=HXS58UUB2N
identity=$(security find-identity -v -p codesigning | awk -v team="$team" 'index($0," \"Developer ID Application: ") && index($0,"(" team ")\"") {print $2; exit}')
[[ -n "$identity" ]] || { echo "Developer ID Application ($team) is required in the keychain." >&2; exit 1; }
app=src-tauri/target/universal-apple-darwin/release/bundle/macos/GenroApp.app
out=dist/release
version=$(python3 -c 'import json; print(json.load(open("src-tauri/tauri.conf.json"))["version"])')
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
rm -rf "$out"
mkdir -p "$out"

# 1. The app: notarize, staple.
ditto -c -k --keepParent --sequesterRsrc "$app" "$work/GenroApp.zip"
xcrun notarytool submit "$work/GenroApp.zip" --keychain-profile "$profile" --wait
xcrun stapler staple "$app"

# 2. The disk image: the app and a link to /Applications; signed, notarized, stapled.
mkdir "$work/image"
ditto "$app" "$work/image/GenroApp.app"
ln -s /Applications "$work/image/Applications"
hdiutil create -volname GenroApp -srcfolder "$work/image" -ov -format UDZO "$out/GenroApp-$version.dmg"
codesign --force --timestamp --sign "$identity" "$out/GenroApp-$version.dmg"
xcrun notarytool submit "$out/GenroApp-$version.dmg" --keychain-profile "$profile" --wait
xcrun stapler staple "$out/GenroApp-$version.dmg"

# 3. The updater archive of the stapled app, signed with the updater key.
archive=GenroApp-$version.app.tar.gz
COPYFILE_DISABLE=1 tar -czf "$out/$archive" -C "$(dirname "$app")" GenroApp.app
cargo tauri signer sign -f "$updater_key" -p "" "$out/$archive"

# 4. latest.json: the same universal archive serves Apple silicon and Intel. Versioned names keep an archive and
# its signature together while a cached latest.json is still around.
python3 - "$out" "$version" "$base/$archive" "${GENROAPP_RELEASE_NOTES:-}" <<'PY'
import datetime, json, pathlib, sys
out, version, url, notes = pathlib.Path(sys.argv[1]), sys.argv[2], sys.argv[3], sys.argv[4]
entry = {"signature": (out / (url.rsplit("/", 1)[1] + ".sig")).read_text().strip(), "url": url}
manifest = {"version": version, "notes": notes,
            "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
            "platforms": {"darwin-aarch64": entry, "darwin-x86_64": entry}}
(out / "latest.json").write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n")
PY
rm "$out/$archive.sig"

spctl -a -t exec -vv "$app"
spctl -a -t open --context context:primary-signature -vv "$out/GenroApp-$version.dmg"
echo "Publish with macos/publish.sh"
