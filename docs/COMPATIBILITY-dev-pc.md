# DroidNative compatibility report

Host: **dev-pc** · AnduinOS 2.0.4 · kernel 7.0.0-38-generic · DroidNative 0.4.0

**Result: compatible**

## System

| | Check | Detail |
|---|---|---|
| ✅ | Distribution | AnduinOS 2.0.4 — base Ubuntu 26.04 (resolute) |
| ✅ | CPU | AMD Ryzen 7 7700X 8-Core Processor · 16 thread · x86_64 |
| ✅ | Memory | 30.5 GiB total, 21.3 GiB available |
| ✅ | Disk space | 23.4 GiB free for /var/lib/waydroid |
| ✅ | systemd | systemd 259 (259.9-0ubuntu0.1) |

## Kernel and container

| | Check | Detail |
|---|---|---|
| ℹ️ | Kernel | 7.0.0-38-generic |
| ✅ | Binder / binderfs | active (/dev/anbox-binder, /dev/anbox-hwbinder, /dev/anbox-vndbinder, /dev/binderfs/anbox-binder, /dev/binderfs/anbox-hwbinder, /dev/binderfs/anbox-vndbinder, /dev/binderfs/binder-control) |
| ✅ | Linux namespaces | cgroup, ipc, mnt, net, pid, pid_for_children, time, time_for_children, user, uts |
| ✅ | cgroup v2 | cpuset cpu io memory hugetlb pids rdma misc dmem |
| ✅ | Pressure Stall Information | active (needed by Android's lmkd) |
| ℹ️ | Security modules | lockdown, capability, landlock, yama, apparmor, ima, evm · AppArmor active (Waydroid profiles applied) |
| ℹ️ | KVM | available (not needed: Android runs in a container) |

## Graphics

| | Check | Detail |
|---|---|---|
| ✅ | Graphical session | Wayland · GNOME · Mutter |
| ℹ️ | NVIDIA GeForce RTX 4070 Ti SUPER | 0000:01:00.0 · driver nvidia · /dev/dri/renderD128 · 16.0 GiB VRAM · drives the display (DP-1) |
| ℹ️ | AMD Raphael | 0000:09:00.0 · driver amdgpu · /dev/dri/renderD129 · 0.5 GiB VRAM |
| ℹ️ | NVIDIA driver | 615.71.09 · open module · modeset on · GBM backend yes · egl-wayland yes |
| ℹ️ | Mesa | 26.0.8 (dpkg:libgl1-mesa-dri) |
| ℹ️ | Vulkan | loader 1.4.341.0 · ICD: asahi_icd 1.4.335, gfxstream_vk_icd 1.1.335, intel_hasvk_icd 1.3.335, intel_icd 1.4.335, lvp_icd 1.4.335, nouveau_icd 1.4.335, nvidia_icd 1.4.351, radeon_icd 1.4.335, virtio_icd 1.4.335 |
| ℹ️ | Vulkan device | NVIDIA GeForce RTX 4070 Ti SUPER · NVIDIA · API 1.4.351 |
| ℹ️ | Vulkan device | AMD Ryzen 7 7700X 8-Core Processor (RADV RAPHAEL_MENDOCINO) · radv · API 1.4.335 |
| ℹ️ | Vulkan device | llvmpipe (LLVM 21.1.8, 256 bits) · llvmpipe · API 1.4.335 |
| ℹ️ | Host OpenGL | NVIDIA GeForce RTX 4070 Ti SUPER/PCIe/SSE2 · 4.6.0 NVIDIA 615.71.09 |
| ℹ️ | /dev/udmabuf | present and accessible |

## Android runtime

| | Check | Detail |
|---|---|---|
| ✅ | LXC | 6.0.6 |
| ✅ | Waydroid | 1.6.3 (/usr/bin/waydroid) |
| ✅ | Android images | 2 images · MAINLINE |
| ✅ | Container service | waydroid-container.service active |
| ✅ | Container network | dnsmasq + nftables/iptables present |
| ✅ | Android network | address 192.168.240.112 assigned by the container DHCP |
| ✅ | Google services | Play Store and Google Play Services present (GAPPS image) |
| ✅ | ARM translation | libhoudini active: ARM-only apps supported |
| ✅ | Privilege elevation (polkit) | /usr/bin/pkexec |
| ℹ️ | Existing installations | Waydroid config present; user data in ~/.local/share/waydroid; waydroid-nvidia (/usr/bin/waydroid-nvidia-setup) |

## GPU backends

**Recommended: NVIDIA Accelerated** — The display is on the NVIDIA GPU and Venus' requirements are met: it's the most tested hardware path for this setup.

### Mesa Native — not supported on this system

Android renders directly on the GPU render node through Mesa (gbm gralloc + Mesa EGL).

Device: AMD Raphael (/dev/dri/renderD129)

- ✅ **Mesa-compatible GPU**: AMD Raphael (amdgpu) → /dev/dri/renderD129
- ✅ **Render node access**: allowed for the user
- ❌ **Compositor on the same GPU**: the display is on NVIDIA GeForce RTX 4070 Ti SUPER: the compositor cannot import AMD Raphael's buffers (the NVIDIA driver imports other vendors' LINEAR dmabufs only as GL_TEXTURE_EXTERNAL_OES). Seen on GNOME/Mutter: “failed to import supplied dmabufs: Could not bind the given EGLImage to a CoglTexture2D”, then the hwcomposer crashes as soon as Android opens a window. — *Connect the monitor to the AMD/Intel GPU, or use an NVIDIA backend or the software fallback*
- ✅ **Wayland session**: Mutter
- ✅ **Kernel binder**: available
- ✅ **Waydroid**: 1.6.3

### NVIDIA Accelerated — ready (acceleration to be verified at start)

Android's Vulkan is forwarded through Mesa Venus to a host render server on the NVIDIA driver; GL ES via ANGLE.

Device: NVIDIA GeForce RTX 4070 Ti SUPER (/dev/dri/renderD128)

- ✅ **Supported NVIDIA GPU**: NVIDIA GeForce RTX 4070 Ti SUPER — Ada architecture
- ⚠️ **Known driver bug on Ada**: redroid-nvidia reproduced a SurfaceFlinger SIGABRT (vn_ring_submit_locked) on an RTX 4060, caused by an NVIDIA kernel driver assertion; open upstream
- ✅ **Open kernel module**: open-gpu-kernel-modules (DMA-BUF available)
- ✅ **Driver ≥ 595.71**: 615.71.09
- ✅ **nvidia-drm.modeset=1**: on
- ✅ **Compositor on the NVIDIA GPU**: display on NVIDIA GeForce RTX 4070 Ti SUPER (DP-1)
- ✅ **Wayland session**: Mutter
- ⚠️ **Compositor tested upstream**: Mutter: upstream tests KWin / Plasma 6 only
- ✅ **Access to /dev/udmabuf**: allowed
- ✅ **Kernel binder**: available
- ✅ **Waydroid**: 1.6.3
- ✅ **waydroid-nvidia**: v0.2.0-rc7, venus mode active

### NVIDIA Experimental — compatible, needs installation

Open-source driver (NVK + NVIDIA backend) running directly inside Android, no proxy; hardware video decode.

Device: NVIDIA GeForce RTX 4070 Ti SUPER (/dev/dri/renderD128)

- ⚠️ **Supported NVIDIA GPU**: NVIDIA GeForce RTX 4070 Ti SUPER — Ada architecture: 🧪 upstream (Waydroid confirmed on RTX 50 only)
- ✅ **NVIDIA driver**: 615.71.09 (open module): ✅ upstream
- ✅ **nvidia-drm.modeset=1**: on
- ✅ **Compositor on the NVIDIA GPU**: display on NVIDIA GeForce RTX 4070 Ti SUPER (DP-1)
- ✅ **Wayland session**: Mutter
- ⚠️ **Compositor tested upstream**: Mutter: upstream tests KWin / Plasma 6 only
- ℹ️ **Android version**: official Android 13 image only (LineageOS 20)
- ✅ **Kernel binder**: available
- ✅ **Waydroid**: 1.6.3
- 📦 **waydroid-nvidia**: v0.2.0-rc7 installed, active mode: venus — *Select this backend: DroidNative runs waydroid-nvidia-setup --mode nvidia-prolapse*

### Software Fallback — ready (acceleration to be verified at start)

CPU rendering with SwiftShader. Always available, never reported as accelerated.

Device: CPU (SwiftShader)

- ⚠️ **Performance**: CPU rendering: games and 3D apps are slow, no acceleration
- ✅ **Wayland session**: Mutter
- ✅ **Kernel binder**: available
- ✅ **Waydroid**: 1.6.3

> No backend is reported active until DroidNative reads SurfaceFlinger's real renderer (`droidnative verify-gpu`).
