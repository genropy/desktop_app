# GenroApp

A Tauri 2 desktop window that loads one GenroPy instance with no browser chrome
(no address bar, no tabs).

The menu bar has three menus: **GenroApp**, **Modifica** and **Finestre**. **Finestre** lists the open applications, then **Apri RunDoc…**, **Apri istanza locale…**, **Ricarica**, **Riduci a icona** and **Chiudi finestra**.

## Open a RunDoc

On macOS, opening GenroApp by itself presents the native RunDoc chooser. Cancel leaves the app running without a document window. Use **Apri RunDoc…** to bring the chooser back. Double-click a signed document or drop it onto the application icon in Finder or the Dock to open its window directly. Each application has one window. The runner also accepts legacy signed `.softwell` files. Explicit `--url <address>` is available for development.

## Open a local instance

**Finestre → Apri istanza locale…** (`Cmd+Shift+L`) asks only for a port and opens `http://localhost:<port>/` without a RunDoc. The host and the path are fixed by the app. The window remembers the last port. A localhost window does not receive the card commands.

## Issuing signed connections

Provision an Ed25519 signing key on the issuing service. Keep its private PEM outside the runner and repository. For a distribution build, set `SOFTWELL_LINK_PUBLIC_KEY` to the base64 encoding of its 32 raw public key bytes when running Cargo/Tauri. A build without this value rejects all connection files. Existing saved configurations continue to load.

For example, with an Ed25519 key and OpenSSL 3:

```sh
export SOFTWELL_LINK_PUBLIC_KEY="$(openssl pkey -in issuer.pem -pubout -outform DER | tail -c 32 | openssl base64 -A)"
python3 scripts/sign-connection.py --key issuer.pem --name "Studio Rossi" --url https://studio.example/app --output "Studio Rossi.softwell"
```

The portal serves the resulting file as an attachment with MIME type `application/vnd.softwell.connection+json`. Its JSON envelope contains the base64-encoded payload and Ed25519 signature over the exact decoded payload bytes. Serve pre-issued files or issue from a trusted registry of application URLs.

## Cargo feature `devices`

The Python sidecar and card signing below are compiled only with the feature `devices`, on by default.
`cargo tauri dev` includes them. The distributed macOS build is the site runner only and uses
`--no-default-features`.

## Python sidecar

Local devices are handled by a Python process started by the runner (`sidecar/`).
The page calls it with `device_call` and receives its events as `device-event`:

```js
const T = window.__TAURI__ || window.top.__TAURI__;   // top window from a GenroPy iframe
await T.core.invoke('device_call', {module: 'system', method: 'ping'});   // 'pong'
await T.event.listen('device-event', (e) => console.log(e.payload));
```

- Debug builds run the sources with `uv run --project sidecar python sidecar/main.py`.
- Release builds freeze it with PyInstaller: `cargo tauri build --config tauri.bundle.json`.
- Modules: `system` (`ping`, `info`), `inbox` (watched folders, see the booklet).
- Full description: `xmldocs/booklet/genropy-runner` and `xmldocs/proposal/device-sidecar`.

## Card signing

Port of `firmatore_mac/tessere.py` (icond, branch `firma-digitale-protocolli`), in `src/card.rs`.
The GenroPy page calls:

```js
const { invoke } = window.__TAURI__.core;
const cards = await invoke('card_list');            // [{fingerprint, holder, serial, valid_until, certificate, open}]
const signature = await invoke('card_sign', {card: cards[0].fingerprint, digest: digestBase64});
await invoke('card_close', {card: cards[0].fingerprint});
```

- A card is identified by the sha256 of its signing certificate (keyUsage nonRepudiation).
- `digest` is the base64 of the 32-byte SHA-256 to sign; the result is the base64 RSA PKCS#1 v1.5
  signature of its DigestInfo. The PDF and pyHanko stay on the server.
- A closed card opens a local PIN window. The PIN never reaches the page. A card that signals its
  last attempt or a locked PIN is not tried; a PIN of the wrong length is not tried.
- The session lasts `session_minutes` (default 240) after the PIN.

Optional keys in `config.json`:

```json
{ "url": "https://example.com/app", "pkcs11_libraries": ["/path/to/libpkcs11.dylib"], "session_minutes": 240 }
```

The runner loads every PKCS#11 library that exists among `pkcs11_libraries` and the default
locations, and lists the cards of all of them. A card read by two libraries is taken from the first.
Default locations on macOS: bit4id `/Library/bit4id/pkcs11/libbit4xpki.dylib`, Athena (Actalis cards)
`/Library/Application Support/Athena/libASEP11.dylib`, OpenSC `/Library/OpenSC/lib/opensc-pkcs11.so`.
On Windows: `bit4xpki.dll` of UKC or Firma4NG under `C:\Program Files (x86)\Bit4id`. Firma4NG ships that DLL at 32 bit only,
so the Windows build must target `i686-pc-windows-msvc`:

```bash
rustup target add i686-pc-windows-msvc
cargo tauri build --target i686-pc-windows-msvc
```

Not ported from icond: library reload after sleep, detection of readers plugged in after launch.

## Security

- The setup window can only preview and activate signed connections (`capabilities/default.json`).
- The PIN window can only answer the pending PIN request (`capabilities/pin.json`).
- `card_list`, `card_sign`, `card_close` are granted at runtime to the origin of the configured URL,
  in the main window only. Any other origin loaded in the window gets no command.
  A grant stays until the app exits, also after Change URL.

## Requirements

- Rust (stable) and the Tauri CLI: `cargo install tauri-cli --version "^2" --locked`
- `uv`, for the Python sidecar
- macOS: Xcode command line tools. Windows: WebView2 (preinstalled on Windows 10/11).

## Commands

Run from `src-tauri/`:

```bash
cargo tauri icon ../assets/genroapp/logo/genroapp.png       # regenerate icons/
cargo tauri dev                     # run in development
cargo tauri dev -- -- --url https://example.com/app
cargo tauri build --config tauri.bundle.json   # .app / .dmg on macOS, .msi / .exe on Windows
```

The window size and position are saved by `tauri-plugin-window-state`.

## macOS release

Universal build signed Developer ID, notarized, published to the public bucket `genroapp-downloads`:

```bash
macos/build-developer-id.sh
macos/release.sh
macos/publish.sh
```

- Download: `https://genroapp-downloads.fsn1.your-objectstorage.com/GenroApp.dmg`
- The installed app checks `latest.json` at start and every hour and offers the update in the GenroApp menu.
- Steps and requirements: `xmldocs/booklet/genroapp-macos-release`.

## License

Apache License 2.0, see `LICENSE`.
