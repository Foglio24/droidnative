# Architettura di DroidNative

## Scelta: Architettura B, predisposta per C

| Opzione | Valutazione |
|---|---|
| A — Fork di Waydroid | Scartata. Waydroid ha 968 issue aperte e rilasci frequenti; tutto ciò che serve (stato, app, proprietà, scelta GPU) è raggiungibile da CLI, D-Bus e `waydroid.cfg` senza modificarlo. Un fork impone un rebase continuo senza vantaggi tecnici. |
| **B — Runtime manager indipendente** | **Adottata.** DroidNative orchestra Waydroid e gli stack GPU esistenti, aggiunge rilevamento, verifica dell'accelerazione, gestione APK e UI. |
| C — Backend multipli | Predisposta: il runtime è dietro il trait `AndroidRuntime`, la GPU dietro i descrittori `GpuBackendKind`. ReDroid può essere aggiunto come secondo backend. |

## Componenti

```
┌───────────────────────────── app/ (Tauri 2) ─────────────────────────────┐
│ React + TypeScript (UI)  ──invoke──▶  src-tauri (comandi Rust)           │
└──────────────────────────────────────┬───────────────────────────────────┘
                                       │ libreria
┌──────────────────────── crates/droidnative-core ─────────────────────────┐
│ host/      rilevamento sistema (os-release, kernel, binder, cgroups, …)  │
│ gpu/       GPU, stack driver, backend GPU, verifica accelerazione        │
│ runtime/   trait AndroidRuntime + backend Waydroid                       │
│ apk/       ispezione APK (AndroidManifest binario, ABI, SHA-256)         │
│ privileged operazioni root in elenco chiuso                              │
│ report     report di compatibilità                                       │
└──────────────────────────────────────┬───────────────────────────────────┘
                     pkexec (polkit)   │   come utente
┌──────────────────────────────────────▼───────────────────────────────────┐
│ droidnative priv <op>  (stesso binario CLI, solo operazioni ammesse)     │
│ waydroid CLI · D-Bus id.waydro.Container · /var/lib/waydroid/waydroid.cfg│
└──────────────────────────────────────────────────────────────────────────┘
```

## Modello di sicurezza

- La UI gira **senza privilegi**. Le operazioni root sono un `enum` chiuso
  (`PrivilegedOp`) eseguito da `droidnative priv …` tramite `pkexec`: nessuna
  stringa di shell arbitraria attraversa il confine dei privilegi.
- Ogni argomento che entra in un'operazione privilegiata viene validato
  (nomi package, chiavi di proprietà, nodi `/dev/dri/renderD*` reali).
- I comandi esterni vengono lanciati senza shell, con `LC_ALL=C` (l'output è
  analizzato in modo deterministico anche su sistemi localizzati) e timeout.
- Mai usate le opzioni pericolose di `waydroid shell` (`-L`, `-C`, `-G`).
- Gli APK vengono ispezionati prima dell'installazione (manifest, ABI,
  SHA-256) e l'installazione è verificata confrontando il package con
  l'elenco app: Waydroid restituisce 0 anche quando fallisce.

## Backend GPU

| Backend | Stack | Quando |
|---|---|---|
| Mesa Native | Mesa (radeonsi/RADV, iris/ANV) via `drm_device` + gralloc gbm | AMD, Intel; GPU che pilota il compositor o import dmabuf verificato |
| NVIDIA Accelerated | waydroid-nvidia `main`: Venus → virglrenderer host, GL ES via ANGLE | RTX 20+ con modulo open, driver ≥ 595.71, modeset=1, compositor su NVIDIA |
| NVIDIA Experimental | waydroid-nvidia `public-test`: `nvidia-prolapse` (NVK + backend NVIDIA nel guest) | Stessi requisiti, componente sperimentale, solo Android 13 |
| Software Fallback | SwiftShader | Sempre disponibile, mai dichiarato accelerato |

### Installazione dello stack NVIDIA (`nvidia_stack.rs`)

1. Il rilascio è **fissato nel codice** (`PINNED_RELEASES`): tag, SHA-256 del file
   `SHA256SUMS` e commit del sorgente. I valori sono stati fissati dopo aver
   controllato l'attestazione di build GitHub di ogni file (`build.yml` al tag).
2. Come root, in `/var/lib/droidnative/nvidia-stack/<tag>` (0700): download di
   `SHA256SUMS` (hash confrontato col valore fissato), dei 6 file (ognuno confrontato
   con la sua riga) e dell'archivio sorgente (commit letto dall'header pax di
   `git archive` e confrontato col valore fissato). File già presenti e validi
   vengono riusati; un file che non corrisponde viene cancellato.
3. Rimozione degli override software di DroidNative da `waydroid.cfg`, poi `apt-get
   purge waydroid` (il suo conffile `waydroid.menu` bloccherebbe l'installer, che non
   sovrascrive file di pacchetti). Immagini e dati non sono file del pacchetto.
4. `packaging/install.sh --tag <tag> --from <dir>` dall'archivio verificato: l'installer
   ufficiale ricontrolla hash e coppia sorgente/binari e installa la Waydroid patchata.
5. `waydroid-nvidia-setup --mode venus|nvidia-prolapse`; stato registrato in
   `/var/lib/droidnative/nvidia-stack.json`.

In modalità `venus` DroidNative avvia il servizio utente `wd-venus.service` prima della
sessione Android.

L'accelerazione è considerata **attiva solo dopo la verifica**: lettura della
riga `GLES:` di `dumpsys SurfaceFlinger` nel container. Le proprietà
`ro.hardware.egl`/`gralloc` indicano solo la configurazione.
