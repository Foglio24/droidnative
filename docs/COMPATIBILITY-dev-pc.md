# Report di compatibilità DroidNative

Host: **pc-di-sviluppo** · AnduinOS 2.0.4 · kernel 7.0.0-38-generic · DroidNative 0.1.0

**Esito: compatibile**

## Sistema

| | Controllo | Dettaglio |
|---|---|---|
| ✅ | Distribuzione | AnduinOS 2.0.4 — base Ubuntu 26.04 (resolute) |
| ✅ | CPU | AMD Ryzen 7 7700X 8-Core Processor · 16 thread · x86_64 |
| ✅ | Memoria RAM | 30.5 GiB totali, 22.1 GiB disponibili |
| ✅ | Spazio disco | 40.9 GiB liberi per /var/lib/waydroid |
| ✅ | systemd | systemd 259 (259.5-0ubuntu3.4) |

## Kernel e container

| | Controllo | Dettaglio |
|---|---|---|
| ℹ️ | Kernel | 7.0.0-38-generic |
| ✅ | Binder / binderfs | attivo (/dev/anbox-binder, /dev/anbox-hwbinder, /dev/anbox-vndbinder, /dev/binderfs/anbox-binder, /dev/binderfs/anbox-hwbinder, /dev/binderfs/anbox-vndbinder, /dev/binderfs/binder-control) |
| ✅ | Namespace Linux | cgroup, ipc, mnt, net, pid, pid_for_children, time, time_for_children, user, uts |
| ✅ | cgroup v2 | cpuset cpu io memory hugetlb pids rdma misc dmem |
| ✅ | Pressure Stall Information | attivo (necessario a lmkd di Android) |
| ℹ️ | Moduli di sicurezza | lockdown, capability, landlock, yama, apparmor, ima, evm · AppArmor attivo (profili Waydroid applicati) |
| ℹ️ | KVM | disponibile (non necessario: Android gira in container) |

## Grafica

| | Controllo | Dettaglio |
|---|---|---|
| ✅ | Sessione grafica | Wayland · GNOME · Mutter |
| ℹ️ | NVIDIA GeForce RTX 4070 Ti SUPER | 0000:01:00.0 · driver nvidia · /dev/dri/renderD129 · 16.0 GiB VRAM · pilota il display (DP-5) |
| ℹ️ | AMD Raphael | 0000:09:00.0 · driver amdgpu · /dev/dri/renderD128 · 0.5 GiB VRAM |
| ℹ️ | Driver NVIDIA | 615.71.09 · modulo open · modeset attivo · backend GBM sì · egl-wayland sì |
| ℹ️ | Mesa | 26.0.8 (dpkg:libgl1-mesa-dri) |
| ℹ️ | Vulkan | loader 1.4.341.0 · ICD: asahi_icd 1.4.335, gfxstream_vk_icd 1.1.335, intel_hasvk_icd 1.3.335, intel_icd 1.4.335, lvp_icd 1.4.335, nouveau_icd 1.4.335, nvidia_icd 1.4.351, radeon_icd 1.4.335, virtio_icd 1.4.335 |
| ℹ️ | vulkaninfo | non installato: dispositivi Vulkan non enumerati (pacchetto vulkan-tools) |
| ℹ️ | OpenGL host | glxinfo/eglinfo non installati: versione non rilevabile (pacchetto mesa-utils) |
| ℹ️ | /dev/udmabuf | presente e accessibile |

## Runtime Android

| | Controllo | Dettaglio |
|---|---|---|
| ✅ | LXC | 6.0.6 |
| ✅ | Waydroid | 1.6.3 (/usr/bin/waydroid) |
| ✅ | Immagini Android | 2 immagini · MAINLINE |
| ✅ | Servizio container | waydroid-container.service attivo |
| ✅ | Rete del container | dnsmasq + nftables/iptables presenti |
| ⚠️ | Rete di Android | firewall UFW attivo (ingresso DROP, inoltro DROP): blocca DHCP, DNS e inoltro su waydroid0, Android resta senza rete — *Installazione guidata → «Consentire la rete del container» (regole limitate a waydroid0)* |
| ✅ | Elevazione privilegi (polkit) | /usr/bin/pkexec |
| ℹ️ | Installazioni preesistenti | config Waydroid presente; dati utente in ~/.local/share/waydroid; waydroid-nvidia (/usr/bin/waydroid-nvidia-setup) |

## Backend GPU

**Consigliato: NVIDIA Accelerated** — Il display è sulla GPU NVIDIA e i requisiti di Venus sono soddisfatti: è il percorso hardware più collaudato per questa configurazione.

### Mesa Native — non supportato su questo sistema

Android usa direttamente il nodo render della GPU tramite Mesa (gralloc gbm + EGL mesa).

Dispositivo: AMD Raphael (/dev/dri/renderD128)

- ✅ **GPU compatibile Mesa**: AMD Raphael (amdgpu) → /dev/dri/renderD128
- ✅ **Accesso al nodo render**: consentito all'utente
- ❌ **Compositor sulla stessa GPU**: lo schermo è su NVIDIA GeForce RTX 4070 Ti SUPER: il compositor non riesce a importare i buffer di AMD Raphael (il driver NVIDIA importa dmabuf LINEAR di altri vendor solo come GL_TEXTURE_EXTERNAL_OES). Verificato su GNOME/Mutter: «failed to import supplied dmabufs: Could not bind the given EGLImage to a CoglTexture2D», poi crash dell'hwcomposer appena Android apre una finestra. — *Collega il monitor alla GPU AMD/Intel, oppure usa un backend NVIDIA o il fallback software*
- ✅ **Sessione Wayland**: Mutter
- ✅ **Binder nel kernel**: disponibile
- ✅ **Waydroid**: 1.6.3
- 📦 **Nodo GPU configurato**: /dev/dri/renderD129 invece di /dev/dri/renderD128

### NVIDIA Accelerated — pronto (accelerazione da verificare all'avvio)

Vulkan di Android inoltrato via Mesa Venus a un render server host sul driver NVIDIA; GL ES tramite ANGLE.

Dispositivo: NVIDIA GeForce RTX 4070 Ti SUPER (/dev/dri/renderD129)

- ✅ **GPU NVIDIA supportata**: NVIDIA GeForce RTX 4070 Ti SUPER — architettura Ada
- ⚠️ **Bug noto del driver su Ada**: redroid-nvidia ha riprodotto su RTX 4060 un SIGABRT di SurfaceFlinger (vn_ring_submit_locked) causato da un'asserzione nel driver kernel NVIDIA, aperto upstream
- ✅ **Modulo kernel open**: open-gpu-kernel-modules (DMA-BUF disponibile)
- ✅ **Driver ≥ 595.71**: 615.71.09
- ✅ **nvidia-drm.modeset=1**: attivo
- ✅ **Compositor sulla GPU NVIDIA**: schermo su NVIDIA GeForce RTX 4070 Ti SUPER (DP-5)
- ✅ **Sessione Wayland**: Mutter
- ⚠️ **Compositor testato upstream**: Mutter: upstream testa solo KWin / Plasma 6
- ✅ **Accesso a /dev/udmabuf**: consentito
- ✅ **Binder nel kernel**: disponibile
- ✅ **Waydroid**: 1.6.3
- ✅ **waydroid-nvidia**: v0.2.0-rc7, modalità venus attiva

### NVIDIA Experimental — compatibile, richiede installazione

Driver open-source (NVK + backend NVIDIA) eseguito direttamente dentro Android, senza proxy; decodifica video HW.

Dispositivo: NVIDIA GeForce RTX 4070 Ti SUPER (/dev/dri/renderD129)

- ⚠️ **GPU NVIDIA supportata**: NVIDIA GeForce RTX 4070 Ti SUPER — architettura Ada: 🧪 in upstream (Waydroid confermato solo su RTX 50)
- ✅ **Driver NVIDIA**: 615.71.09 (modulo open): ✅ in upstream
- ✅ **nvidia-drm.modeset=1**: attivo
- ✅ **Compositor sulla GPU NVIDIA**: schermo su NVIDIA GeForce RTX 4070 Ti SUPER (DP-5)
- ✅ **Sessione Wayland**: Mutter
- ⚠️ **Compositor testato upstream**: Mutter: upstream testa solo KWin / Plasma 6
- ℹ️ **Versione Android**: solo immagine ufficiale Android 13 (LineageOS 20)
- ✅ **Binder nel kernel**: disponibile
- ✅ **Waydroid**: 1.6.3
- 📦 **waydroid-nvidia**: v0.2.0-rc7 installato, modalità attiva: venus — *Seleziona questo backend: DroidNative esegue waydroid-nvidia-setup --mode nvidia-prolapse*

### Software Fallback — pronto (accelerazione da verificare all'avvio)

Rendering CPU con SwiftShader. Sempre disponibile, mai considerato accelerato.

Dispositivo: CPU (SwiftShader)

- ⚠️ **Prestazioni**: rendering su CPU: giochi e app 3D lenti, nessuna accelerazione
- ✅ **Sessione Wayland**: Mutter
- ✅ **Binder nel kernel**: disponibile
- ✅ **Waydroid**: 1.6.3

> Nessun backend è dichiarato attivo finché DroidNative non legge il renderer reale di SurfaceFlinger (`droidnative verify-gpu`).
