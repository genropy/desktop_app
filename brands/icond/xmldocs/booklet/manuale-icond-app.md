<!-- generated from manuale-icond-app.xml by xmldocs: do not edit, edit the XML -->

## icond app: installazione e uso
*Instruction booklet · booklet/manuale-icond-app · 2026-11-05*

### Purpose

**icond app** è un'app per Mac che apre icond in una finestra propria, al posto del browser.

- Si lavora come nel browser, con le stesse pagine e lo stesso utente.
- Non c'è la barra di navigazione né le schede: la finestra mostra solo icond.
- L'app può usare servizi del Mac che il browser non raggiunge, come il lettore di tessere per la firma digitale.

### Requirements

- Un Mac con processore Apple (M1 o successivo).
- Il file **icond app.zip**, ricevuto dall'amministratore.
- L'indirizzo di icond, il proprio utente e la password.

#### 1. Installare

1. Fai doppio clic su **icond app.zip**: accanto compare **icond app**.
2. Trascina **icond app** nella cartella **Applicazioni**.

#### 2. Sbloccare l'app la prima volta

Questa versione di prova non è ancora certificata da Apple. Al primo avvio macOS la blocca: va sbloccata una volta sola.

Il blocco dipende da come arriva il file: browser, Mail, Messaggi, AirDrop e Telegram lo marcano come scaricato. Copiata da una chiavetta USB, l'app si apre senza blocchi e questo passo non serve.

**Metodo 1: da Impostazioni di Sistema.**

1. Sposta prima l'app in **Applicazioni**, come al passo precedente.
2. Fai doppio clic su **icond app**. macOS mostra un avviso e non la apre. Premi **Fine**: non scegliere **Sposta nel Cestino**.
3. Apri il menu Apple (la mela in alto a sinistra) → **Impostazioni di Sistema…** → **Privacy e sicurezza** nella colonna a sinistra.
4. Scorri fino alla sezione **Sicurezza**, in fondo alla pagina. Compare un messaggio che nomina **icond app**, con il pulsante **Apri comunque**: premilo.
5. Conferma con la password del Mac, o con Touch ID.
6. Si apre un ultimo avviso: premi **Apri comunque**. L'app parte.
7. Da qui in poi l'app si apre con un normale doppio clic, senza avvisi.

Il pulsante **Apri comunque** compare solo dopo un tentativo di apertura. Se non lo vedi, fai di nuovo doppio clic sull'app e torna in **Privacy e sicurezza**.

**Metodo 2: da Terminale.** Serve quando il metodo 1 non mostra il pulsante, o quando macOS dice che l'app è danneggiata.

1. Sposta l'app in **Applicazioni**.
2. Premi **Cmd+Spazio**, scrivi **Terminale** e premi Invio.
3. Copia questa riga, incollala nel Terminale e premi Invio:

```bash
xattr -dr com.apple.quarantine "/Applications/icond app.app"
```

- Se il comando non scrive niente, ha funzionato. Chiudi il Terminale e apri l'app con un doppio clic.
- Il comando toglie dall'app solo il marchio «scaricato da Internet». Non modifica l'app e non cambia le impostazioni di sicurezza del Mac.

#### 3. Primo avvio

1. Si apre una piccola finestra **Instance URL**. Scrivi l'indirizzo di icond e premi **Open**.
2. Si apre icond. Entra con il tuo utente e la tua password.

L'indirizzo resta salvato: dalle volte successive l'app si apre direttamente su icond. Anche dimensione e posizione della finestra vengono ricordate.

#### 4. Lavorare

Si lavora come nel browser. Il menu **Instance** ha due comandi:

*Comandi del menu Instance*

| Comando | Tasti | Cosa fa |
|---|---|---|
| Reload | Cmd+R | Ricarica la pagina, come il tasto di aggiornamento del browser. |
| Change URL… | Cmd+Shift+U | Riapre la finestra dell'indirizzo, per passare a un'altra installazione di icond. |

Per chiudere l'app: **Cmd+Q**.

#### 5. Firmare i documenti

Con la tessera di firma nel lettore si firmano i documenti dentro l'app: si individuano i documenti, si dà l'ordine di firmare e si scrive il PIN nella finestra dell'app. La guida **icond app: lavorare e firmare dal Mac** descrive i passi.

#### 6. Se qualcosa non va

*Problemi e rimedi*

| Cosa succede | Cosa fare |
|---|---|
| macOS non apre l'app | Sbloccala come al passo «Sbloccare l'app la prima volta». |
| In Privacy e sicurezza non c'è **Apri comunque** | Fai di nuovo doppio clic sull'app e torna nella pagina, oppure usa il metodo 2 da Terminale. |
| macOS dice che l'app è danneggiata e propone il Cestino | Non spostarla nel Cestino: usa il metodo 2 da Terminale. |
| Hai premuto **Sposta nel Cestino** per errore | Recupera l'app dal Cestino (clic destro → **Ripristina**) e ripeti lo sblocco. |
| L'app non parte su un Mac con processore Intel | Questa versione è solo per processori Apple. Chiedi all'amministratore la versione per Intel. |
| Hai sbagliato l'indirizzo | Menu **Instance → Change URL…** e scrivi quello giusto. |
| La pagina non risponde | Menu **Instance → Reload**. |

### Current limits

- Versione di prova: non è certificata (notarizzata) da Apple, per questo va sbloccata al primo avvio.
- Funziona solo sui Mac con processore Apple.
- I menu e le finestre dell'app sono in inglese.
