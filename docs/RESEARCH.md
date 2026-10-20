# Preliminary research — open source projects

Analysis carried out on 2026-10-19 by reading the **source code** (shallow clones of the
repositories, not just the READMEs). Metadata from the GitHub API on the same date.

## Summary

| Project | Language | License | Last push | Open issues | Role for DroidNative |
|---|---|---|---|---|---|
| waydroid/waydroid | Python | GPL-3.0 | 2026-10-16 | 968 | **Runtime backend** (reused as is, not forked) |
| Shiro836/waydroid-nvidia | C + patches | MIT (+ upstream licenses on the patches) | 2026-10-18 | 22 | **NVIDIA GPU backend** (stable Venus + `public-test`) |
| remote-android/redroid-doc | Shell/docs | not stated | 2026-05-17 | 359 | Possible future backend (architecture C) |
| atxbot/redroid | — | — | 2023-12-13 | 0 | Static fork of redroid-doc, nothing to reuse |
| fogelmanjg/redroid-nvidia | C | Apache-2.0 | 2026-09-27 | 0 | Technical reference (NVENC, Ada driver bug) |
| waydroid-helper/waydroid-helper | Python/GTK4 | GPL-3.0 | 2026-09-12 | 46 | UX reference (extensions, keymapper, polkit) |
| Iskierek100/waydroid-x11-launcher | Shell | MIT | 2025-08-15 | 0 | Reference for X11 support (nested weston) |
| waydroid/waydroid-package-manager | Shell | GPL-3.0 | 2023-11-06 | 2 | Reference for an F-Droid repository (inactive) |

## waydroid/waydroid

**Architecture (from `tools/`)**: the `waydroid` Python CLI, on three levels.

- *Container* (root): `waydroid container start`, the `waydroid-container.service` systemd
  unit, system D-Bus name `id.waydro.Container`, object `/ContainerManager`, interface
  `id.waydro.ContainerManager` with the methods `Start(a{ss})`, `Stop(b)`, `Freeze()`,
  `Unfreeze()`, `GetSession() → a{ss}`. The D-Bus policy (`dbus/id.waydro.Container.conf`)
  lets **any user** send messages: the state is readable without privileges.
- *Session* (user): `waydroid session start` stays in the foreground and owns
  `id.waydro.Session` on the session bus.
- *Android*: `app install/remove/launch/list` and `prop get/set` go through the `IPlatform`
  binder service inside Android via gbinder (`tools/interfaces/IPlatform.py`).

**Relevant behaviors found in the code**

- `tools/helpers/gpu.py`: `unsupported = ["nvidia"]` — the NVIDIA render node is **always
  skipped** and Waydroid picks the first non-NVIDIA `renderD*`. On a hybrid NVIDIA + AMD iGPU
  system it therefore picks the iGPU.
- The GPU is forced with the `drm_device` key in the `[waydroid]` section of
  `/var/lib/waydroid/waydroid.cfg`.
- `tools/helpers/lxc.py`: with a DRM node it sets `ro.hardware.gralloc=gbm`,
  `ro.hardware.egl=mesa`, `gralloc.gbm.device=<node>`; without one it falls back to
  `gralloc=default` + `egl=swiftshader` (software rendering). The Vulkan driver is derived from
  the kernel driver (`amdgpu→radeon`, `i915/xe→intel`, …).
- `tools/actions/app_manager.py`: `install()` logs the error with `logging.error` but **exits
  with code 0** even when it fails. A frontend must verify the install on its own (for example
  by matching the APK's package against `waydroid app list`).
- `tools/helpers/images.py`: OTA images are checked with SHA-256 against the channel's `id`
  field (`ota.waydro.id`). We reuse this check.
- `tools/helpers/arch.py`: supports x86, x86_64, arm, arm64; requires SSSE3 on x86.
- Wayland is required: the session uses `WAYLAND_DISPLAY`; on X11 a nested compositor is
  needed.
- Security: AppArmor profiles (`data/configs/apparmor_profiles`), seccomp
  (`waydroid.seccomp`); `waydroid shell -L/-C/-G` disable LSMs, capabilities and cgroups —
  DroidNative must never use them.
- Package: not in the Ubuntu 26.04 repositories; available from the official `repo.waydro.id`
  repository (suite `resolute` present, signed with key `7CE0331F71E0A238`).

**Reuse**: as the runtime, as is. A fork (architecture A) would mean maintaining 82 Python
files with 968 open issues and frequent releases: high cost, low benefit, because everything
needed is reachable through the CLI, D-Bus and `waydroid.cfg`.

## Shiro836/waydroid-nvidia

**`main` architecture (Venus)**, from `docs/architecture.md` and the code in `src/`:

```
Android app ── Vulkan ──▶ guest Mesa Venus ── unix socket ──▶ host virglrenderer (wd-venus)
                                                                     │ real Vulkan
compositor ◀── hwcomposer ◀── gralloc (minigbm vtest) ◀── NVIDIA block-linear dmabuf
```

- GL ES through ANGLE on top of Venus; ASTC emulated with a compute shader (NVIDIA desktop GPUs
  lack it in hardware); ETC2 on the `public-test` branch.
- Buffers allocated **on the host** in NVIDIA's block-linear format: NVIDIA EGL imports other
  vendors' LINEAR dmabufs only as `GL_TEXTURE_EXTERNAL_OES`, so cross-vendor buffers cannot be
  displayed by a compositor running on NVIDIA. **A direct consequence for hybrid systems** (see
  the report).
- Synchronization entirely on the GPU: shared DRM timeline syncobjs + `sync_fd` import.
- Patches to Waydroid (`patches/waydroid/0001-nvidia-integration.patch`): Venus socket mount in
  `lxc.py`, `suspend_action=none`.
- Requirements stated and checked in the setup code: **open** kernel module (DMA-BUF), driver
  ≥ 595.71, `nvidia-drm.modeset=1`, Wayland with the compositor on the NVIDIA GPU, access to
  `/dev/udmabuf` (udev `uaccess` rule).
- Tested on KDE Plasma 6 / KWin; GNOME/Mutter not mentioned.

**`public-test` branch** (v0.2.0-rc7): `nvidia-prolapse` mode (Mesa's NVK + an NVIDIA backend
inside Android, no proxy, H.264/HEVC/VP9/AV1 hardware video decoding), `venus`, `glibc-shim`.
README compatibility table: RTX 40 + open driver 615 → `venus` ✅, `nvidia-prolapse` 🧪 (driver
tests pass, Waydroid not tried yet). Android 13 image only (LineageOS 20). A `curl | sudo bash`
installer that verifies its downloads.

**Reuse**: as the “NVIDIA Accelerated” (Venus, `main`) and “NVIDIA Experimental”
(`public-test`) GPU backends. We do not rebuild the stack: we use the signed releases (SLSA
attestations checkable with `gh attestation verify`).

**Install constraint** (from `docs/install-manual.md` and `public-test`'s
`packaging/install.sh`): both branches install a **patched Waydroid** (Venus socket bind mount,
`suspend_action = none`) built from source at the commit named in `patches/waydroid/BASE`. The
`public-test` installer checks the SHA-256 and provenance (cosign/gh) of every asset and stops
if a file belongs to a distribution package: repo.waydro.id's `waydroid` package must be
removed first. Moving to an NVIDIA backend therefore replaces the runtime; it is not just a
configuration option.

## remote-android/redroid-doc (+ atxbot/redroid)

- Android in Docker (`--privileged`), binder through `modprobe binder_linux
  devices=binder,hwbinder,vndbinder`, `androidboot.redroid_*` parameters (resolution, DPI,
  `gpu_mode=auto|host|guest`, `gpu_node`).
- Meant for remote/headless use (scrcpy/VNC), **not** for window-per-app desktop integration.
  Binary images with no license stated in the repository.
- `atxbot/redroid` is a fork frozen in 2023 with no changes of its own.

**Reuse**: candidate second backend (architecture C) for headless / CI scenarios; not a
priority.

## fogelmanjg/redroid-nvidia

- Brings waydroid-nvidia's Venus approach to ReDroid; adds NVENC through CUDA external memory
  and a `VCMD_ENCODE_RESOURCE` vtest command.
- **Important finding for Ada (RTX 40)**: on an RTX 4060 SurfaceFlinger can abort (`SIGABRT` in
  `vn_ring_submit_locked`) because of an assertion in the NVIDIA kernel driver
  (`gpu_vaspace.c`), an open upstream bug reproduced on 580 and 595. To be treated as a known
  risk for the RTX 4070 / 4070 Ti SUPER.

**Reuse**: knowledge only (diagnosis, tests). Apache-2.0 license, compatible.

## waydroid-helper

- GTK4/libadwaita in Python; manages extensions (Magisk, libhoudini, libndk, GApps, microG), a
  keymapper based on scrcpy, shared folders.
- Privilege model: a `waydroid-cli` binary started through **polkit** (`auth_admin_keep`) + an
  `id.waydro.Mount` D-Bus service for bind mounts.
- **Reuse**: the privilege model (dedicated polkit actions instead of arbitrary shells) and UX
  ideas. GPL-3.0 code: not copied.

## waydroid-x11-launcher

- Starts `weston --backend=x11-backend.so` and runs Waydroid inside it; syncs the X11 ↔ Wayland
  clipboard with `xclip` + `wl-copy/wl-paste`.
- **Reuse**: the strategy for X11 sessions (nested compositor). MIT script.

## waydroid-package-manager

- A `wpm.sh` script that downloads F-Droid indexes and installs APKs with `waydroid app
  install`. It does not verify the index signatures. Inactive since 2023.
- **Reuse**: only the idea of F-Droid repositories as an APK source; an implementation will
  have to verify the signed `index-v1.jar`/`entry.jar`.

## Technical conclusions

1. Waydroid is the only mature runtime with per-app desktop integration: it must be used as
   the backend, not rewritten or forked.
2. The NVIDIA problem is not solved in Waydroid itself but by a dedicated guest/host graphics
   stack (waydroid-nvidia). DroidNative orchestrates it and **verifies** the result by reading
   SurfaceFlinger's real GLES renderer.
3. The source of truth for “acceleration active” is the `GLES:` line of `dumpsys
   SurfaceFlinger` (SwiftShader/llvmpipe = software).
4. On hybrid systems what matters is **which GPU drives the compositor**, not just which GPUs
   are present.

## End-to-end checks on the development PC

2026-10-19 and 2026-10-20. Waydroid 1.6.2 from repo.waydro.id, LineageOS 20 images of
2026-09-27, RTX 4070 Ti SUPER (display, open driver 615) + AMD Raphael iGPU, GNOME/Mutter on
Wayland.

| Observation | Evidence | Consequence in DroidNative |
|---|---|---|
| Waydroid picks the AMD iGPU, skipping the NVIDIA GPU | `gralloc.gbm.device=/dev/dri/renderD128`, `ro.hardware.vulkan=radeon` | As predicted from `gpu.py` |
| Real hardware rendering on the iGPU | `GLES: AMD … (radeonsi, raphael_mendocino, ACO …), OpenGL ES 3.2 Mesa 26.0.8` | Check of the real renderer |
| But display fails: Mutter on NVIDIA rejects the AMD buffers, the hwcomposer aborts as soon as a window opens | `wayland-hwc: failed to import supplied dmabufs: Could not bind the given EGLImage to a CoglTexture2D`, then `SIGABRT` in `hwcomposer.waydroid.so` | Mesa Native “not supported” with the display on NVIDIA; the GPU check also reads display health |
| `waydroid app list` shows a package before Android completes its installation | Launching right after: `Package org.fdroid.fdroid is currently frozen!` while `dex2oat` runs | An install is confirmed only when Waydroid writes `~/.local/share/applications/waydroid.<pkg>.desktop` (the `packageStateChanged` callback, after the commit) |
| `waydroid app remove` always fails and exits with 0 | `Failed with code: -3` = `EX_ILLEGAL_ARGUMENT` from `WayDroidService.removeApp` (same code in `android_vendor_waydroid` base-patches-33 and in `android_frameworks_waydroid`) | Falls back to `pm uninstall` through the privileged helper, removal verified |
| UFW on with INPUT/FORWARD DROP: Android gets no address | `udp dport 67 … jump ufw-skip-to-policy-input` (15 packets dropped), empty lease file | “Android network” check and a guided step with 3 rules limited to `waydroid0` |
| Software fallback works | `ANGLE (Google, Vulkan 1.2.0 (SwiftShader Device …))`, no display errors, apps open in a window | Recognized and shown as “Software rendering” |
| waydroid-nvidia v0.2.0-rc7 installed by DroidNative (pinned `SHA256SUMS` hash and commit; GitHub attestations `build.yml@refs/tags/v0.2.0-rc7` checked for the 6 files) | purge of `waydroid` 1.6.2, patched Waydroid 1.6.3, images and data kept | “Set up the GPU backend” guided step |
| venus mode on the RTX 4070 Ti SUPER with GNOME/Mutter (upstream tests KWin only) | `ANGLE (NVIDIA, Vulkan 1.3.351 (NVIDIA Virtio-GPU Venus (NVIDIA GeForce RTX 4070 Ti SUPER)))`; Settings and F-Droid opened, no hwcomposer errors, no SurfaceFlinger restart | NVIDIA Accelerated verified |
| nvidia-prolapse mode (🧪 upstream for RTX 40) | install gate 22/22 on module 615.71.09; `ANGLE (NVIDIA, Vulkan 1.4.354 (NVIDIA GeForce RTX 4070 Ti SUPER (NVK NV193)))` | NVIDIA Experimental verified |
| The `removeApp` bug is still there in the patched Waydroid 1.6.3 | `Failed to uninstall package` | `pm uninstall` fallback confirmed |
| Venus needs the `wd-venus` user unit | socket `/run/waydroid-venus/venus.sock` | DroidNative starts it before the session |
| `waydroid init -f -s GAPPS` restarts the container outside systemd | unit `inactive` but `lxc-info` `RUNNING`; `waydroid-nvidia-setup`: “the Waydroid container did not stop” | Stop verified with `lxc-info` before and after `init`, then the waydroid-nvidia mode is re-activated |
| GAPPS image with venus | GAPPS system.img 2.58 GB; `com.android.vending`, `com.google.android.gms` 23.10.15, `com.google.android.gsf`; same `ANGLE (NVIDIA, … Venus …)` renderer, no display errors | “Google services” step; GSF ID read from `gservices.db` |
| libndk (supremegamers, Android 13) installed in the overlay | 177 files; `ro.product.cpu.abilist=x86_64,x86,arm64-v8a,armeabi-v7a,armeabi`, `ro.dalvik.vm.native.bridge=libndk_translation.so` | “ARM translation” step |
| ARM-only VLC 3.7.2 | `primaryCpuAbi=arm64-v8a` and then `armeabi-v7a`; in the process `lib/arm64/libvlc.so` with `libndk_translation.so` (`lib/arm/libvlc.so` with `/system/lib/libndk_translation.so` on 32 bit); activity in the foreground, no `UnsatisfiedLinkError` or crash | “Translated” ABI in the APK analysis |
| libndk registers 4 `binfmt_misc` handlers in the host kernel at every Android boot | `arm_exe`, `arm_dyn`, `arm64_exe`, `arm64_dyn`, interpreter `/system/bin/ndk_translation_program_runner_binfmt_misc*`, flag `P` without `F` | Documented (precedence over qemu-user-static); removal unregisters them, also verified after an Android restart |
| After a reboot the NVIDIA GPU moves from `renderD129` to `renderD128` (load order of `nvidia` and `amdgpu`) and Android no longer starts | `driver-mode verify-active`: “venus mode needs drm_device = /dev/dri/renderD128 … it is /dev/dri/renderD129”; `waydroid-container.service` failed, `systemctl enable --now` too | `droidnative-render-node.service` before every container start: `drm_device` picked by PCI vendor and `waydroid upgrade -o`; guided step and report name the cause. Verified: `renderD129 → renderD128` both through `systemctl start` (as at boot) and from the guided step, then venus and display OK |
| `lxc-attach` (used by `waydroid shell`) changes owner and mode of the files bound to stdout/stderr | log files turned into `root 0600` | DroidNative always captures output through pipes; in manual tests redirect with `\| cat` |
| Age of Empires Mobile (ARM64 only, Tencent ACE anti-cheat `libanogs.so`) with libndk | `SIGABRT` 11 s after launch: “Guest call didn't restore sp: expected 0x…fd0, actual 0x…fc0” in `ndk_translation::ExecuteGuestCall`, in a native thread of the game, while it downloads resources | Translator choice: libhoudini as an alternative |
| The same game with libhoudini (supremegamers, Android 13, 650 files; the package's `houdini.rc` reads `/vendor/etc`, rewritten to `/system/etc`) | `Initialize library(version: 14.0.0_z.GoogleGame_com1.3 RELEASE)... successfully`; handlers with interpreter `/system/bin/houdini64`; the game runs, renders the tutorial and downloads its resources, no tombstones | “Use libhoudini” button; the library declares itself licensed for Google Play Games on Windows, stated in the docs |
| The game's first launch (with libndk) also brought SurfaceFlinger down once | `HidlComposer::createLayer` → `DEAD_OBJECT`: the composer HAL had died, Android restarted | Not reproduced in the following launches; kept as an observation |
| Unity games ignore the mouse | Waydroid delivers the pointer as a mouse; `persist.waydroid.fake_touch` (Waydroid's `ViewRootImpl` patch) rewrites the source of down/move/up to touchscreen, read when the window is created; values up to 91 characters | Per-app “Mouse as touch”, applies at the next app start |
| `persist.waydroid.multi_windows=true` | `pc.xml` mounted: `android.hardware.type.pc`, `android.software.freeform_window_management`; Settings opens as a `mode=freeform` task in its own window; the user resized it (bounds 464×824 → 842×934) | “Windows” card |
| …but with waydroid-nvidia on GNOME/Mutter (three 2560×1440 monitors at 240 Hz) the whole desktop glitches while an Android window is open | moving an Android window leaves trails of it over the desktop (photo from the user); the IME opens as a separate toplevel `waydroid.InputMethod` at every text field and takes the focus, so typed text seems lost; bursts of 47 `Can't update stage views actor` warnings in gnome-shell; no NVIDIA Xid, GPU at 15 % | In single-window mode the user confirmed it works. Turning off the IME (`show_ime_with_hard_keyboard=0`) and the direct subsurface did not remove the trails: per-app windows are refused with waydroid-nvidia outside KWin |
| waydroid-nvidia's direct subsurface (`persist.waydroid.use_subsurface=true`) on Mutter | upstream issue #21: the composer HAL aborts and SurfaceFlinger dies with `HidlComposer … DEAD_OBJECT`, the same as this PC's tombstone at Age of Empires' first launch; `waydroid-nvidia-setup` writes `true` on every run | DroidNative writes `false` after every setup unless `kwin_wayland` runs; acceleration stays (`ANGLE … Venus`), SurfaceFlinger composes |
| Resizing the single Android window on GNOME | waydroid-nvidia's hwcomposer opens an undecorated maximized toplevel; `persist.waydroid.window_decorations=true` → “compositor has no zxdg_decoration_manager_v1; window stays undecorated”, but the window is no longer maximized; with `persist.waydroid.width/height` set, `choose_width_height` ignores the compositor's sizes (window locked) | Not offered; documented |
| Android inside gamescope 3.16 (`--backend wayland --expose-wayland`, `WAYLAND_DISPLAY=gamescope-0`) | the session connects (`waydroid status` shows `gamescope-0`), 3 dmabufs imported, acceleration verified, libdecor title bar; the nested resolution stays fixed, so Android is scaled into the resized window with black bands | Not adopted |
| Android follows the monitors' refresh rate | `refreshRate=240.00005`, VSYNC period 4.17 ms | Noted (waydroid-nvidia caps it with `--refresh`) |

Screenshots captured in Android with `screencap`: [venus](android-nvidia-venus-screencap.png),
[ARM-only VLC through libndk](android-arm-vlc-screencap.png).

Not observed in these checks: the Ada driver crash reported by redroid-nvidia (RTX 4060). The
checks are short (opening apps, a game's first minutes, no long sessions).
