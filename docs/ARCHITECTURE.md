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
│ extras     immagine GAPPS, traduzione ARM libndk, ID dispositivo Google  │
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

### Nodo della GPU NVIDIA a ogni avvio (`droidnative-render-node.service`)

waydroid-nvidia scrive in `drm_device` il nodo `/dev/dri/renderD<N>` della GPU NVIDIA solo
quando applica una modalità, mentre il suo `ExecStartPre` (`driver-mode verify-active`)
controlla soltanto. Sui sistemi ibridi i numeri seguono l'ordine di caricamento dei driver e
possono scambiarsi tra un avvio e l'altro: il container allora non parte. Il pacchetto
installa `droidnative-render-node.service` (oneshot, `ConditionPathExists` su
`waydroid-nvidia-setup`, `After=` il dispositivo `sys-subsystem-waydroid_nvidia-render`) e un
drop-in di `waydroid-container.service` che lo richiede e lo precede. Il servizio esegue
`droidnative priv repin-render-node`: in modalità venus sceglie il nodo per produttore PCI
(`0x10de`, come waydroid-nvidia), aggiorna `drm_device` e rigenera nodi LXC e proprietà con
`waydroid upgrade -o`, che su un container fermo non usa né D-Bus né systemd. Non chiama mai
`systemctl` sul container: un `stop` o `start` in quel momento si scontrerebbe con il job di
avvio che lo sta aspettando. Dall'interfaccia la stessa operazione con `--start` avvia poi il
container.

## Google Play e traduzione ARM (`extras.rs`)

**Cambio d'immagine** (`SwitchSystemImage`): `waydroid init -f -s GAPPS|VANILLA`. Prima
e dopo, DroidNative si assicura che il container LXC sia fermo (`lxc-info`, poi
`systemctl stop` e `waydroid container stop`): `waydroid init` riavvia il container fuori
da systemd e `waydroid-nvidia-setup` rifiuterebbe di procedere. Con waydroid-nvidia la
modalità letta da `waydroid.cfg` prima del cambio viene riattivata, perché `init -f`
riscrive le proprietà del vendor. `init -f` conserva `[properties]` e l'overlay
(`/var/lib/waydroid/overlay`), quindi anche libndk; i dati Android restano.

**libndk** (`InstallArmTranslation`): archivio supremegamers per Android 13 fissato a
commit e SHA-256 (lo stesso usato da waydroid_script). Come root, in
`/var/lib/droidnative/libndk` (0700): download solo se il file in cache non corrisponde,
verifica dell'hash, estrazione delle sole voci `*/prebuilts/` nell'overlay di sistema con
rifiuto dei percorsi che escono dalla destinazione, permessi `root:shell 0755` per
`bin/` e `root:root 0644` per il resto, elenco dei file installati in
`/var/lib/droidnative/libndk-files.txt`. Le proprietà (`ro.product.cpu.abilist*`,
`ro.dalvik.vm.native.bridge=libndk_translation.so`, `ro.enable.native.bridge.exec`,
`ro.dalvik.vm.isa.arm*`) vanno in `[properties]`, poi `waydroid upgrade -o` e riavvio
del container. La rimozione cancella solo i file elencati e le directory rimaste vuote,
toglie le proprietà e annulla i gestori `binfmt_misc` che `ndk_translation.rc` registra
nel kernel dell'host a ogni avvio di Android.

**ID dispositivo Google** (`GoogleDeviceId`): `sqlite3` nel container su
`gservices.db` di Google Services Framework. La registrazione su
google.com/android/uncertified la fa l'utente con il proprio account; l'app apre solo
quell'URL, l'unico ammesso dalla capability `opener:allow-open-url`.

L'accelerazione è considerata **attiva solo dopo la verifica**: lettura della
riga `GLES:` di `dumpsys SurfaceFlinger` nel container. Le proprietà
`ro.hardware.egl`/`gralloc` indicano solo la configurazione.
