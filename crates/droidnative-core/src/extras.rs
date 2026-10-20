//! Android extras: Google services (official GAPPS system image) and ARM translation.
//!
//! GAPPS: `waydroid init -f -s GAPPS` swaps the system image and keeps apps and data; with the
//! waydroid-nvidia patched Waydroid, `waydroid-nvidia-setup` must run again afterwards or the
//! container refuses to start. Google then needs the device's GSF Android ID registered at
//! <https://www.google.com/android/uncertified> by the account owner.
//!
//! ARM translation: Google's libndk or Intel's libhoudini as packaged by supremegamers (the
//! archives casualsnek/waydroid_script installs for Android 13), pinned here by commit and
//! SHA-256, copied into Waydroid's static system overlay plus the native-bridge properties.

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::exec::{CmdOutput, CommandRunner};
use crate::fs::HostFs;
use crate::ini::Ini;
use crate::nvidia_stack::{configured_mode, SETUP_BIN};
use crate::privileged::{PrivError, SystemType};
use crate::runtime::waydroid::{CONTAINER_UNIT, WAYDROID_CFG};
use crate::tr;

pub const OVERLAY_SYSTEM: &str = "/var/lib/waydroid/overlay/system";
/// Cache of the verified archives (named after the first translator; holds both).
pub const TRANSLATION_CACHE: &str = "/var/lib/droidnative/libndk";
/// Files the active translator put in the overlay (path kept from 0.3.0 installs).
pub const TRANSLATION_MANIFEST: &str = "/var/lib/droidnative/libndk-files.txt";
pub const UNCERTIFIED_URL: &str = "https://www.google.com/android/uncertified";
const BINFMT_MISC: &str = "/proc/sys/fs/binfmt_misc";
/// Entries both translators' init scripts register in the host kernel (Android 13 packages).
const TRANSLATION_BINFMT: &[&str] = &["arm_exe", "arm_dyn", "arm64_exe", "arm64_dyn"];

pub struct PinnedArchive {
    pub url: &'static str,
    pub commit: &'static str,
    pub sha256: &'static str,
}

/// Android 13 build used by waydroid_script (`stuff/ndk.py`, md5 0b2207c490fcb400aa5c87fcf0d52d38).
pub const LIBNDK_A13: PinnedArchive = PinnedArchive {
    url: "https://github.com/supremegamers/vendor_google_proprietary_ndk_translation-prebuilt/archive/68734c52556d3d7a6db34c603dd9276915c29f2f.zip",
    commit: "68734c52556d3d7a6db34c603dd9276915c29f2f",
    sha256: "a142d1586c9eafb5edf62277110f2128bc03066179a6783bcaa33ee322e1cbd0",
};

/// Intel's libhoudini, Android 13 build used by waydroid_script (`stuff/houdini.py`,
/// md5 37fe0899f1e4da7f9a724cca1c1ab1ea).
pub const LIBHOUDINI_A13: PinnedArchive = PinnedArchive {
    url: "https://github.com/supremegamers/vendor_intel_proprietary_houdini/archive/2f8f088671182e17e67321e098e8411a3972a628.zip",
    commit: "2f8f088671182e17e67321e098e8411a3972a628",
    sha256: "2e82cdc88ddc4d418f7fb861aeebb7c49c83a91b97f57da10510b5e1146a4ed5",
};

pub const LIBNDK_PROPS: &[(&str, &str)] = &[
    ("ro.product.cpu.abilist", "x86_64,x86,arm64-v8a,armeabi-v7a,armeabi"),
    ("ro.product.cpu.abilist32", "x86,armeabi-v7a,armeabi"),
    ("ro.product.cpu.abilist64", "x86_64,arm64-v8a"),
    ("ro.dalvik.vm.native.bridge", "libndk_translation.so"),
    ("ro.enable.native.bridge.exec", "1"),
    ("ro.vendor.enable.native.bridge.exec", "1"),
    ("ro.vendor.enable.native.bridge.exec64", "1"),
    ("ro.ndk_translation.version", "0.2.3"),
    ("ro.dalvik.vm.isa.arm", "x86"),
    ("ro.dalvik.vm.isa.arm64", "x86_64"),
];

/// The package's houdini.mk properties plus the ABI lists Waydroid's image does not carry.
pub const LIBHOUDINI_PROPS: &[(&str, &str)] = &[
    ("ro.product.cpu.abilist", "x86_64,x86,arm64-v8a,armeabi-v7a,armeabi"),
    ("ro.product.cpu.abilist32", "x86,armeabi-v7a,armeabi"),
    ("ro.product.cpu.abilist64", "x86_64,arm64-v8a"),
    ("ro.dalvik.vm.native.bridge", "libhoudini.so"),
    ("ro.enable.native.bridge.exec", "1"),
    ("ro.enable.native.bridge.exec64", "1"),
    ("ro.dalvik.vm.isa.arm", "x86"),
    ("ro.dalvik.vm.isa.arm64", "x86_64"),
];

/// The package's houdini.rc reads its binfmt_misc files from /vendor/etc; in Waydroid they sit in
/// the system overlay (waydroid_script rewrites the file for the same reason).
const HOUDINI_RC: &str = "# Written by DroidNative: binfmt_misc files from the system overlay
on early-init
    mount binfmt_misc binfmt_misc /proc/sys/fs/binfmt_misc

on property:ro.enable.native.bridge.exec=1
    copy /system/etc/binfmt_misc/arm_exe /proc/sys/fs/binfmt_misc/register
    copy /system/etc/binfmt_misc/arm_dyn /proc/sys/fs/binfmt_misc/register

on property:ro.enable.native.bridge.exec64=1
    copy /system/etc/binfmt_misc/arm64_exe /proc/sys/fs/binfmt_misc/register
    copy /system/etc/binfmt_misc/arm64_dyn /proc/sys/fs/binfmt_misc/register
";

/// ARM→x86 translators Android 13 can load as native bridge. Some apps run only with one of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Translator {
    /// Google's libndk_translation (from Chromebooks).
    #[default]
    Libndk,
    /// Intel's libhoudini.
    Libhoudini,
}

impl Translator {
    pub const ALL: [Translator; 2] = [Translator::Libndk, Translator::Libhoudini];

    pub fn as_arg(self) -> &'static str {
        match self {
            Self::Libndk => "libndk",
            Self::Libhoudini => "libhoudini",
        }
    }

    pub fn from_arg(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_arg() == s)
    }

    pub fn archive(self) -> &'static PinnedArchive {
        match self {
            Self::Libndk => &LIBNDK_A13,
            Self::Libhoudini => &LIBHOUDINI_A13,
        }
    }

    /// The native-bridge library, as `ro.dalvik.vm.native.bridge` names it.
    pub fn library(self) -> &'static str {
        match self {
            Self::Libndk => "libndk_translation.so",
            Self::Libhoudini => "libhoudini.so",
        }
    }

    pub fn props(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Libndk => LIBNDK_PROPS,
            Self::Libhoudini => LIBHOUDINI_PROPS,
        }
    }

    /// Overlay files (relative to the system overlay) written over the archive's own.
    fn overrides(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Libndk => &[],
            Self::Libhoudini => &[("etc/init/houdini.rc", HOUDINI_RC)],
        }
    }

    /// The usual advice (Arch wiki, waydroid_script): libndk on AMD, libhoudini on Intel.
    pub fn recommended(cpu_vendor: Option<&str>) -> Self {
        if cpu_vendor.is_some_and(|v| v.contains("Intel")) {
            Self::Libhoudini
        } else {
            Self::Libndk
        }
    }

    pub fn other(self) -> Self {
        match self {
            Self::Libndk => Self::Libhoudini,
            Self::Libhoudini => Self::Libndk,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemImage {
    Vanilla,
    Gapps,
    Foss,
    Unknown,
}

/// From `system_ota = …/lineage/waydroid_x86_64/GAPPS.json`.
pub fn system_image(cfg_text: &str) -> SystemImage {
    let ota = Ini::parse(cfg_text).get("waydroid", "system_ota").unwrap_or_default().to_ascii_uppercase();
    if ota.ends_with("/GAPPS.JSON") {
        SystemImage::Gapps
    } else if ota.ends_with("/VANILLA.JSON") {
        SystemImage::Vanilla
    } else if ota.ends_with("/FOSS.JSON") {
        SystemImage::Foss
    } else {
        SystemImage::Unknown
    }
}

/// The active translator: its library sits in the overlay and the bridge property names it.
pub fn arm_translator(fs: &HostFs, cfg_text: &str) -> Option<Translator> {
    let ini = Ini::parse(cfg_text);
    let bridge = ini.get("properties", "ro.dalvik.vm.native.bridge")?;
    Translator::ALL.into_iter().find(|t| bridge == t.library() && fs.exists(format!("{OVERLAY_SYSTEM}/lib64/{}", t.library())))
}

pub fn arm_translation_installed(fs: &HostFs, cfg_text: &str) -> bool {
    arm_translator(fs, cfg_text).is_some()
}

fn step(cmd: &dyn CommandRunner, log: &mut String, program: &str, args: &[&str], timeout: Duration) -> Result<CmdOutput, PrivError> {
    log.push_str(&format!("$ {program} {}\n", args.join(" ")));
    let out = cmd.run_with_timeout(program, args, timeout)?;
    let text = out.combined();
    let lines: Vec<&str> = text.lines().collect();
    for l in &lines[lines.len().saturating_sub(30)..] {
        log.push_str(l);
        log.push('\n');
    }
    if !out.success() {
        return Err(PrivError::StepFailed {
            step: format!("{program} {}", args.join(" ")),
            output: if out.timed_out { "timeout".into() } else { text },
        });
    }
    Ok(out)
}

fn waydroid_bin(cmd: &dyn CommandRunner) -> Result<String, PrivError> {
    cmd.which("waydroid").map(|p| p.to_string_lossy().into_owned()).ok_or(PrivError::Missing("Waydroid"))
}

pub(crate) fn regenerate_props(fs: &HostFs, cmd: &dyn CommandRunner, log: &mut String) -> Result<(), PrivError> {
    let _ = fs;
    step(cmd, log, &waydroid_bin(cmd)?, &["upgrade", "-o"], Duration::from_secs(300))?;
    let active = cmd.run("systemctl", &["is-active", CONTAINER_UNIT]).map(|o| o.stdout.trim() == "active").unwrap_or(false);
    if active {
        step(cmd, log, "systemctl", &["restart", CONTAINER_UNIT], Duration::from_secs(120))?;
    }
    Ok(())
}

/// Stop the container whether systemd or `waydroid init` started it, and wait until LXC agrees.
/// `waydroid init -f` restarts a container that was running *outside* the systemd unit, which
/// waydroid-nvidia-setup then cannot stop ("the Waydroid container did not stop"): seen on hardware.
pub(crate) fn ensure_container_stopped(cmd: &dyn CommandRunner, log: &mut String, poll: Duration) -> Result<(), PrivError> {
    let state = || {
        cmd.run("lxc-info", &["-P", "/var/lib/waydroid/lxc", "-n", "waydroid", "-sH"])
            .map(|o| o.stdout.trim().to_string())
            .unwrap_or_default()
    };
    if matches!(state().as_str(), "" | "STOPPED") && !cmd.run("systemctl", &["is-active", CONTAINER_UNIT]).map(|o| o.stdout.trim() == "active").unwrap_or(false) {
        return Ok(());
    }
    let _ = cmd.run_with_timeout("systemctl", &["stop", CONTAINER_UNIT], Duration::from_secs(120));
    if state() != "STOPPED" {
        let _ = cmd.run_with_timeout(&waydroid_bin(cmd)?, &["container", "stop"], Duration::from_secs(120));
    }
    for _ in 0..60 {
        if matches!(state().as_str(), "" | "STOPPED") {
            log.push_str(&tr!("container Android fermato\n", "Android container stopped\n"));
            return Ok(());
        }
        std::thread::sleep(poll);
    }
    Err(PrivError::StepFailed {
        step: tr!("arresto del container", "container stop"),
        output: tr!("il container Waydroid non si è fermato entro 60 secondi", "the Waydroid container did not stop within 60 seconds"),
    })
}

/// Swap the Android system image (keeps apps and data), then re-provision waydroid-nvidia if present.
pub fn switch_system_image(target: SystemType, fs: &HostFs, cmd: &dyn CommandRunner) -> Result<CmdOutput, PrivError> {
    switch_system_image_with(target, fs, cmd, Duration::from_secs(1))
}

fn switch_system_image_with(target: SystemType, fs: &HostFs, cmd: &dyn CommandRunner, poll: Duration) -> Result<CmdOutput, PrivError> {
    let mut log = String::new();
    let cfg_before = std::fs::read_to_string(fs.path(WAYDROID_CFG))
        .map_err(|_| PrivError::Invalid(tr!("waydroid.cfg assente: inizializza prima Android", "waydroid.cfg missing: initialize Android first")))?;
    let nvidia_mode = configured_mode(&cfg_before);
    // A stopped container keeps `waydroid init` from restarting it behind systemd's back.
    ensure_container_stopped(cmd, &mut log, poll)?;
    step(cmd, &mut log, &waydroid_bin(cmd)?, &["init", "-f", "-s", target.as_arg()], Duration::from_secs(3600))?;
    ensure_container_stopped(cmd, &mut log, poll)?;
    if fs.exists(SETUP_BIN) {
        let mode = nvidia_mode.map(|m| m.as_arg()).unwrap_or("venus");
        log.push_str(&tr!(
            "waydroid-nvidia: riattivo la modalità {mode} dopo il cambio di immagine\n",
            "waydroid-nvidia: re-activating {mode} mode after the image switch\n"
        ));
        step(cmd, &mut log, SETUP_BIN, &["--mode", mode], Duration::from_secs(1800))?;
        crate::nvidia_stack::apply_subsurface_policy(fs, cmd, &mut log)?;
    } else {
        step(cmd, &mut log, "systemctl", &["start", CONTAINER_UNIT], Duration::from_secs(120))?;
    }
    Ok(CmdOutput::ok(log))
}

/// Archive path inside the zip → destination under the overlay, rejecting traversal.
fn overlay_target(entry: &str, overlay: &Path) -> Option<PathBuf> {
    let (_, rest) = entry.split_once("/prebuilts/")?;
    if rest.is_empty() {
        return None;
    }
    let rel = Path::new(rest);
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    Some(overlay.join(rel))
}

pub fn install_translation(t: Translator, fs: &HostFs, cmd: &dyn CommandRunner) -> Result<CmdOutput, PrivError> {
    install_archive(t, t.archive(), fs, cmd)
}

fn install_archive(t: Translator, archive: &PinnedArchive, fs: &HostFs, cmd: &dyn CommandRunner) -> Result<CmdOutput, PrivError> {
    use std::os::unix::fs::PermissionsExt;
    let mut log = String::new();
    let name = t.as_arg();
    let fail = |what: String| PrivError::StepFailed { step: tr!("verifica di {name}", "{name} check"), output: what };

    let work = fs.path(TRANSLATION_CACHE);
    std::fs::create_dir_all(&work)?;
    std::fs::set_permissions(&work, std::fs::Permissions::from_mode(0o700))?;
    let zip_path = work.join(format!("{}.zip", archive.commit));
    if crate::nvidia_stack::sha256_file(&zip_path).ok().as_deref() != Some(archive.sha256) {
        let dest = zip_path.to_string_lossy().into_owned();
        step(cmd, &mut log, "curl", &["--fail", "--location", "--proto", "=https", "--proto-redir", "=https", "--retry", "2", "-sS", "-o", &dest, archive.url], Duration::from_secs(900))?;
    }
    let got = crate::nvidia_stack::sha256_file(&zip_path).map_err(|e| fail(e.to_string()))?;
    if got != archive.sha256 {
        let _ = std::fs::remove_file(&zip_path);
        let want = archive.sha256;
        return Err(fail(tr!("archivio con hash {got}, atteso {want}", "archive hash {got}, expected {want}")));
    }
    log.push_str(&tr!("{name}: hash corrispondente al valore fissato ({got})\n", "{name}: hash matches the pinned value ({got})\n"));

    // Switching translator: the previous one's files go first, and its binfmt_misc handlers too,
    // or the new ones could not register under the same names.
    let overlay = fs.path(OVERLAY_SYSTEM);
    remove_listed_files(fs, &overlay);
    unregister_binfmt(fs, &mut log)?;

    let file = std::fs::File::open(&zip_path)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| fail(e.to_string()))?;
    let mut installed: Vec<PathBuf> = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| fail(e.to_string()))?;
        let Some(target) = overlay_target(entry.name(), &overlay) else { continue };
        if entry.is_dir() {
            std::fs::create_dir_all(&target)?;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))?;
            continue;
        }
        let mut data = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut data)?;
        place_file(fs, &overlay, &mut installed, &target, &data)?;
    }
    if installed.is_empty() {
        return Err(fail(tr!("nessun file prebuilts/ nell'archivio", "no prebuilts/ files in the archive")));
    }
    for (rel, content) in t.overrides() {
        place_file(fs, &overlay, &mut installed, &overlay.join(rel), content.as_bytes())?;
    }
    let listed: Vec<String> = installed.iter().map(|p| p.strip_prefix(fs.path("/")).map(|p| format!("/{}", p.display())).unwrap_or_default()).collect();
    std::fs::write(fs.path(TRANSLATION_MANIFEST), listed.join("\n") + "\n")?;
    let n = listed.len();
    log.push_str(&tr!("{n} file copiati in {OVERLAY_SYSTEM}\n", "{n} files copied into {OVERLAY_SYSTEM}\n"));

    let cfg_path = fs.path(WAYDROID_CFG);
    let mut ini = Ini::parse(&std::fs::read_to_string(&cfg_path)?);
    for (k, _) in LIBNDK_PROPS.iter().chain(LIBHOUDINI_PROPS) {
        ini.remove("properties", k);
    }
    for (k, v) in t.props() {
        ini.set("properties", k, v);
    }
    std::fs::write(&cfg_path, ini.render())?;
    regenerate_props(fs, cmd, &mut log)?;
    Ok(CmdOutput::ok(log))
}

fn place_file(fs: &HostFs, overlay: &Path, installed: &mut Vec<PathBuf>, target: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(target, data)?;
    // Android expects root:shell 0755 for executables under bin/, root:root 0644 otherwise.
    let in_bin = target.strip_prefix(overlay).map(|p| p.starts_with("bin")).unwrap_or(false);
    std::fs::set_permissions(target, std::fs::Permissions::from_mode(if in_bin { 0o755 } else { 0o644 }))?;
    if fs.is_real() {
        std::os::unix::fs::chown(target, Some(0), Some(if in_bin { 2000 } else { 0 }))?;
    }
    if !installed.iter().any(|p| p == target) {
        installed.push(target.to_path_buf());
    }
    Ok(())
}

/// Deletes what the manifest lists (only inside the overlay) and the directories left empty.
fn remove_listed_files(fs: &HostFs, overlay: &Path) {
    let Some(list) = fs.read(TRANSLATION_MANIFEST) else { return };
    let mut dirs = std::collections::BTreeSet::new();
    for f in list.lines().filter(|l| !l.is_empty()) {
        let p = fs.path(f);
        if p.starts_with(overlay) {
            let _ = std::fs::remove_file(&p);
            dirs.extend(p.ancestors().skip(1).take_while(|d| *d != overlay).map(Path::to_path_buf));
        }
    }
    // Deepest first; remove_dir leaves directories that still hold other overlay files.
    for d in dirs.iter().rev() {
        let _ = std::fs::remove_dir(d);
    }
    let _ = std::fs::remove_file(fs.path(TRANSLATION_MANIFEST));
}

/// Both translators' init scripts register these in the host kernel at every Android boot and
/// nothing removes them; foreign handlers (qemu-user) are left alone.
fn unregister_binfmt(fs: &HostFs, log: &mut String) -> Result<(), PrivError> {
    for name in TRANSLATION_BINFMT {
        let entry = format!("{BINFMT_MISC}/{name}");
        if fs.read(&entry).is_some_and(|s| s.contains("ndk_translation_program_runner") || s.contains("/system/bin/houdini")) {
            std::fs::write(fs.path(&entry), "-1")?;
            log.push_str(&tr!("binfmt_misc: rimosso il gestore {name}\n", "binfmt_misc: removed the {name} handler\n"));
        }
    }
    Ok(())
}

pub fn remove_translation(fs: &HostFs, cmd: &dyn CommandRunner) -> Result<CmdOutput, PrivError> {
    let mut log = String::new();
    remove_listed_files(fs, &fs.path(OVERLAY_SYSTEM));
    let cfg_path = fs.path(WAYDROID_CFG);
    let mut ini = Ini::parse(&std::fs::read_to_string(&cfg_path)?);
    for (k, _) in LIBNDK_PROPS.iter().chain(LIBHOUDINI_PROPS) {
        ini.remove("properties", k);
    }
    std::fs::write(&cfg_path, ini.render())?;
    // Before the restart in regenerate_props; with the files gone nothing registers them again.
    unregister_binfmt(fs, &mut log)?;
    regenerate_props(fs, cmd, &mut log)?;
    Ok(CmdOutput::ok(log))
}

/// GSF Android ID (decimal, as Google's registration page expects it). Exists once Google
/// services have started at least once on a GAPPS image.
pub fn google_device_id(cmd: &dyn CommandRunner) -> Result<CmdOutput, PrivError> {
    let mut log = String::new();
    let out = step(
        cmd,
        &mut log,
        &waydroid_bin(cmd)?,
        &[
            "shell",
            "--",
            "env",
            "ANDROID_RUNTIME_ROOT=/apex/com.android.runtime",
            "ANDROID_DATA=/data",
            "ANDROID_TZDATA_ROOT=/apex/com.android.tzdata",
            "ANDROID_I18N_ROOT=/apex/com.android.i18n",
            "sqlite3",
            "/data/data/com.google.android.gsf/databases/gservices.db",
            "select value from main where name = 'android_id';",
        ],
        Duration::from_secs(60),
    )?;
    let id = out.stdout.lines().map(str::trim).find(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_digit()));
    match id {
        Some(id) => Ok(CmdOutput::ok(format!("{id}\n"))),
        None => Err(PrivError::StepFailed {
            step: tr!("lettura dell'ID dispositivo Google", "reading the Google device ID"),
            output: tr!(
                "ID non ancora disponibile: avvia Android con l'immagine GAPPS, apri il Play Store una volta e riprova",
                "ID not available yet: start Android on the GAPPS image, open the Play Store once and try again"
            ),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::FakeRunner;
    use std::io::Write;

    #[test]
    fn system_image_from_ota_url() {
        assert_eq!(system_image("[waydroid]\nsystem_ota = https://ota.waydro.id/system/lineage/waydroid_x86_64/GAPPS.json\n"), SystemImage::Gapps);
        assert_eq!(system_image("[waydroid]\nsystem_ota = https://ota.waydro.id/system/lineage/waydroid_x86_64/VANILLA.json\n"), SystemImage::Vanilla);
        assert_eq!(system_image("[waydroid]\n"), SystemImage::Unknown);
    }

    #[test]
    fn overlay_paths_reject_traversal() {
        let o = Path::new("/ov/system");
        assert_eq!(overlay_target("repo-abc/prebuilts/lib64/libndk_translation.so", o), Some(PathBuf::from("/ov/system/lib64/libndk_translation.so")));
        assert_eq!(overlay_target("repo-abc/prebuilts/../../etc/passwd", o), None);
        assert_eq!(overlay_target("repo-abc/README.md", o), None);
        assert_eq!(overlay_target("repo-abc/prebuilts/", o), None);
    }

    fn zip_with(path: &Path, entries: &[(&str, &str)]) -> String {
        let f = std::fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default();
        for (name, body) in entries {
            z.start_file(*name, opts).unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
        crate::nvidia_stack::sha256_file(path).unwrap()
    }

    fn fake_archive(path: &Path) -> String {
        zip_with(
            path,
            &[
                ("repo-abc/README.md", "readme"),
                ("repo-abc/prebuilts/lib64/libndk_translation.so", "\x7fELF lib"),
                ("repo-abc/prebuilts/bin/arm64/linker64", "\x7fELF linker"),
                ("repo-abc/prebuilts/etc/init/ndk_translation.rc", "on early-init"),
            ],
        )
    }

    #[test]
    fn switching_to_libhoudini_replaces_libndk() {
        let root = tempfile::tempdir().unwrap();
        let fs = HostFs::rooted(root.path());
        std::fs::create_dir_all(fs.path("/var/lib/waydroid")).unwrap();
        std::fs::create_dir_all(fs.path(TRANSLATION_CACHE)).unwrap();
        std::fs::write(fs.path(WAYDROID_CFG), "[waydroid]\narch = x86_64\n\n[properties]\npersist.waydroid.multi_windows = true\n").unwrap();
        let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
        let ndk = PinnedArchive { url: "https://example.invalid/n.zip", commit: "abc", sha256: leak(fake_archive(&fs.path(format!("{TRANSLATION_CACHE}/abc.zip")))) };
        let hou_sha = zip_with(
            &fs.path(format!("{TRANSLATION_CACHE}/def.zip")),
            &[
                ("repo-def/prebuilts/lib64/libhoudini.so", "\x7fELF houdini"),
                ("repo-def/prebuilts/bin/houdini64", "\x7fELF runner"),
                ("repo-def/prebuilts/etc/init/houdini.rc", "copy /vendor/etc/binfmt_misc/arm_exe"),
            ],
        );
        let hou = PinnedArchive { url: "https://example.invalid/h.zip", commit: "def", sha256: leak(hou_sha) };
        let runner = FakeRunner::new()
            .with_program("waydroid")
            .respond("/usr/bin/waydroid upgrade -o", CmdOutput::ok(""))
            .respond("systemctl is-active waydroid-container.service", CmdOutput::ok("inactive\n"));

        install_archive(Translator::Libndk, &ndk, &fs, &runner).unwrap();
        // What libndk's init script registered at the last Android boot.
        std::fs::create_dir_all(fs.path(BINFMT_MISC)).unwrap();
        let handler = fs.path(format!("{BINFMT_MISC}/arm64_exe"));
        std::fs::write(&handler, "enabled\ninterpreter /system/bin/ndk_translation_program_runner_binfmt_misc_arm64\n").unwrap();

        install_archive(Translator::Libhoudini, &hou, &fs, &runner).unwrap();
        let cfg = std::fs::read_to_string(fs.path(WAYDROID_CFG)).unwrap();
        assert_eq!(arm_translator(&fs, &cfg), Some(Translator::Libhoudini));
        assert!(!fs.exists(format!("{OVERLAY_SYSTEM}/lib64/libndk_translation.so")));
        assert!(!fs.exists(format!("{OVERLAY_SYSTEM}/bin/arm64")), "libndk's directories go too");
        assert_eq!(std::fs::read_to_string(fs.path(format!("{OVERLAY_SYSTEM}/etc/init/houdini.rc"))).unwrap(), HOUDINI_RC);
        assert!(cfg.contains("ro.dalvik.vm.native.bridge = libhoudini.so") && cfg.contains("ro.enable.native.bridge.exec64 = 1"));
        assert!(!cfg.contains("ndk_translation") && cfg.contains("persist.waydroid.multi_windows = true"));
        assert_eq!(std::fs::read_to_string(&handler).unwrap(), "-1", "the name must be free for houdini's handler");

        remove_translation(&fs, &runner).unwrap();
        let cfg = std::fs::read_to_string(fs.path(WAYDROID_CFG)).unwrap();
        assert_eq!(arm_translator(&fs, &cfg), None);
        assert!(!cfg.contains("native.bridge") && !fs.exists(format!("{OVERLAY_SYSTEM}/etc/init/houdini.rc")));
    }

    #[test]
    fn libndk_install_and_remove_round_trip() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let fs = HostFs::rooted(root.path());
        std::fs::create_dir_all(fs.path("/var/lib/waydroid")).unwrap();
        std::fs::create_dir_all(fs.path(TRANSLATION_CACHE)).unwrap();
        std::fs::write(fs.path(WAYDROID_CFG), "[waydroid]\narch = x86_64\n\n[properties]\npersist.waydroid.multi_windows = true\n").unwrap();
        let commit = "abc";
        let sha = fake_archive(&fs.path(format!("{TRANSLATION_CACHE}/{commit}.zip")));
        let archive = PinnedArchive { url: "https://example.invalid/x.zip", commit: "abc", sha256: Box::leak(sha.into_boxed_str()) };
        let runner = FakeRunner::new()
            .with_program("waydroid")
            .respond("/usr/bin/waydroid upgrade -o", CmdOutput::ok(""))
            .respond("systemctl is-active waydroid-container.service", CmdOutput::ok("inactive\n"));

        install_archive(Translator::Libndk, &archive, &fs, &runner).unwrap();
        let cfg = std::fs::read_to_string(fs.path(WAYDROID_CFG)).unwrap();
        assert!(arm_translation_installed(&fs, &cfg));
        assert!(cfg.contains("persist.waydroid.multi_windows = true"));
        let bin = fs.path(format!("{OVERLAY_SYSTEM}/bin/arm64/linker64"));
        assert_eq!(std::fs::metadata(&bin).unwrap().permissions().mode() & 0o777, 0o755);
        assert!(!fs.exists(format!("{OVERLAY_SYSTEM}/README.md")));
        assert!(runner.calls().iter().all(|c| !c.starts_with("curl")), "cached archive must not be re-downloaded");

        // Something else living in the overlay, and the handlers Android registered at boot.
        std::fs::write(fs.path(format!("{OVERLAY_SYSTEM}/bin/other")), "keep").unwrap();
        std::fs::create_dir_all(fs.path(BINFMT_MISC)).unwrap();
        let arm64 = fs.path(format!("{BINFMT_MISC}/arm64_exe"));
        std::fs::write(&arm64, "enabled\ninterpreter /system/bin/ndk_translation_program_runner_binfmt_misc_arm64\n").unwrap();
        let qemu = fs.path(format!("{BINFMT_MISC}/arm_exe"));
        std::fs::write(&qemu, "enabled\ninterpreter /usr/libexec/qemu-binfmt/arm-binfmt-P\n").unwrap();

        let out = remove_translation(&fs, &runner).unwrap();
        let cfg = std::fs::read_to_string(fs.path(WAYDROID_CFG)).unwrap();
        assert!(!arm_translation_installed(&fs, &cfg));
        assert!(!cfg.contains("native.bridge"));
        assert!(!bin.exists());
        assert!(!fs.exists(format!("{OVERLAY_SYSTEM}/bin/arm64")), "empty directories go too");
        assert!(fs.exists(format!("{OVERLAY_SYSTEM}/bin/other")));
        assert_eq!(std::fs::read_to_string(&arm64).unwrap(), "-1");
        assert!(std::fs::read_to_string(&qemu).unwrap().contains("qemu"), "foreign handlers are left alone");
        assert!(out.stdout.contains("arm64_exe"));
    }

    #[test]
    fn tampered_libndk_archive_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let fs = HostFs::rooted(root.path());
        std::fs::create_dir_all(fs.path(TRANSLATION_CACHE)).unwrap();
        let zip_path = fs.path(format!("{TRANSLATION_CACHE}/abc.zip"));
        fake_archive(&zip_path);
        let archive = PinnedArchive { url: "https://example.invalid/x.zip", commit: "abc", sha256: "0000000000000000000000000000000000000000000000000000000000000000" };
        let runner = FakeRunner::new().respond(
            &format!("curl --fail --location --proto =https --proto-redir =https --retry 2 -sS -o {} https://example.invalid/x.zip", zip_path.display()),
            CmdOutput::ok(""),
        );
        let err = install_archive(Translator::Libndk, &archive, &fs, &runner).unwrap_err();
        assert!(err.to_string().contains("hash"));
        assert!(!zip_path.exists());
    }

    #[test]
    fn gapps_switch_reprovisions_waydroid_nvidia() {
        let root = tempfile::tempdir().unwrap();
        let fs = HostFs::rooted(root.path());
        std::fs::create_dir_all(fs.path("/var/lib/waydroid")).unwrap();
        std::fs::create_dir_all(fs.path("/usr/bin")).unwrap();
        std::fs::write(fs.path(SETUP_BIN), "#!/bin/sh").unwrap();
        std::fs::write(fs.path(WAYDROID_CFG), "[waydroid]\nnvidia_driver_mode = nvidia-prolapse\n").unwrap();
        let runner = FakeRunner::new()
            .with_program("waydroid")
            .respond("/usr/bin/waydroid init -f -s GAPPS", CmdOutput::ok("Downloading GAPPS"))
            .respond("/usr/bin/waydroid-nvidia-setup --mode nvidia-prolapse", CmdOutput::ok("Done"))
            // Running when we start, restarted by `waydroid init`, stopped after `waydroid container stop`.
            .respond("lxc-info -P /var/lib/waydroid/lxc -n waydroid -sH", CmdOutput::ok("RUNNING\n"))
            .respond("lxc-info -P /var/lib/waydroid/lxc -n waydroid -sH", CmdOutput::ok("RUNNING\n"))
            .respond("lxc-info -P /var/lib/waydroid/lxc -n waydroid -sH", CmdOutput::ok("STOPPED\n"))
            .respond("lxc-info -P /var/lib/waydroid/lxc -n waydroid -sH", CmdOutput::ok("RUNNING\n"))
            .respond("lxc-info -P /var/lib/waydroid/lxc -n waydroid -sH", CmdOutput::ok("RUNNING\n"))
            .respond("lxc-info -P /var/lib/waydroid/lxc -n waydroid -sH", CmdOutput::ok("STOPPED\n"))
            .respond("systemctl is-active waydroid-container.service", CmdOutput::ok("inactive\n"))
            .respond("systemctl stop waydroid-container.service", CmdOutput::ok(""))
            .respond("/usr/bin/waydroid container stop", CmdOutput::ok(""));
        switch_system_image_with(SystemType::Gapps, &fs, &runner, Duration::from_millis(1)).unwrap();
        let calls: Vec<String> = runner.calls().into_iter().filter(|c| !c.starts_with("lxc-info") && !c.contains("is-active")).collect();
        assert_eq!(
            calls,
            vec![
                "systemctl stop waydroid-container.service",
                "/usr/bin/waydroid container stop",
                "/usr/bin/waydroid init -f -s GAPPS",
                "systemctl stop waydroid-container.service",
                "/usr/bin/waydroid container stop",
                "/usr/bin/waydroid-nvidia-setup --mode nvidia-prolapse"
            ]
        );
    }

    #[test]
    fn gsf_id_parsing() {
        let runner = FakeRunner::new().with_program("waydroid").respond(
            "/usr/bin/waydroid shell -- env ANDROID_RUNTIME_ROOT=/apex/com.android.runtime ANDROID_DATA=/data ANDROID_TZDATA_ROOT=/apex/com.android.tzdata ANDROID_I18N_ROOT=/apex/com.android.i18n sqlite3 /data/data/com.google.android.gsf/databases/gservices.db select value from main where name = 'android_id';",
            CmdOutput::ok("3912345678901234567\n"),
        );
        assert_eq!(google_device_id(&runner).unwrap().stdout.trim(), "3912345678901234567");
    }
}
