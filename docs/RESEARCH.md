# Ricerca preliminare — progetti open source

Analisi svolta il 2026-10-19 leggendo il **codice sorgente** (clone shallow dei
repository, non solo README). Metadati da GitHub API alla stessa data.

## Sintesi

| Progetto | Linguaggio | Licenza | Ultimo push | Issue aperte | Ruolo per DroidNative |
|---|---|---|---|---|---|
| waydroid/waydroid | Python | GPL-3.0 | 2026-10-16 | 968 | **Backend runtime** (riuso integrale, non fork) |
| Shiro836/waydroid-nvidia | C + patch | MIT (+ licenze upstream sulle patch) | 2026-10-18 | 22 | **Backend GPU NVIDIA** (Venus stabile + `public-test`) |
| remote-android/redroid-doc | Shell/doc | non dichiarata | 2026-05-17 | 359 | Backend alternativo futuro (Architettura C) |
| atxbot/redroid | — | — | 2023-12-13 | 0 | Fork statico di redroid-doc, nulla da riusare |
| fogelmanjg/redroid-nvidia | C | Apache-2.0 | 2026-09-27 | 0 | Riferimento tecnico (NVENC, bug driver Ada) |
| waydroid-helper/waydroid-helper | Python/GTK4 | GPL-3.0 | 2026-09-12 | 46 | Riferimento UX (estensioni, keymapper, polkit) |
| Iskierek100/waydroid-x11-launcher | Shell | MIT | 2025-08-15 | 0 | Riferimento per supporto X11 (weston annidato) |
| waydroid/waydroid-package-manager | Shell | GPL-3.0 | 2023-11-06 | 2 | Riferimento per repo F-Droid (inattivo) |

## waydroid/waydroid

**Architettura (da `tools/`)**: CLI Python `waydroid` con tre livelli.

- *Container* (root): `waydroid container start`, servizio systemd
  `waydroid-container.service`, nome D-Bus di sistema `id.waydro.Container`,
  oggetto `/ContainerManager`, interfaccia `id.waydro.ContainerManager` con i
  metodi `Start(a{ss})`, `Stop(b)`, `Freeze()`, `Unfreeze()`, `GetSession() → a{ss}`.
  La policy D-Bus (`dbus/id.waydro.Container.conf`) consente a **qualsiasi
  utente** di inviare messaggi: lo stato è leggibile senza privilegi.
- *Sessione* (utente): `waydroid session start` resta in foreground e possiede
  `id.waydro.Session` sul bus di sessione.
- *Android*: i comandi `app install/remove/launch/list` e `prop get/set` passano
  per il servizio binder `IPlatform` dentro Android tramite gbinder
  (`tools/interfaces/IPlatform.py`).

**Comportamenti rilevanti scoperti nel codice**

- `tools/helpers/gpu.py`: `unsupported = ["nvidia"]` — il nodo render NVIDIA
  viene **sempre scartato** e Waydroid sceglie il primo `renderD*` non NVIDIA.
  Su un sistema ibrido NVIDIA + iGPU AMD sceglierà quindi l'iGPU.
- La GPU si forza con la chiave `drm_device` nella sezione `[waydroid]` di
  `/var/lib/waydroid/waydroid.cfg`.
- `tools/helpers/lxc.py`: con un nodo DRM imposta `ro.hardware.gralloc=gbm`,
  `ro.hardware.egl=mesa`, `gralloc.gbm.device=<nodo>`; senza nodo ricade su
  `gralloc=default` + `egl=swiftshader` (rendering software). Il driver Vulkan
  è dedotto dal driver kernel (`amdgpu→radeon`, `i915/xe→intel`, …).
- `tools/actions/app_manager.py`: `install()` registra l'errore con
  `logging.error` ma **esce con codice 0** anche in caso di fallimento. Un
  frontend deve verificare l'installazione in modo indipendente (es. confrontare
  il package dell'APK con `waydroid app list`).
- `tools/helpers/images.py`: le immagini OTA sono verificate con SHA-256 contro
  il campo `id` del canale (`ota.waydro.id`). Riutilizziamo questa verifica.
- `tools/helpers/arch.py`: supporta x86, x86_64, arm, arm64; richiede SSSE3 su x86.
- Wayland obbligatorio: la sessione usa `WAYLAND_DISPLAY`; su X11 serve un
  compositor annidato.
- Sicurezza: profili AppArmor (`data/configs/apparmor_profiles`), seccomp
  (`waydroid.seccomp`); `waydroid shell -L/-C/-G` disabilitano LSM, capability e
  cgroup — DroidNative non deve mai usarli.
- Pacchetto: non presente nei repository Ubuntu 26.04; disponibile nel
  repository ufficiale `repo.waydro.id` (suite `resolute` presente, firmata con
  chiave `7CE0331F71E0A238`).

**Riuso**: integrale come runtime. Un fork (Architettura A) significherebbe
mantenere 82 file Python con 968 issue aperte e rilasci frequenti: costo alto,
beneficio basso, perché tutto ciò che serve è raggiungibile da CLI, D-Bus e
`waydroid.cfg`.

## Shiro836/waydroid-nvidia

**Architettura `main` (Venus)**, da `docs/architecture.md` e dal codice in `src/`:

```
App Android ── Vulkan ──▶ Mesa Venus guest ── socket unix ──▶ virglrenderer host (wd-venus)
                                                                     │ Vulkan reale
compositor ◀── hwcomposer ◀── gralloc (minigbm vtest) ◀── dmabuf NVIDIA block-linear
```

- GL ES tramite ANGLE sopra Venus; ASTC emulato via compute shader (le GPU
  desktop NVIDIA non lo supportano in hardware); ETC2 nel ramo `public-test`.
- Allocazione buffer **lato host** in formato block-linear NVIDIA: NVIDIA EGL
  importa dmabuf LINEAR di altri vendor solo come `GL_TEXTURE_EXTERNAL_OES`,
  quindi i buffer cross-vendor non sono visualizzabili da un compositor su
  NVIDIA. **Conseguenza diretta per i sistemi ibridi** (vedi report).
- Sincronizzazione completamente lato GPU: timeline syncobj DRM condivisi +
  import di `sync_fd`.
- Patch a Waydroid (`patches/waydroid/0001-nvidia-integration.patch`): mount del
  socket Venus in `lxc.py`, `suspend_action=none`.
- Requisiti dichiarati e verificati nel codice di setup: modulo kernel
  **open** (DMA-BUF), driver ≥ 595.71, `nvidia-drm.modeset=1`, Wayland con il
  compositor sulla GPU NVIDIA, accesso a `/dev/udmabuf` (regola udev `uaccess`).
- Testato su KDE Plasma 6 / KWin; GNOME/Mutter non citato.

**Ramo `public-test`** (v0.2.0-rc7): modalità `nvidia-prolapse` (NVK di Mesa +
backend NVIDIA dentro Android, senza proxy, decodifica video HW H.264/HEVC/VP9/AV1),
`venus`, `glibc-shim`. Tabella di compatibilità del README: RTX 40 + driver
615 open → `venus` ✅, `nvidia-prolapse` 🧪 (test driver superati, Waydroid
non ancora provato). Solo immagine Android 13 (LineageOS 20). Installer
`curl | sudo bash` che verifica i download.

**Riuso**: come backend GPU "NVIDIA Accelerated" (Venus, `main`) e "NVIDIA
Experimental" (`public-test`). Non ricompiliamo lo stack: usiamo i rilasci
firmati (attestazioni SLSA verificabili con `gh attestation verify`).

**Vincolo di installazione** (da `docs/install-manual.md` e `packaging/install.sh`
di `public-test`): entrambi i rami installano una **Waydroid patchata** (bind-mount
del socket Venus, `suspend_action = none`) compilata dal sorgente al commit
indicato in `patches/waydroid/BASE`. L'installer `public-test` verifica SHA-256 e
provenienza (cosign/gh) di ogni asset e si ferma se un file appartiene a un
pacchetto della distribuzione: il pacchetto `waydroid` di repo.waydro.id va
rimosso prima. Passare a un backend NVIDIA è quindi una sostituzione del runtime,
non una semplice opzione di configurazione.

## remote-android/redroid-doc (+ atxbot/redroid)

- Android in Docker (`--privileged`), binder via `modprobe binder_linux
  devices=binder,hwbinder,vndbinder`, parametri `androidboot.redroid_*`
  (risoluzione, DPI, `gpu_mode=auto|host|guest`, `gpu_node`).
- Pensato per uso remoto/headless (scrcpy/VNC), **non** per integrazione
  desktop finestra-per-app. Immagini binarie senza licenza dichiarata nel repo.
- `atxbot/redroid` è un fork fermo al 2023 senza modifiche proprie.

**Riuso**: candidato secondo backend (Architettura C) per scenari headless /
CI; non prioritario.

## fogelmanjg/redroid-nvidia

- Porta l'approccio Venus di waydroid-nvidia su ReDroid; aggiunge NVENC via
  CUDA external memory e un comando vtest `VCMD_ENCODE_RESOURCE`.
- **Risultato importante per Ada (RTX 40)**: su RTX 4060 SurfaceFlinger può
  abortire (`SIGABRT` in `vn_ring_submit_locked`) per un'asserzione nel driver
  kernel NVIDIA (`gpu_vaspace.c`), bug aperto upstream e riprodotto su 580 e
  595. Va trattato come rischio noto per la RTX 4070 / 4070 Ti SUPER.

**Riuso**: solo conoscenza (diagnosi, test). Licenza Apache-2.0 compatibile.

## waydroid-helper

- GTK4/libadwaita in Python; gestione estensioni (Magisk, libhoudini, libndk,
  GApps, microG), keymapper basato su scrcpy, cartelle condivise.
- Modello privilegi: binario `waydroid-cli` lanciato via **polkit**
  (`auth_admin_keep`) + servizio D-Bus `id.waydro.Mount` per i bind mount.
- **Riuso**: modello di privilegi (azioni polkit dedicate invece di shell
  arbitrarie) e idee UX. Codice GPL-3.0: non copiato.

## waydroid-x11-launcher

- Avvia `weston --backend=x11-backend.so` e vi esegue Waydroid; sincronizza la
  clipboard X11 ↔ Wayland con `xclip` + `wl-copy/wl-paste`.
- **Riuso**: strategia per le sessioni X11 (compositor annidato). Script MIT.

## waydroid-package-manager

- Script `wpm.sh` che scarica indici F-Droid e installa APK con `waydroid app
  install`. Non verifica le firme degli indici. Inattivo dal 2023.
- **Riuso**: solo l'idea di repository F-Droid come sorgente APK; un'eventuale
  implementazione dovrà verificare `index-v1.jar`/`entry.jar` firmati.

## Conclusioni tecniche

1. Waydroid è l'unico runtime maturo con integrazione desktop per-app: va
   usato come backend, non riscritto né forkato.
2. Il problema NVIDIA non si risolve in Waydroid stesso ma con uno stack
   grafico guest/host dedicato (waydroid-nvidia). DroidNative lo orchestra e
   **verifica** il risultato leggendo il renderer GLES reale di SurfaceFlinger.
3. Il punto di verità per "accelerazione attiva" è la riga `GLES:` di
   `dumpsys SurfaceFlinger` (SwiftShader/llvmpipe = software).
4. Sui sistemi ibridi conta **quale GPU pilota il compositor**, non solo
   quali GPU sono presenti.

## Verifiche end-to-end su questo PC (2026-10-19)

Waydroid 1.6.2 da repo.waydro.id, immagini LineageOS 20 del 2026-09-27, RTX 4070 Ti
SUPER (display, driver 615 open) + iGPU AMD Raphael, GNOME/Mutter su Wayland.

| Osservazione | Evidenza | Conseguenza in DroidNative |
|---|---|---|
| Waydroid sceglie l'iGPU AMD scartando la NVIDIA | `gralloc.gbm.device=/dev/dri/renderD128`, `ro.hardware.vulkan=radeon` | Previsto dall'analisi di `gpu.py` |
| Rendering hardware reale sull'iGPU | `GLES: AMD … (radeonsi, raphael_mendocino, ACO …), OpenGL ES 3.2 Mesa 26.0.8` | Verifica del renderer reale |
| Ma la visualizzazione fallisce: Mutter su NVIDIA rifiuta i buffer AMD, l'hwcomposer va in abort appena si apre una finestra | `wayland-hwc: failed to import supplied dmabufs: Could not bind the given EGLImage to a CoglTexture2D`, poi `SIGABRT` in `hwcomposer.waydroid.so` | Mesa Native "non supportato" con display su NVIDIA; la verifica GPU legge anche la salute della visualizzazione |
| `waydroid app list` mostra un package prima che Android ne completi l'installazione | Avvio subito dopo: `Package org.fdroid.fdroid is currently frozen!` mentre `dex2oat` è in corso | L'installazione è confermata solo quando Waydroid scrive `~/.local/share/applications/waydroid.<pkg>.desktop` (callback `packageStateChanged`, dopo il commit) |
| `waydroid app remove` fallisce sempre ed esce con 0 | `Failed with code: -3` = `EX_ILLEGAL_ARGUMENT` da `WayDroidService.removeApp` (stesso codice in `android_vendor_waydroid` base-patches-33 e in `android_frameworks_waydroid`) | Ripiego su `pm uninstall` tramite l'helper privilegiato, rimozione verificata |
| UFW attivo con INPUT/FORWARD DROP: Android non riceve un indirizzo | `udp dport 67 … jump ufw-skip-to-policy-input` (15 pacchetti scartati), lease file vuoto | Controllo "Rete di Android" e passo guidato con 3 regole limitate a `waydroid0` |
| Fallback software funzionante | `ANGLE (Google, Vulkan 1.2.0 (SwiftShader Device …))`, nessun errore di visualizzazione, app aperte in finestra | Riconosciuto e mostrato come "Rendering software" |
| waydroid-nvidia v0.2.0-rc7 installato da DroidNative (hash di `SHA256SUMS` e commit fissati; attestazioni GitHub `build.yml@refs/tags/v0.2.0-rc7` controllate per i 6 file) | purge di `waydroid` 1.6.2, Waydroid patchata 1.6.3, immagini e dati conservati | Passo guidato "Configurare il backend GPU" |
| Modalità venus sulla RTX 4070 Ti SUPER con GNOME/Mutter (upstream testa solo KWin) | `ANGLE (NVIDIA, Vulkan 1.3.351 (NVIDIA Virtio-GPU Venus (NVIDIA GeForce RTX 4070 Ti SUPER)))`; Settings e F-Droid aperte, nessun errore hwcomposer, nessun riavvio di SurfaceFlinger | NVIDIA Accelerated verificato |
| Modalità nvidia-prolapse (🧪 upstream per RTX 40) | gate di installazione 22/22 sul modulo 615.71.09; `ANGLE (NVIDIA, Vulkan 1.4.354 (NVIDIA GeForce RTX 4070 Ti SUPER (NVK NV193)))` | NVIDIA Experimental verificato |
| Il bug di `removeApp` resta anche in Waydroid 1.6.3 patchata | `Failed to uninstall package` | Ripiego `pm uninstall` confermato |
| Venus richiede il servizio utente `wd-venus` | socket `/run/waydroid-venus/venus.sock` | DroidNative lo avvia prima della sessione |
| `waydroid init -f -s GAPPS` riavvia il container fuori da systemd | servizio `inactive` ma `lxc-info` `RUNNING`; `waydroid-nvidia-setup`: "the Waydroid container did not stop" | Arresto verificato con `lxc-info` prima e dopo `init`, poi la modalità waydroid-nvidia viene riattivata |
| Immagine GAPPS con venus | system.img GAPPS 2,58 GB; `com.android.vending`, `com.google.android.gms` 23.10.15, `com.google.android.gsf`; stesso renderer `ANGLE (NVIDIA, … Venus …)`, nessun errore di visualizzazione | Passo "Servizi Google"; ID GSF letto da `gservices.db` |
| libndk (supremegamers, Android 13) installato nell'overlay | 177 file; `ro.product.cpu.abilist=x86_64,x86,arm64-v8a,armeabi-v7a,armeabi`, `ro.dalvik.vm.native.bridge=libndk_translation.so` | Passo "Traduzione ARM" |
| VLC 3.7.2 solo ARM | `primaryCpuAbi=arm64-v8a` e poi `armeabi-v7a`; nel processo `lib/arm64/libvlc.so` con `libndk_translation.so` (`lib/arm/libvlc.so` con `/system/lib/libndk_translation.so` a 32 bit); attività in primo piano, nessun `UnsatisfiedLinkError` né crash | ABI "tradotta" nell'analisi APK |
| libndk registra 4 gestori `binfmt_misc` nel kernel dell'host a ogni avvio di Android | `arm_exe`, `arm_dyn`, `arm64_exe`, `arm64_dyn`, interprete `/system/bin/ndk_translation_program_runner_binfmt_misc*`, flag `P` senza `F` | Documentato (precedenza su qemu-user-static); la rimozione li annulla, verificato anche dopo il riavvio di Android |
| Dopo un riavvio la GPU NVIDIA passa da `renderD129` a `renderD128` (ordine di caricamento di `nvidia` e `amdgpu`) e Android non parte più | `driver-mode verify-active`: "venus mode needs drm_device = /dev/dri/renderD128 … it is /dev/dri/renderD129"; `waydroid-container.service` fallito anche con `systemctl enable --now` | `droidnative-render-node.service` prima di ogni avvio del container: `drm_device` scelto per produttore PCI e `waydroid upgrade -o`; passo guidato e report indicano la causa. Verificato: `renderD129 → renderD128` sia con `systemctl start` (come all'avvio) sia dal passo guidato, poi venus e visualizzazione OK |
| `lxc-attach` (usato da `waydroid shell`) cambia proprietario e permessi dei file collegati a stdout/stderr | file di log diventati `root 0600` | DroidNative cattura sempre l'output con pipe; nei test manuali reindirizzare con `\| cat` |

Schermate catturate in Android con `screencap`: [venus](android-nvidia-venus-screencap.png),
[VLC solo ARM tramite libndk](android-arm-vlc-screencap.png).

Non osservato in queste prove: il crash del driver su Ada riportato da redroid-nvidia
(RTX 4060). Le prove sono brevi (apertura di app, non giochi o carichi prolungati).
