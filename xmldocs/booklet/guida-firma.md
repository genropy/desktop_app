<!-- generated from guida-firma.xml by xmldocs: do not edit, edit the XML -->

## Firmare i documenti con GenroPy Runner
*Instruction booklet · booklet/guida-firma · 2026-11-04*

### Purpose

Questa guida spiega come firmare digitalmente i documenti del gestionale dal proprio Mac, con la tessera o la chiavetta di firma.

Il documento resta sempre sul server. Sul Mac arriva solo un codice di controllo del documento: la tessera lo firma e la firma torna al server, che la applica al PDF. Il PIN non esce mai dal Mac.

### Requirements

- L'app **GenroPy Runner** installata sul Mac.
- La tessera di firma digitale con il suo lettore USB, oppure la chiavetta USB di firma.
- Il programma del fornitore della tessera installato sul Mac, per esempio bit4id per le tessere InfoCamere, Aruba e Actalis.
- Il PIN della tessera.
- L'indirizzo del gestionale, che fornisce l'amministratore.

#### 1. Aprire l'app

1. Apri **GenroPy Runner** con un doppio clic.
2. La prima volta l'app chiede l'indirizzo del gestionale. Scrivilo e conferma: dalla volta successiva l'app si apre direttamente sul gestionale.
3. Entra con il tuo utente e la tua password, come nel browser.

La finestra non ha barra degli indirizzi né schede: è il gestionale e basta. Il menu **Instance → Reload** ricarica la pagina; **Instance → Change URL…** cambia l'indirizzo.

#### 2. Inserire la tessera

Inserisci la tessera nel lettore, o la chiavetta in una porta USB, prima di firmare. L'app riconosce la tessera dal suo certificato: non importa quale lettore usi.

#### 3. Firmare

1. Apri l'elenco dei documenti da firmare.
2. Apri il PDF e controlla che sia quello giusto.
3. Seleziona uno o più documenti e premi il pulsante di firma.
4. Se più tessere sono inserite, scegli la tua.
5. Si apre la finestra del PIN, con il nome del titolare della tessera. Scrivi il PIN e premi **Sign**.
6. I documenti risultano firmati. Da qui possono partire per mail, PEC o posta.

**La finestra del PIN è dell'app, non del gestionale.** Il PIN non passa mai dalla pagina né dalla rete.

#### 4. Un PIN per molte firme

Dopo il PIN la tessera resta aperta per alcune ore, di solito 4. In quel tempo le firme successive non chiedono di nuovo il PIN.

Per chiudere la tessera prima, estraila dal lettore o chiudi l'app. Alla firma successiva l'app chiede di nuovo il PIN.

#### 5. Se qualcosa non va

*Problemi e rimedi*

| Cosa succede | Cosa fare |
|---|---|
| L'app non vede la tessera dopo che il Mac è andato in stop, o dopo aver collegato il lettore ad app già aperta | Chiudi e riapri l'app. |
| Il PIN viene rifiutato | Riprova con attenzione: ogni errore consuma un tentativo. |
| L'app dice che la tessera segnala l'ultimo tentativo o il PIN bloccato | L'app non prova il PIN, per non bloccare la tessera. Rivolgiti all'amministratore: serve il PUK del fornitore. |
| L'app dice che il PIN ha una lunghezza sbagliata | Il PIN non è stato provato e nessun tentativo è perso. Ricontrolla il PIN. |
| La pagina non risponde | Usa **Instance → Reload**. |

### Rules

- **must-not** — Scrivere il PIN in una pagina del gestionale o del browser. Il PIN si scrive solo nella finestra dell'app. *(Il gestionale non chiede mai il PIN.)*
- **must** — Estrarre la tessera, o chiudere l'app, quando ci si allontana dal Mac. *(Con la tessera aperta le firme non chiedono il PIN.)*
- **must** — Controllare il documento prima di firmarlo. *(La firma vale come la firma autografa.)*

### Current limits

- Bozza: descrive il funzionamento previsto. Il pulsante di firma nelle pagine del gestionale è da realizzare.
- La firma funziona solo dentro l'app. Dal browser (Safari, Chrome) la firma diretta non è disponibile.
- La finestra del PIN e i menu dell'app sono in inglese.

### Related documents

- `proposal/signing-flow` (proposal) — Signing server PDFs with the GenroPy Runner
