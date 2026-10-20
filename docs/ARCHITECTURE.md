# DroidNative architecture

## Choice: architecture B, ready for C

| Option | Assessment |
|---|---|
| A — Fork Waydroid | Rejected. Waydroid has 968 open issues and frequent releases; everything needed (state, apps, properties, GPU choice) is reachable through its CLI, D-Bus and `waydroid.cfg` without modifying it. A fork means rebasing forever with no technical gain. |
| **B — Independent runtime manager** | **Adopted.** DroidNative orchestrates Waydroid and the existing GPU stacks, and adds detection, acceleration verification, APK management and the UI. |
| C — Multiple backends | Prepared: the runtime sits behind the `AndroidRuntime` trait, the GPU behind the `GpuBackendKind` descriptors. ReDroid can be added as a second backend. |

## Components

```
┌───────────────────────────── app/ (Tauri 2) ─────────────────────────────┐
│ React + TypeScript (UI)  ──invoke──▶  src-tauri (Rust commands)          │
└──────────────────────────────────────┬───────────────────────────────────┘
                                       │ library
┌──────────────────────── crates/droidnative-core ─────────────────────────┐
│ host/      system detection (os-release, kernel, binder, cgroups, …)     │
│ gpu/       GPUs, driver stacks, GPU backends, acceleration check         │
│ runtime/   AndroidRuntime trait + Waydroid backend                       │
│ apk/       APK inspection (binary AndroidManifest, ABIs, SHA-256)        │
│ privileged root operations from a closed list                            │
│ extras     GAPPS image, ARM translation, Google device ID                │
│ report     compatibility report                                          │
└──────────────────────────────────────┬───────────────────────────────────┘
                     pkexec (polkit)   │   as the user
┌──────────────────────────────────────▼───────────────────────────────────┐
│ droidnative priv <op>  (same CLI binary, allowed operations only)        │
│ waydroid CLI · D-Bus id.waydro.Container · /var/lib/waydroid/waydroid.cfg│
└──────────────────────────────────────────────────────────────────────────┘
```

## Security model

- The UI runs **unprivileged**. Root operations are a closed `enum` (`PrivilegedOp`) run by
  `droidnative priv …` through `pkexec`: no arbitrary shell string crosses the privilege
  boundary.
- Every argument entering a privileged operation is validated (package names, property keys,
  real `/dev/dri/renderD*` nodes, translator names).
- External commands run without a shell, with `LC_ALL=C` (output is parsed deterministically
  on localized systems too) and timeouts.
- `waydroid shell`'s dangerous options (`-L`, `-C`, `-G`) are never used.
- APKs are inspected before installation (manifest, ABIs, SHA-256) and the install is
  verified against Android's own commit signal: Waydroid returns 0 even when it fails.
- Window mode and mouse-as-touch use `waydroid prop set` from the user session: no
  privileges involved.

## GPU backends

| Backend | Stack | When |
|---|---|---|
| Mesa Native | Mesa (radeonsi/RADV, iris/ANV) via `drm_device` + gralloc gbm | AMD, Intel; the GPU driving the compositor, or a verified dmabuf import |
| NVIDIA Accelerated | waydroid-nvidia `main`: Venus → host virglrenderer, GL ES through ANGLE | RTX 20+ with the open module, driver ≥ 595.71, modeset=1, compositor on NVIDIA |
| NVIDIA Experimental | waydroid-nvidia `public-test`: `nvidia-prolapse` (NVK + NVIDIA backend in the guest) | Same requirements, experimental component, Android 13 only |
| Software Fallback | SwiftShader | Always available, never reported as accelerated |

### Installing the NVIDIA stack (`nvidia_stack.rs`)

1. The release is **pinned in the code** (`PINNED_RELEASES`): tag, SHA-256 of the
   `SHA256SUMS` file and source commit. The values were pinned after checking the GitHub build
   attestation of every file (`build.yml` at the tag).
2. As root, in `/var/lib/droidnative/nvidia-stack/<tag>` (0700): download `SHA256SUMS` (hash
   compared with the pinned value), the 6 files (each compared with its line) and the source
   archive (commit read from `git archive`'s pax header and compared with the pinned value).
   Files already present and valid are reused; a file that does not match is deleted.
3. Remove DroidNative's software overrides from `waydroid.cfg`, then `apt-get purge waydroid`
   (its `waydroid.menu` conffile would block the installer, which does not overwrite package
   files). Images and data are not package files.
4. `packaging/install.sh --tag <tag> --from <dir>` from the verified archive: the official
   installer checks hashes and the source/binary pair again and installs the patched Waydroid.
5. `waydroid-nvidia-setup --mode venus|nvidia-prolapse`; state recorded in
   `/var/lib/droidnative/nvidia-stack.json`.

In `venus` mode DroidNative starts the `wd-venus.service` user unit before the Android
session.

### The NVIDIA GPU's node at every boot (`droidnative-render-node.service`)

waydroid-nvidia writes the NVIDIA GPU's `/dev/dri/renderD<N>` node into `drm_device` only when
it applies a mode, while its `ExecStartPre` (`driver-mode verify-active`) only checks it. On
hybrid systems the numbers follow driver load order and can swap from one boot to the next:
the container then does not start. The package installs `droidnative-render-node.service`
(oneshot, `ConditionPathExists` on `waydroid-nvidia-setup`, `After=` the
`sys-subsystem-waydroid_nvidia-render` device) and a `waydroid-container.service` drop-in that
wants it and orders itself after it. The service runs `droidnative priv repin-render-node`: in
venus mode it picks the node by PCI vendor (`0x10de`, like waydroid-nvidia), updates
`drm_device` and regenerates the LXC nodes and properties with `waydroid upgrade -o`, which uses
neither D-Bus nor systemd on a stopped container. It never calls `systemctl` on the container:
a `stop` or `start` at that moment would collide with the start job waiting for it. From the
UI the same operation with `--start` then starts the container.

### Direct subsurface outside KWin (`nvidia_stack::apply_subsurface_policy`)

`waydroid-nvidia-setup` writes `persist.waydroid.use_subsurface = true` into `[properties]` on
every run. That path hands Android's layer buffers to the compositor as Wayland subsurfaces and
is tested on KWin only: on GNOME/Mutter the composer HAL can abort and take SurfaceFlinger down
(upstream issue #21, seen here as `HidlComposer::createLayer … DEAD_OBJECT`) and windows leave
trails. After every `waydroid-nvidia-setup` it runs (install, mode switch, image switch),
DroidNative sets the property back to `false` unless a `kwin_wayland` process is running (the
helper runs as root without the user's environment, so it looks at `/proc`), then regenerates
the properties with `waydroid upgrade -o`. SurfaceFlinger then composes through its
RenderEngine, still ANGLE on Venus. The report and the GPU setup step flag a `true` value on a
compositor other than KWin and offer `SetSubsurface { enabled: false }`.

## Google Play and ARM translation (`extras.rs`)

**Image switch** (`SwitchSystemImage`): `waydroid init -f -s GAPPS|VANILLA`. Before and after,
DroidNative makes sure the LXC container is stopped (`lxc-info`, then `systemctl stop` and
`waydroid container stop`): `waydroid init` restarts the container outside systemd and
`waydroid-nvidia-setup` would refuse to proceed. With waydroid-nvidia, the mode read from
`waydroid.cfg` before the switch is re-activated, because `init -f` rewrites the vendor
properties. `init -f` keeps `[properties]` and the overlay (`/var/lib/waydroid/overlay`), so
the ARM translator too; Android data stays.

**ARM translators** (`InstallArmTranslation { translator }`): Google's libndk and Intel's
libhoudini, the supremegamers Android 13 archives pinned by commit and SHA-256 (the ones
waydroid_script uses, whose MD5 matches). As root, in `/var/lib/droidnative/libndk` (0700,
cache for both): download only when the cached file does not match, hash check, then removal
of the previous translator's files and of its `binfmt_misc` handlers (the new ones register
under the same names), extraction of the `*/prebuilts/` entries only into the system overlay,
rejecting paths that leave the destination, `root:shell 0755` permissions for `bin/` and
`root:root 0644` for the rest, list of the installed files in
`/var/lib/droidnative/libndk-files.txt`. libhoudini's own `houdini.rc` reads its binfmt files
from `/vendor/etc`, while in Waydroid they sit in the system overlay: DroidNative writes an
`etc/init/houdini.rc` that reads them from `/system/etc`. The properties
(`ro.product.cpu.abilist*`, `ro.dalvik.vm.native.bridge`, `ro.enable.native.bridge.exec*`,
`ro.dalvik.vm.isa.arm*`) go into `[properties]`, followed by `waydroid upgrade -o` and a
container restart. Removal deletes only the listed files and the directories left empty,
drops the properties and unregisters the handlers the translator's init script registers in
the host kernel at every Android boot. The default is libndk on AMD and libhoudini on Intel;
some apps run with only one of them (Age of Empires Mobile aborts in libndk with “Guest call
didn't restore sp” and runs with libhoudini).

**Google device ID** (`GoogleDeviceId`): `sqlite3` in the container on Google Services
Framework's `gservices.db`. The user registers it at google.com/android/uncertified with
their own account; the app only opens that URL, the only one the `opener:allow-open-url`
capability allows.

## Windows and input (`runtime/waydroid.rs`)

**Window mode**: `persist.waydroid.multi_windows`. With `true`, Waydroid's init bind-mounts
`pc.xml` (`android.hardware.type.pc`, `android.software.freeform_window_management`) and the
hwcomposer gives every task its own Wayland toplevel; tasks open in `freeform` mode. Android
reads the features at boot, so the change restarts the session. With waydroid-nvidia on a
compositor other than KWin the mode is refused (`per_app_windows_refusal`): on GNOME moving a
window leaves trails over the whole desktop, with the direct subsurface on or off, and the IME
becomes a toplevel of its own that takes the keyboard focus.

**The single window on GNOME**: waydroid-nvidia's hwcomposer creates an undecorated, maximized
toplevel. `persist.waydroid.window_decorations=true` asks for server-side decorations through
`zxdg_decoration_manager_v1`, which Mutter does not implement: the window then becomes a normal
(non-maximized) toplevel but still has no title bar or borders. `persist.waydroid.width/height`
make `choose_width_height` ignore the compositor's size hints, so they lock the window size.
Running the session inside gamescope (`--backend wayland --expose-wayland`, `WAYLAND_DISPLAY`
set to its socket) works and gamescope draws a libdecor title bar, but its nested resolution is
fixed, so Android is scaled into the window with black bands. DroidNative keeps Waydroid's
default maximized window.

**Mouse as touch**: `persist.waydroid.fake_touch`, a comma-separated list of packages (`*`
globs allowed). Waydroid's `ViewRootImpl` patch reads it when an app creates a window and then
rewrites the source of that window's mouse down/move/up events to touchscreen, so the change
applies the next time the app opens. Android caps non-`ro.` property values at 91 characters;
DroidNative refuses a list that would not fit instead of truncating it.

Acceleration counts as **active only after verification**: reading the `GLES:` line of
`dumpsys SurfaceFlinger` in the container. The `ro.hardware.egl`/`gralloc` properties only
describe the configuration.
