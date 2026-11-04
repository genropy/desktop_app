# GenroPy Runner

A Tauri 2 desktop window that loads one GenroPy instance with no browser chrome
(no address bar, no tabs).

## Instance URL

The URL is resolved at startup in this order:

1. `--url <address>` on the command line (that launch only).
2. `url` in `config.json` inside the app config directory:
   - macOS: `~/Library/Application Support/it.softwell.genropy.runner/config.json`
   - Windows: `%APPDATA%\it.softwell.genropy.runner\config.json`
   - Linux: `~/.config/it.softwell.genropy.runner/config.json`
3. Neither present: a local setup page asks for the URL and saves it to `config.json`.

The menu **Instance → Change URL…** (`Cmd/Ctrl+Shift+U`) reopens the setup page.
**Instance → Reload** (`Cmd/Ctrl+R`) reloads the GenroPy page.

```json
{ "url": "https://example.com/app" }
```

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

- The setup window can only read and save the URL (`capabilities/default.json`).
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
cargo tauri icon app-icon.svg       # regenerate icons/
cargo tauri dev                     # run in development
cargo tauri dev -- -- --url https://example.com/app
cargo tauri build --config tauri.bundle.json   # .app / .dmg on macOS, .msi / .exe on Windows
```

The window size and position are saved by `tauri-plugin-window-state`.

## License

Apache License 2.0, see `LICENSE`.
