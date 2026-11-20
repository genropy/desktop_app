<!-- generated from fiscal-epson.xml by xmldocs: do not edit, edit the XML -->

## Epson fiscal cash registers built into GenroApp
*Proposal · proposal/fiscal-epson · 2026-11-20 · accepted*

### Problem

A GenroPy page must print fiscal receipts and run the daily closure on a cash register of the shop. The page cannot reach the register by itself:

- the page is served over HTTPS and the register answers on the local network, often over plain HTTP: the browser blocks the mixed request;
- the register is on a private address of the shop, which the GenroPy server cannot reach.

GenroApp runs on the computer of the shop, on the same network as the register. It can send the request in Rust and give the answer to the page. The first registers are the Epson FP-81 II RT and FP-90 III RT. RCH Print!F and Custom may follow.

### Current state

- Cash registers are built into GenroApp in Rust, by the decision d-builtin (→ proposal/device-sidecar#d-builtin).
- No code in GenroApp talks to a cash register today.

- `READ` `src-tauri/src/device_login.rs:37-56 (commit 13e1017)` · 2026-11-20 — The device login commands are granted at runtime with a `CapabilityBuilder` limited to `{origin}/*` and to the window of the site. `documents::open` calls the grant only when the origin is a saved site (`src-tauri/src/documents.rs:71-73`).
- `READ` `src-tauri/capabilities/manage.json (commit 13e1017)` · 2026-11-20 — The commands of the window Gestione are granted only to the local window `manage`, never to a site page.
- `READ` `src-tauri/build.rs:5-31 (commit 13e1017)` · 2026-11-20 — Each command gets an `allow-<command>` permission only when it is listed in `build.rs`.
- `READ` `src-tauri/Cargo.toml (commit 13e1017)` · 2026-11-20 — The crate has `reqwest` 0.13 with `rustls-no-provider` and `rustls` 0.23 with `ring`. It has no XML crate.

### Proposed solution

#### Protocol

The protocol is Epson "ePOS Fiscal Print". The sources are three Epson documents published on [scontrinosmart.it](https://www.scontrinosmart.it/en/fp81ii/documents), all marked confidential by Epson:

- "ePOS Fiscal Print Solution Development Guide", FP 000 055 EN Rev. U, 2023-12-01 (below: Guide). Rev. T on download4.epson.biz answers HTTP 403; Rev. U replaces it.
- "Fiscal Printer Intelligent Features Guide", FP 000 056 EN Rev. L, 2023-12-29, for both FP-81 II RT and FP-90 III RT (below: Features).
- "Communication Protocol", FP 000 008 EN Rev 8.10, 2023-12-18 (below: Protocol).

Secondary sources, used only to confirm: Epson `fiscalprint.js` 1.1.0, Odoo 19 `l10n_it_pos`, OCA `fiscal_epos_print`.

- `READ` `Guide §2.2.1-2.2.3` · 2026-11-20 — A request is `POST /cgi-bin/fpmate.cgi?devid=local_printer&timeout=10000` with `Content-Type: text/xml; charset=utf-8`. The body is a SOAP envelope `s:Envelope` / `s:Body` (namespace `http://schemas.xmlsoap.org/soap/envelope/`) holding one root: `printerFiscalReceipt`, `printerFiscalReport` or `printerCommand`.
- `READ` `Features §8` · 2026-11-20 — HTTP or HTTPS is a setting of the register (SET 34, SSL MODE 0, 1 or 2), not of the model. With SSL on, only HTTPS on port 443 answers, with a self-signed or uploaded certificate.
- `READ` `Guide §7, Features §2` · 2026-11-20 — A user and password are optional on the register. On a standalone RT model they use HTTP Digest authentication.
- `READ` `Guide §3.11, §6.1-6.2` · 2026-11-20 — The answer is `<response success="true|false" code="…" status="…">` with an `addInfo` child. `code` names the error (`PRINTER ERROR`, `EPTR_REC_EMPTY`, `PARSER_ERROR`, …); with `PRINTER ERROR`, `status` is the native error number of the Protocol §8.3.1.
- `READ` `Guide §5.4.1, §5.8.1, §5.9.16` · 2026-11-20 — After a receipt `addInfo` carries `fiscalReceiptNumber`, `fiscalReceiptAmount`, `fiscalReceiptDate`, `fiscalReceiptTime`, `zRepNumber`, `serialNumber`. After a report it carries `zRepNumber`, `dailyAmount`, `serialNumber`. After `queryPrinterStatus statusType="1"` it carries the `rt…` fields.
- `READ` `Guide §2.2.2, §5.2` · 2026-11-20 — Amounts and quantities in a request accept dot or comma, with no thousands separator. Amounts in an answer use a comma unless the URL carries `reply_decimal_separator=.`.
- `READ` `Guide §3.13` · 2026-11-20 — fpmate resets the register at every fiscal request: a receipt left open is cancelled.
- `READ` `Guide §5.4.14` · 2026-11-20 — `paymentType` is `0` cash, `1` cheque, `2` credit or electronic, `3` ticket, `4` multiple tickets, `5` not paid, `6` payment discount. `index` selects the totaliser inside the type.
- `READ` `Guide §6, Protocol §1` · 2026-11-20 — The fpmate XML has no element specific to FP-81 II RT or FP-90 III RT. The differences are in native commands reached through `directIO`, which this proposal does not use.

Not verified, and left out of the first version:

- the details of the Digest authentication (realm, algorithm, qop);
- whether the register refuses a request without `SOAPAction` or `If-Modified-Since`: GenroApp sends both, as the Guide and `fiscalprint.js` do;
- the meaning of `payment="0"` on cash in the examples of the Guide: GenroApp always sends the amount;
- the format of `rtExpiryCD` and `rtExpiryCA` (the Guide says `yyyymmdd`, the Protocol says `DDMMYY`): GenroApp passes them as text;
- the name of each electronic totaliser `index` 1-10: it is a programming of each register.

#### Commands for the page

Four commands, granted at runtime only to the windows of saved sites, for the origin of the site, like `device_login::grant`. Each command checks that `origin` is the origin of the calling page and of a saved site, like `device_login::caller_site`. The page never passes an address.

| Command | XML sent | Result |
|---|---|---|
| `fiscal_status({origin})` | `queryPrinterStatus` with `statusType="0"`, then `"1"` | `{configured, printer_status, cpu_rel, mf_status, rt: {main_status, sub_status, daily_open, no_working_period, files_to_send, …}}` |
| `fiscal_receipt({origin, lines, payments})` | `printerFiscalReceipt`: `beginFiscalReceipt`, one `printRecItem` per line, one `printRecTotal` per payment, `endFiscalReceipt` | `{number, amount, date, time, z_number, serial}` |
| `fiscal_x_report({origin})` | `printerFiscalReport` / `printXReport` | `{z_number, daily_amount, serial}` |
| `fiscal_daily_closure({origin})` | `printerFiscalReport` / `printZReport` | `{z_number, daily_amount, serial}` |

**Example — Arguments of fiscal_receipt**

```json
{
  "origin": "https://shop.example.com",
  "lines": [
    {"description": "PANINO", "quantity": "2", "unit_price": "6.00", "department": 1}
  ],
  "payments": [
    {"type": "cash", "amount": "10.00"},
    {"type": "electronic", "index": 1, "amount": "2.00"}
  ]
}
```

- `type` is one of `cash`, `cheque`, `electronic`, `ticket`, `not_paid`, mapped to `paymentType` 0, 1, 2, 3, 5. `index` is optional; the register uses 1 when it is missing.
- Amounts and quantities are decimal strings with a dot, at most 2 and 3 decimals, as the Guide §5.2 accepts them. A float from JavaScript is refused: it cannot carry cents exactly.
- A description longer than 38 characters, or with a character the Guide §8 lists as not printable, is refused with an error. GenroApp does not truncate or replace text on its own.
- An answer with `success="false"` becomes an error with `code`, `status` and, for a native error, its Italian name from the Protocol §8.3.1.

- **must** — GenroApp sends one request at a time to each register. A second command waits for the first one. *(fpmate resets the register at every request (Guide §3.13): a second request would cancel a receipt in progress)*
- **must** — The HTTP timeout of GenroApp is longer than the `timeout` of the request: 10 s for status and receipts, 60 s for the closure. *(the Z report waits for the transmission to the tax authority before it answers (Guide §3.4))*

#### Configuration in Gestione

The window Gestione gains a section "Registratore di cassa" for each saved site:

- address of the register (host or IP, optional port);
- a box "HTTPS", for a register with SSL MODE 1 or 2;
- a button "Prova", which runs `queryPrinterStatus` and shows serial number, firmware and RT state.

- The HTTPS client is rustls: it speaks TLS 1.2 and 1.3 only. A register with SSL MODE 1 that offers only an older version does not connect.

The configuration is saved in `<app config dir>/fiscal.json`, keyed by the origin of the site. The commands of this section are granted only to the window `manage` (`capabilities/manage.json`).

#### Code

- `src-tauri/src/fiscal.rs`: the four page commands, the grant, the configuration, the lock per register, the pinned certificate, and the three commands `manage_fiscal_get`, `manage_fiscal_set`, `manage_fiscal_test` of Gestione.
- `src-tauri/src/fiscal/epson.rs`: building the XML and reading the answer. No I/O, so the tests run without a register.
- `setup/manage.html`, `setup/manage.js`: the section of Gestione. `manage.rs`: removing a site also removes its register.
- `lib.rs` (handler list), `build.rs` (command list), `documents.rs` (call to the grant), `capabilities/manage.json`, `Cargo.toml` (XML crate).

RCH and Custom get their own file beside `epson.rs` when they are written. A shared trait waits for the second register.

#### Tests

- The XML built for status, receipt, X report and Z report is compared with the expected text.
- The answers are parsed from samples shaped like those of the Guide §5.4.1, §5.8.1, §5.9.16, §6.5 and §6.10: success, `PRINTER ERROR`, offline, comma amounts.
- Validation: description too long, unsupported character, float amount, more than 2 decimals, unknown payment type.

- No test runs against a real register. A test on an FP-81 II RT or FP-90 III RT is needed before release, and it settles the points listed as not verified.

### Decisions

- **d-fiscal-scope** `decided` — What does the first version cover?
   1. **Status, receipt, X report, daily closure**: Sales and payments only. No discounts, refunds, voids, lottery code, customer tax code.
   2. **Also refund and void**: Adds `printRecRefund` and the `REFUND` / `VOID` messages of the Guide §5.5. They need the number, date and serial of the original receipt.
   → Choice: **Status, receipt, X report, daily closure** (GP, 2026-11-20) — Status, receipt, X report and daily closure. Refund and void come later.
- **d-fiscal-key** `decided` — Is the register bound to a site or to the computer?
   1. **One register per saved site**: `fiscal.json` keyed by origin. Two sites on the same computer each name their register, also the same one.
   2. **One register per computer**: Every saved site uses it. Simpler to configure; a site that should not print can still print.
   → Choice: **One register per saved site** (GP, 2026-11-20) — One register per saved site, in fiscal.json keyed by origin.
- **d-fiscal-tls** `decided` — How does GenroApp trust the self-signed certificate of a register with HTTPS?
   1. **Pin the certificate at the test**: "Prova" shows the SHA-256 fingerprint; the user confirms it and GenroApp saves it in `fiscal.json`. Later connections accept only that certificate. Needs a custom rustls verifier.
   2. **Accept any certificate**: Encryption without identity: another machine on the network can answer in place of the register.
   → Choice: **Pin the certificate at the test** (GP, 2026-11-20) — The certificate is pinned at the test in Gestione.
- **d-fiscal-auth** `decided` — What about a register with user and password (HTTP Digest)?
   1. **Not in the first version**: An answer 401 becomes the error "Il registratore chiede una password: non ancora supportato". Digest is added after a test on a real register.
   2. **Implement Digest now**: Written without a verified realm and algorithm, and without a register to test it.
   → Choice: **Not in the first version** (GP, 2026-11-20) — No Digest authentication in the first version.
- **d-fiscal-confirm** `decided` — Does GenroApp ask the user before the daily closure?
   1. **Native dialog before the Z report**: The closure cannot be undone. A dialog of GenroApp, not of the page, names the site and the register.
   2. **The page decides**: The page already asks; GenroApp runs the command.
   → Choice: **Native dialog before the Z report** (GP, 2026-11-20) — A native dialog of GenroApp before the Z report.
- **d-fiscal-build** `decided` — Is the code always in the build, or behind a cargo feature?
   1. **Always built**: No new dependency except an XML crate. It works in the distributed build, as d-builtin (→ proposal/device-sidecar#d-builtin) requires.
   2. **Feature fiscal**: Fits the split of features under way in another branch. One more combination to build.
   → Choice: **Always built** (GP, 2026-11-20) — Always built, no cargo feature.

### Consequences

- `Cargo.toml` gains an XML crate (`quick-xml`) for reading the answer. The request is built by hand with escaping, so its text is fixed in the tests.
- The files shared with the branches on local printing and on downloaded modules (`Cargo.toml`, `lib.rs`, `build.rs`, `manage.rs`, `setup/manage.*`) need an agreed order of merge.
- A saved site can print fiscal receipts on the register configured for it, with no further confirmation except the closure if d-fiscal-confirm (→ d-fiscal-confirm) says so.
- The register must be reachable from the computer: same network, port 80 or 443 open.

### Related documents

- `proposal/device-sidecar` (proposal) — Local devices through a Python sidecar
- `proposal/biometric-login` (proposal) — GenroApp sites and biometric login through a device key
