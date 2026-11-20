<!-- generated from guida-firma.xml by xmldocs: do not edit, edit the XML -->

## icond app: lavorare e firmare dal Mac
*Instruction booklet · booklet/guida-firma · 2026-11-04*

### Purpose

Invece del browser si usa **icond app**: un'app che si scarica sul Mac, con il logo di icond.

- Con icond app si lavora come nel browser, con le stesse pagine e lo stesso utente.
- Non c'è la barra di navigazione né le schede: la finestra mostra solo icond.
- In genere è più veloce del browser.

Essendo un'app installata sul Mac, icond app può usare servizi del computer che dal browser non sono raggiungibili, e librerie Python che girano sul Mac o sul PC. Per esempio:

- stampare sulle stampanti dell'ufficio;
- tenere d'occhio una cartella del Finder e caricare in icond i documenti che vi si trascinano;
- usare il lettore di tessere per la firma digitale.

Questa guida riguarda la firma digitale. Si individua il documento, o i documenti, e si dà l'ordine di firmare. Tutto avviene dentro icond app: non servono altre applicazioni.

Il documento resta sempre sul server. Sul Mac arriva solo un codice di controllo del documento: la tessera lo firma e la firma torna al server, che la applica al PDF. Il PIN non esce mai dal Mac.

### Requirements

- **icond app** installata sul Mac.
- La tessera di firma digitale con il suo lettore USB, oppure la chiavetta USB di firma.
- Il programma del fornitore della tessera installato sul Mac, per esempio bit4id per le tessere InfoCamere, Aruba e Actalis.
- Il PIN della tessera.

#### 1. Aprire icond app

1. Apri **icond app** con un doppio clic, dalla cartella Applicazioni o dal Dock.
2. Entra con il tuo utente e la tua password, come nel browser.

Se la prima volta l'app chiede l'indirizzo di icond, lo fornisce l'amministratore. Il menu **Instance → Reload** ricarica la pagina.

#### 2. Inserire la tessera

Inserisci la tessera nel lettore, o la chiavetta in una porta USB, prima di firmare. L'app riconosce la tessera dal suo certificato: non importa quale lettore usi.

#### 3. Firmare

1. In icond individua il documento da firmare, o seleziona più documenti.
2. Apri il PDF e controlla che sia quello giusto.
3. Dai l'ordine di firmare.
4. Se più tessere sono inserite, scegli la tua.
5. Si apre la finestra del PIN, con il nome del titolare della tessera. Scrivi il PIN e premi **Sign**.
6. I documenti risultano firmati. Da qui possono partire per mail, PEC o posta.

**La finestra del PIN è di icond app, non della pagina di icond.** Il PIN non passa mai dalla pagina né dalla rete.

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

- **must-not** — Scrivere il PIN in una pagina di icond o del browser. Il PIN si scrive solo nella finestra di icond app. *(icond non chiede mai il PIN nelle sue pagine.)*
- **must** — Estrarre la tessera, o chiudere l'app, quando ci si allontana dal Mac. *(Con la tessera aperta le firme non chiedono il PIN.)*
- **must** — Controllare il documento prima di firmarlo. *(La firma vale come la firma autografa.)*

### Current limits

- Bozza: descrive il funzionamento previsto. icond app con nome e logo di icond, e il comando di firma nelle pagine di icond, sono da realizzare.
- La firma funziona solo dentro icond app. Dal browser (Safari, Chrome) non è disponibile.
- La finestra del PIN e i menu dell'app sono in inglese.

### Related documents

- `proposal/signing-flow` (proposal) — Signing server PDFs with the GenroApp
