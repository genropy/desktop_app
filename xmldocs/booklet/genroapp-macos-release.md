<!-- generated from genroapp-macos-release.xml by xmldocs: do not edit, edit the XML -->

## Release GenroApp for macOS: build, sign, notarize, publish
*Instruction booklet · booklet/genroapp-macos-release · 2026-11-20*

### Purpose

Produce a GenroApp build for macOS that another Mac opens without a Gatekeeper refusal: a universal app signed with the Developer ID certificate of team `HXS58UUB2N`, notarized by Apple, with the ticket stapled, published as a disk image and as an update for the installed apps.

The distributed build is the site runner only. It is compiled without the Cargo feature `devices`: no Python sidecar, no card signing, no PIN window.

### Requirements

- Rust with the targets `aarch64-apple-darwin` and `x86_64-apple-darwin`, Tauri CLI 2, Xcode command line tools.
- The certificate `Developer ID Application` of team `HXS58UUB2N`, with its private key, in the keychain.
- The notarytool keychain profile `gp-notary-tool` of team `HXS58UUB2N`. It is created once per Mac with `xcrun notarytool store-credentials gp-notary-tool --team-id HXS58UUB2N`.
- The private key of the updater in `~/.tauri/genroapp-updater.key`, with no password. It is kept in 1Password, vault "Comuni Softwell", document "GenroApp updater key". Its public key is in `src-tauri/tauri.conf.json`, `plugins.updater.pubkey`.
- The public key that signs connection files, base64, in `~/.tauri/genroapp-link.pub.b64`. It is kept in 1Password, vault "Comuni Softwell", document "GenroApp connection signing key (Ed25519)".
- The AWS CLI and the 1Password CLI `op`, signed in, with access to the item `s3-meetlot` of the vault `gpkube`.

#### 1. Raise the version

Set the new version in `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml`. Installed apps see an update only when the version in `latest.json` is higher than theirs.

#### 2. Build and sign

```bash
macos/build-developer-id.sh
```

The script builds `GenroApp.app` for `universal-apple-darwin` with `--no-default-features`. It reads `SOFTWELL_LINK_PUBLIC_KEY` from the environment, or from `~/.tauri/genroapp-link.pub.b64` when the variable is not set. A build without this key rejects every connection file.

The app is signed with the hardened runtime, a secure timestamp and two entitlements: `com.apple.security.device.camera` and `com.apple.security.device.audio-input`. No window of the app uses them since the menus Meetlot and Assistenza were removed. No provisioning profile is embedded.

The app is at `src-tauri/target/universal-apple-darwin/release/bundle/macos/GenroApp.app`.

#### 3. Release: notarize, disk image, updater archive

```bash
GENROAPP_RELEASE_NOTES="What changed in this version" macos/release.sh
```

The script notarizes and staples the app. It builds `GenroApp-VERSION.dmg` with the app and a link to `/Applications`, then signs, notarizes and staples the disk image. It packs the stapled app in `GenroApp-VERSION.app.tar.gz`, signs that archive with the updater key and writes `latest.json`. The three files are in `dist/release/`.

`GENROAPP_NOTARY_PROFILE` and `GENROAPP_UPDATER_KEY` replace the default profile and key file.

On a notarization answer `Invalid`, read the reasons with the submission id:

```bash
xcrun notarytool log SUBMISSION_ID --keychain-profile gp-notary-tool
```

#### 4. Publish

```bash
macos/publish.sh
```

The script uploads to the bucket `genroapp-downloads` of the Hetzner Object Storage (endpoint `https://fsn1.your-objectstorage.com`). `op run` passes the keys of the 1Password item `s3-meetlot` to `aws s3 cp`; they are never printed.

*Published objects*

| Object | Content | Cache-Control |
|---|---|---|
| `GenroApp-VERSION.app.tar.gz` | the archive the updater installs | `immutable` |
| `GenroApp-VERSION.dmg` | the disk image of that version | `immutable` |
| `GenroApp.dmg` | the disk image of the latest version: the fixed download link | `no-cache` |
| `latest.json` | the manifest the installed apps read | `no-cache` |

The public address of an object is `https://genroapp-downloads.fsn1.your-objectstorage.com/OBJECT`. The listing of the bucket answers 403. `latest.json` goes last: an installed app never reads a manifest whose archive is not there yet.

#### 5. How an installed app updates

A release build reads `latest.json` at start and every hour. A higher version puts the badge "1" on the Dock icon and the item "Aggiorna a VERSION e riavvia" in the GenroApp menu, under "Informazioni su GenroApp". The item downloads the archive, verifies its signature with the public key in `tauri.conf.json`, replaces the app and restarts it. A debug build never checks.

#### 6. Verify

```bash
APP=src-tauri/target/universal-apple-darwin/release/bundle/macos/GenroApp.app
xcrun stapler validate "$APP"
spctl -a -t exec -vv "$APP"
codesign -dv --verbose=4 "$APP" 2>&1 | grep -E 'CDHash=|Notarization'
curl -fsS https://genroapp-downloads.fsn1.your-objectstorage.com/latest.json
```

*Expected output*

| Command | Notarized build |
|---|---|
| `stapler validate` | `The validate action worked!` |
| `spctl -a -t exec -vv` | `accepted`, `source=Notarized Developer ID` |
| `codesign -dv` | `Notarization Ticket=stapled` |
| `curl latest.json` | the new version and the URL of its archive |

### Rules

- **must** — Notarize and staple every new build. *(The ticket is bound to the CDHash of one build; a rebuild has another CDHash.)*
- **must** — Keep `com.apple.security.device.camera` and `com.apple.security.device.audio-input` in the signed entitlements. *(Under the hardened runtime macOS denies the camera and the microphone to an app without these entitlements, and shows no permission prompt.)*
- **must** — Build the distributed app with `--no-default-features`. *(The release bundle carries no sidecar executable; with the feature on, the app stops at start when it cannot launch the sidecar.)*
- **must-not** — Write the private key of the updater in a file of this repository. *(Whoever holds it can publish an update that every installed app accepts.)*
- **must** — Keep the private key of the updater in 1Password, vault "Comuni Softwell". *(The installed apps accept only archives signed with the key whose public half they carry; without it they cannot be updated, only reinstalled.)*
- **must-not** — Reuse the name of a published archive for another build. *(An installed app could read a new latest.json together with an old cached archive, and refuse its signature.)*

### Current limits

- The build is for macOS only.
- The Associated Domains entitlement for passkeys (`src-tauri/AssociatedDomains.plist`, `src-tauri/tauri.passkeys.json`) is not in the distributed build: it needs a Developer ID provisioning profile for `it.softwell.genropy.runner`.
- Old versions stay in the bucket until they are deleted by hand.

### Verified facts

- `RUN` `notarytool submit, submission 025519c1-ee07-4f4e-81b8-03690c728a31` · 2026-11-20 — Apple accepted GenroApp 0.1.0, CDHash `9024c8bf811b3afd58dcbe3e8e2725a0b7749d7b`; after stapling, `spctl` answers `accepted`, `source=Notarized Developer ID`.
- `RUN` `notarytool submit, submission 9a1d9df5-7872-4551-8e8b-d29073e980f5` · 2026-11-20 — Apple accepted the disk image `GenroApp-0.1.0.dmg`; `spctl -t open` answers `accepted`, `source=Notarized Developer ID`.
- `RUN` `lipo -archs` · 2026-11-20 — The executable `genropy-runner` of the build holds `x86_64 arm64`.
- `RUN` `macos/publish.sh, curl, shasum` · 2026-11-20 — After publishing, `latest.json`, `GenroApp.dmg` and `GenroApp-0.1.0.app.tar.gz` downloaded over HTTPS have the same SHA-256 as the files in `dist/release/`; the bucket root answers 403.

### Related documents

- `booklet/genropy-runner` (booklet) — GenroApp: a desktop window with card signing
