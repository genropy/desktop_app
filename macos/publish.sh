#!/bin/bash
# Upload dist/release to the public bucket genroapp-downloads of the Hetzner Object Storage. 1Password (item
# s3-meetlot, vault gpkube) gives the keys to the process through `op run`; they are never printed.
# The dmg also goes to GenroApp.dmg, the fixed download link. latest.json goes last: an installed app never reads
# a manifest whose archive is not there yet.
set -euo pipefail
cd "$(dirname "$0")/.."
release=dist/release
version=$(python3 -c 'import json; print(json.load(open("src-tauri/tauri.conf.json"))["version"])')
export AWS_ACCESS_KEY_ID=op://gpkube/s3-meetlot/access_key_id
export AWS_SECRET_ACCESS_KEY=op://gpkube/s3-meetlot/secret_access_key
export AWS_DEFAULT_REGION=fsn1
# Hetzner refuses the checksum headers that recent aws-cli versions add by default.
export AWS_REQUEST_CHECKSUM_CALCULATION=when_required AWS_RESPONSE_CHECKSUM_VALIDATION=when_required
bucket=s3://genroapp-downloads
put() {  # put <file> <key> <content-type> <cache-control>
    op run -- aws s3 cp --endpoint-url https://fsn1.your-objectstorage.com --acl public-read --only-show-errors \
        --content-type "$3" --cache-control "$4" "$1" "$bucket/$2"
}
put "$release/GenroApp-$version.app.tar.gz" "GenroApp-$version.app.tar.gz" application/gzip "public, max-age=31536000, immutable"
put "$release/GenroApp-$version.dmg" "GenroApp-$version.dmg" application/x-apple-diskimage "public, max-age=31536000, immutable"
put "$release/GenroApp-$version.dmg" GenroApp.dmg application/x-apple-diskimage "no-cache"
put "$release/latest.json" latest.json application/json "no-cache"
for key in latest.json GenroApp.dmg "GenroApp-$version.app.tar.gz"; do
    curl -fsSI "https://genroapp-downloads.fsn1.your-objectstorage.com/$key" | head -1
done
