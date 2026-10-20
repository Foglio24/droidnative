//! waydroid-nvidia (Shiro836/waydroid-nvidia, `public-test` releases): pinned releases, asset
//! verification and the root-side installation flow.
//!
//! Trust model: DroidNative pins, per release, the SHA-256 of the release's `SHA256SUMS` and the git
//! commit of its source archive. Every downloaded asset must match its `SHA256SUMS` line, and the
//! source archive must carry the pinned commit, before the upstream installer runs in `--from` mode
//! on the already-verified files (it re-checks hashes and the source/binary commit pairing itself).
//! The pins were taken after checking GitHub's build attestation of every asset
//! (`.github/workflows/build.yml` at the release tag).

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::exec::{CmdOutput, CommandRunner};
use crate::fs::HostFs;
use crate::ini::Ini;
use crate::privileged::{apply_gpu_selection, GpuSelection, PrivError};
use crate::runtime::waydroid::WAYDROID_CFG;
use crate::tr;

pub const GH_REPO: &str = "Shiro836/waydroid-nvidia";
pub const SETUP_BIN: &str = "/usr/bin/waydroid-nvidia-setup";
pub const UNINSTALLER: &str = "/usr/lib/waydroid-nvidia/install.sh";
pub const WORK_ROOT: &str = "/var/lib/droidnative/nvidia-stack";
pub const STATE_FILE: &str = "/var/lib/droidnative/nvidia-stack.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NvidiaRelease {
    pub tag: &'static str,
    pub sha256sums_sha256: &'static str,
    pub source_commit: &'static str,
}

/// Newest first. Verified 2026-10-19: SHA256SUMS hashes, source commit, and a GitHub build
/// attestation from build.yml@refs/tags/v0.2.0-rc7 for each of the six assets.
pub const PINNED_RELEASES: &[NvidiaRelease] = &[NvidiaRelease {
    tag: "v0.2.0-rc7",
    sha256sums_sha256: "43765b3bb128e3314df281eb157faac270edd6791fc2e834983f008b47c94d06",
    source_commit: "e9b98260d027858ce5e4e4ebd49ce0de6e2697ee",
}];

pub fn latest_release() -> &'static NvidiaRelease {
    &PINNED_RELEASES[0]
}

pub fn find_release(tag: &str) -> Option<&'static NvidiaRelease> {
    PINNED_RELEASES.iter().find(|r| r.tag == tag)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NvidiaMode {
    /// Venus proxy to the host driver (DroidNative's "NVIDIA Accelerated").
    Venus,
    /// NVK + NVIDIA backend inside Android (DroidNative's "NVIDIA Experimental").
    Prolapse,
}

impl NvidiaMode {
    pub fn as_arg(self) -> &'static str {
        match self {
            Self::Venus => "venus",
            Self::Prolapse => "nvidia-prolapse",
        }
    }

    pub fn from_arg(s: &str) -> Option<Self> {
        match s {
            "venus" => Some(Self::Venus),
            "nvidia-prolapse" | "prolapse" | "rust-driver" => Some(Self::Prolapse),
            _ => None,
        }
    }
}

/// The installer's asset list (`packaging/install.sh`), in its order.
pub fn asset_names(tag: &str) -> Vec<String> {
    vec![
        format!("waydroid-nvidia-host-x86_64-{tag}.tar.zst"),
        format!("waydroid-nvidia-guest-android-x86_64-{tag}.tar.zst"),
        format!("waydroid-nvidia-guest-prebuilts-{tag}.tar.zst"),
        format!("waydroid-nvidia-native-x86_64-{tag}.tar.zst"),
        format!("waydroid-nvidia-vendor-x86_64-{tag}.zip"),
        format!("waydroid-nvidia-vendor-ota-{tag}.json"),
    ]
}

pub fn source_tarball_name(tag: &str) -> String {
    format!("waydroid-nvidia-{tag}.tar.gz")
}

/// `<hash>  ./name` / `<hash> *name` lines → name → hash.
pub fn parse_sha256sums(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let (hash, name) = l.split_once(char::is_whitespace)?;
            let name = name.trim_start().trim_start_matches('*').trim_start_matches("./");
            (hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit())).then(|| (name.to_string(), hash.to_ascii_lowercase()))
        })
        .collect()
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Commit id stored by `git archive` in the pax global header (what `git get-tar-commit-id` reads).
pub fn tar_commit_id(gzipped_tar: &Path) -> Option<String> {
    let f = std::fs::File::open(gzipped_tar).ok()?;
    let mut header = [0u8; 1024];
    let mut dec = flate2::read::GzDecoder::new(f);
    dec.read_exact(&mut header).ok()?;
    // ustar header: typeflag at 156 must be 'g' (pax global header); payload follows the 512-byte header.
    if header[156] != b'g' {
        return None;
    }
    let payload = String::from_utf8_lossy(&header[512..]);
    let rec = payload.lines().next()?;
    let (_, kv) = rec.split_once(' ')?;
    let commit = kv.strip_prefix("comment=")?.trim();
    (commit.len() == 40 && commit.chars().all(|c| c.is_ascii_hexdigit())).then(|| commit.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackState {
    pub tag: String,
    pub mode: NvidiaMode,
    pub installed_unix: u64,
}

pub fn read_state(fs: &HostFs) -> Option<StackState> {
    fs.read(STATE_FILE).and_then(|t| serde_json::from_str(&t).ok())
}

/// `nvidia_driver_mode` written into waydroid.cfg by waydroid-nvidia-setup.
pub fn configured_mode(cfg_text: &str) -> Option<NvidiaMode> {
    Ini::parse(cfg_text).get("waydroid", "nvidia_driver_mode").and_then(NvidiaMode::from_arg)
}

fn step(cmd: &dyn CommandRunner, log: &mut String, program: &str, args: &[&str], timeout: Duration) -> Result<CmdOutput, PrivError> {
    log.push_str(&format!("$ {program} {}\n", args.join(" ")));
    let out = cmd.run_with_timeout(program, args, timeout)?;
    let text = out.combined();
    // Keep the log readable: the tail of long outputs (apt, make) is what explains a failure.
    let lines: Vec<&str> = text.lines().collect();
    for l in &lines[lines.len().saturating_sub(40)..] {
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

fn download(cmd: &dyn CommandRunner, log: &mut String, url: &str, dest: &Path) -> Result<(), PrivError> {
    let dest_s = dest.to_string_lossy().into_owned();
    step(
        cmd,
        log,
        "curl",
        &["--fail", "--location", "--proto", "=https", "--proto-redir", "=https", "--retry", "2", "--connect-timeout", "15", "-sS", "-o", &dest_s, url],
        Duration::from_secs(1800),
    )?;
    Ok(())
}

/// Download (or reuse) and verify every file of a pinned release into `dir`.
pub fn fetch_and_verify(release: &NvidiaRelease, dir: &Path, cmd: &dyn CommandRunner, log: &mut String) -> Result<(), PrivError> {
    let tag = release.tag;
    let base = format!("https://github.com/{GH_REPO}/releases/download/{tag}");
    let fail = |what: String| PrivError::StepFailed { step: tr!("verifica del rilascio waydroid-nvidia", "waydroid-nvidia release check"), output: what };

    let sums_path = dir.join("SHA256SUMS");
    if sha256_file(&sums_path).ok().as_deref() != Some(release.sha256sums_sha256) {
        download(cmd, log, &format!("{base}/SHA256SUMS"), &sums_path)?;
    }
    let got = sha256_file(&sums_path).map_err(|e| fail(format!("SHA256SUMS: {e}")))?;
    if got != release.sha256sums_sha256 {
        let _ = std::fs::remove_file(&sums_path);
        let want = release.sha256sums_sha256;
        return Err(fail(tr!("SHA256SUMS ha hash {got}, atteso {want} (fissato in DroidNative)", "SHA256SUMS has hash {got}, expected {want} (pinned in DroidNative)")));
    }
    log.push_str(&tr!("SHA256SUMS: hash corrispondente al valore fissato\n", "SHA256SUMS: hash matches the pinned value\n"));
    let sums = parse_sha256sums(&std::fs::read_to_string(&sums_path).map_err(|e| fail(e.to_string()))?);

    for name in asset_names(tag) {
        let want = sums.get(&name).ok_or_else(|| fail(tr!("SHA256SUMS non elenca {name}", "SHA256SUMS does not list {name}")))?;
        let path = dir.join(&name);
        if sha256_file(&path).ok().as_deref() != Some(want.as_str()) {
            download(cmd, log, &format!("{base}/{name}"), &path)?;
        }
        let got = sha256_file(&path).map_err(|e| fail(format!("{name}: {e}")))?;
        if &got != want {
            let _ = std::fs::remove_file(&path);
            return Err(fail(tr!("{name}: hash {got}, atteso {want}", "{name}: hash {got}, expected {want}")));
        }
        log.push_str(&format!("{name}: OK\n"));
    }

    let src = dir.join(source_tarball_name(tag));
    if tar_commit_id(&src).as_deref() != Some(release.source_commit) {
        download(cmd, log, &format!("https://github.com/{GH_REPO}/archive/refs/tags/{tag}.tar.gz"), &src)?;
    }
    match tar_commit_id(&src) {
        Some(c) if c == release.source_commit => log.push_str(&tr!("sorgente: commit {c} (fissato)\n", "source: commit {c} (pinned)\n")),
        other => {
            let _ = std::fs::remove_file(&src);
            let want = release.source_commit;
            return Err(fail(tr!("archivio sorgente con commit {other:?}, atteso {want}", "source archive with commit {other:?}, expected {want}")));
        }
    }
    Ok(())
}

fn distro_waydroid_status(cmd: &dyn CommandRunner) -> Option<String> {
    cmd.which("dpkg-query")?;
    let out = cmd.run("dpkg-query", &["-W", "-f=${Status}", "waydroid"]).ok()?;
    let s = out.stdout.trim().to_string();
    (out.success() && !s.is_empty() && s != "unknown ok not-installed").then_some(s)
}

/// Root side of "install waydroid-nvidia". Images in /var/lib/waydroid and user data in
/// ~/.local/share/waydroid are never touched; only the distro `waydroid` package is purged
/// (its conffile /etc/xdg/menus/applications-merged/waydroid.menu would otherwise block the
/// upstream installer, which refuses to overwrite files owned by a package).
pub fn install(release: &NvidiaRelease, mode: NvidiaMode, fs: &HostFs, cmd: &dyn CommandRunner) -> Result<CmdOutput, PrivError> {
    use std::os::unix::fs::PermissionsExt;
    let mut log = String::new();
    let dir: PathBuf = fs.path(format!("{WORK_ROOT}/{}", release.tag));
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;

    fetch_and_verify(release, &dir, cmd, &mut log)?;

    // DroidNative's own software-rendering overrides would keep SwiftShader selected.
    let cfg_path = fs.path(WAYDROID_CFG);
    if let Ok(text) = std::fs::read_to_string(&cfg_path) {
        let cleaned = apply_gpu_selection(&text, &GpuSelection::Auto);
        if cleaned != text {
            std::fs::write(&cfg_path, cleaned)?;
            log.push_str(&tr!("waydroid.cfg: rimossi gli override di rendering software di DroidNative\n", "waydroid.cfg: removed DroidNative's software rendering overrides\n"));
        }
    }

    if let Some(status) = distro_waydroid_status(cmd) {
        log.push_str(&tr!(
            "pacchetto waydroid della distribuzione: {status} → purge (immagini e dati restano)\n",
            "distribution waydroid package: {status} → purge (images and data are kept)\n"
        ));
        step(cmd, &mut log, "env", &["DEBIAN_FRONTEND=noninteractive", "apt-get", "purge", "-y", "waydroid"], Duration::from_secs(900))?;
    }

    let src = dir.join("src");
    let _ = std::fs::remove_dir_all(&src);
    std::fs::create_dir_all(&src)?;
    let tarball = dir.join(source_tarball_name(release.tag));
    step(
        cmd,
        &mut log,
        "tar",
        &["-xzf", &tarball.to_string_lossy(), "-C", &src.to_string_lossy(), "--strip-components=1"],
        Duration::from_secs(120),
    )?;
    let installer = src.join("packaging/install.sh");
    step(
        cmd,
        &mut log,
        "bash",
        &[&installer.to_string_lossy(), "--tag", release.tag, "--from", &dir.to_string_lossy()],
        Duration::from_secs(3600),
    )?;
    step(cmd, &mut log, SETUP_BIN, &["--mode", mode.as_arg()], Duration::from_secs(1800))?;

    let state = StackState {
        tag: release.tag.to_string(),
        mode,
        installed_unix: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
    };
    let state_path = fs.path(STATE_FILE);
    std::fs::write(&state_path, serde_json::to_string_pretty(&state).unwrap_or_default())?;
    std::fs::set_permissions(&state_path, std::fs::Permissions::from_mode(0o644))?;
    Ok(CmdOutput::ok(log))
}

pub fn set_mode(mode: NvidiaMode, fs: &HostFs, cmd: &dyn CommandRunner) -> Result<CmdOutput, PrivError> {
    if !fs.exists(SETUP_BIN) {
        return Err(PrivError::Missing("waydroid-nvidia"));
    }
    let mut log = String::new();
    crate::extras::ensure_container_stopped(cmd, &mut log, Duration::from_secs(1))?;
    step(cmd, &mut log, SETUP_BIN, &["--mode", mode.as_arg()], Duration::from_secs(1800))?;
    if let Some(mut state) = read_state(fs) {
        state.mode = mode;
        std::fs::write(fs.path(STATE_FILE), serde_json::to_string_pretty(&state).unwrap_or_default())?;
    }
    Ok(CmdOutput::ok(log))
}

/// The NVIDIA GPU's render node, chosen by PCI vendor as waydroid-nvidia does. The renderD numbers
/// follow driver load order, which on hybrid desktops changes from one boot to the next.
pub fn nvidia_render_node(fs: &HostFs) -> Option<String> {
    let mut nodes: Vec<String> = fs.list("/sys/class/drm").into_iter().filter(|n| n.starts_with("renderD")).collect();
    nodes.sort();
    nodes
        .into_iter()
        .find(|n| fs.read_trimmed(format!("/sys/class/drm/{n}/device/vendor")).as_deref() == Some("0x10de"))
        .map(|n| format!("/dev/dri/{n}"))
}

/// A `drm_device` left on another GPU's node. In venus mode waydroid-nvidia requires the NVIDIA
/// node but writes its number only when a mode is applied, and its container check then refuses to
/// start: seen on hardware after a reboot swapped renderD128 and renderD129.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaleRenderNode {
    pub configured: String,
    pub nvidia: String,
}

pub fn stale_render_node(fs: &HostFs, cfg_text: &str) -> Option<StaleRenderNode> {
    if !fs.exists(SETUP_BIN) || configured_mode(cfg_text) != Some(NvidiaMode::Venus) {
        return None;
    }
    let nvidia = nvidia_render_node(fs)?;
    let configured = Ini::parse(cfg_text).get("waydroid", "drm_device").unwrap_or_default().to_string();
    (configured != nvidia).then_some(StaleRenderNode { configured, nvidia })
}

/// Points `drm_device` at this boot's NVIDIA node and regenerates Waydroid's LXC nodes and
/// properties (`waydroid upgrade -o` does no D-Bus or systemd calls on a stopped container).
/// droidnative-render-node.service runs it before every container start, so without
/// `start_container` it never touches the unit: a stop or start would collide with the start job
/// that is waiting for it.
pub fn repin_render_node(start_container: bool, fs: &HostFs, cmd: &dyn CommandRunner) -> Result<CmdOutput, PrivError> {
    let mut log = String::new();
    let cfg_path = fs.path(WAYDROID_CFG);
    let text = std::fs::read_to_string(&cfg_path).unwrap_or_default();
    match stale_render_node(fs, &text) {
        None if fs.exists(SETUP_BIN) && configured_mode(&text) == Some(NvidiaMode::Venus) && nvidia_render_node(fs).is_none() => {
            return Err(PrivError::Missing("NVIDIA render node (/dev/dri/renderD*)"));
        }
        None => log.push_str(&tr!("drm_device già corretto\n", "drm_device already correct\n")),
        Some(StaleRenderNode { configured, nvidia }) => {
            let mut ini = Ini::parse(&text);
            ini.set("waydroid", "drm_device", &nvidia);
            std::fs::write(&cfg_path, ini.render())?;
            log.push_str(&tr!(
                "drm_device: {configured} → {nvidia} (nodo della GPU NVIDIA in questo avvio)\n",
                "drm_device: {configured} → {nvidia} (the NVIDIA GPU's node in this boot)\n"
            ));
            let waydroid = cmd.which("waydroid").map(|p| p.to_string_lossy().into_owned()).ok_or(PrivError::Missing("Waydroid"))?;
            step(cmd, &mut log, &waydroid, &["upgrade", "-o"], Duration::from_secs(300))?;
        }
    }
    if start_container {
        let unit = crate::runtime::waydroid::CONTAINER_UNIT;
        let _ = cmd.run("systemctl", &["reset-failed", unit]);
        step(cmd, &mut log, "systemctl", &["start", unit], Duration::from_secs(180))?;
    }
    Ok(CmdOutput::ok(log))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::exec::FakeRunner;

    /// A gzip'd tar whose pax global header carries `commit`, like `git archive` output.
    pub fn git_archive_with_commit(path: &Path, commit: &str) {
        let mut hdr = [0u8; 512];
        let name = b"pax_global_header";
        hdr[..name.len()].copy_from_slice(name);
        let payload = format!("52 comment={commit}\n");
        hdr[100..107].copy_from_slice(b"0000666");
        let size = format!("{:011o}", payload.len());
        hdr[124..135].copy_from_slice(size.as_bytes());
        hdr[156] = b'g';
        hdr[257..262].copy_from_slice(b"ustar");
        let mut body = hdr.to_vec();
        let mut p = payload.into_bytes();
        p.resize(512, 0);
        body.extend(p);
        body.extend([0u8; 1024]);
        let f = std::fs::File::create(path).unwrap();
        let mut enc = flate2::write::GzEncoder::new(f, flate2::Compression::fast());
        std::io::Write::write_all(&mut enc, &body).unwrap();
        enc.finish().unwrap();
    }

    fn sha_hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
    }

    /// Writes a complete fake release into `dir` and returns a matching pin.
    fn fake_release(dir: &Path) -> NvidiaRelease {
        let tag = "v9.9.9";
        let mut sums = String::new();
        for name in asset_names(tag) {
            let content = format!("asset {name}");
            std::fs::write(dir.join(&name), &content).unwrap();
            sums.push_str(&format!("{}  ./{name}\n", sha_hex(content.as_bytes())));
        }
        std::fs::write(dir.join("SHA256SUMS"), &sums).unwrap();
        let commit = "e9b98260d027858ce5e4e4ebd49ce0de6e2697ee";
        git_archive_with_commit(&dir.join(source_tarball_name(tag)), commit);
        NvidiaRelease {
            tag: Box::leak(tag.to_string().into_boxed_str()),
            sha256sums_sha256: Box::leak(sha_hex(sums.as_bytes()).into_boxed_str()),
            source_commit: commit,
        }
    }

    #[test]
    fn sha256sums_parsing_matches_installer_format() {
        let m = parse_sha256sums("d2dd437c15d13f37b529d0e7fceb20c4a4f142d21e76a38c1f5348408aae965f  ./waydroid-nvidia-guest-android-x86_64-v0.2.0-rc7.tar.zst\nbogus line\n");
        assert_eq!(m.len(), 1);
        assert!(m.contains_key("waydroid-nvidia-guest-android-x86_64-v0.2.0-rc7.tar.zst"));
    }

    #[test]
    fn reads_git_archive_commit() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("src.tar.gz");
        git_archive_with_commit(&p, "e9b98260d027858ce5e4e4ebd49ce0de6e2697ee");
        assert_eq!(tar_commit_id(&p).as_deref(), Some("e9b98260d027858ce5e4e4ebd49ce0de6e2697ee"));
    }

    #[test]
    fn verified_files_are_reused_without_downloading() {
        let d = tempfile::tempdir().unwrap();
        let rel = fake_release(d.path());
        let runner = FakeRunner::new();
        let mut log = String::new();
        fetch_and_verify(&rel, d.path(), &runner, &mut log).unwrap();
        assert!(runner.calls().is_empty(), "{:?}", runner.calls());
        assert!(log.contains("fissato"));
    }

    #[test]
    fn tampered_asset_is_rejected_and_deleted() {
        let d = tempfile::tempdir().unwrap();
        let rel = fake_release(d.path());
        let victim = d.path().join(&asset_names(rel.tag)[3]);
        std::fs::write(&victim, b"tampered").unwrap();
        // The re-download (simulated as a no-op) still yields the tampered file.
        let runner = FakeRunner::new().respond(
            &format!(
                "curl --fail --location --proto =https --proto-redir =https --retry 2 --connect-timeout 15 -sS -o {} https://github.com/{GH_REPO}/releases/download/{}/{}",
                victim.display(),
                rel.tag,
                asset_names(rel.tag)[3]
            ),
            CmdOutput::ok(""),
        );
        let err = fetch_and_verify(&rel, d.path(), &runner, &mut String::new()).unwrap_err();
        assert!(err.to_string().contains("hash"));
        assert!(!victim.exists());
    }

    #[test]
    fn wrong_pinned_sums_or_commit_fail() {
        let d = tempfile::tempdir().unwrap();
        let mut rel = fake_release(d.path());
        rel.source_commit = "0000000000000000000000000000000000000000";
        let src = d.path().join(source_tarball_name(rel.tag));
        let runner = FakeRunner::new().respond(
            &format!(
                "curl --fail --location --proto =https --proto-redir =https --retry 2 --connect-timeout 15 -sS -o {} https://github.com/{GH_REPO}/archive/refs/tags/{}.tar.gz",
                src.display(),
                rel.tag
            ),
            CmdOutput::ok(""),
        );
        assert!(fetch_and_verify(&rel, d.path(), &runner, &mut String::new()).unwrap_err().to_string().contains("commit"));
    }

    #[test]
    fn full_install_flow_runs_steps_in_order() {
        let root = tempfile::tempdir().unwrap();
        let fs = HostFs::rooted(root.path());
        let work = fs.path(format!("{WORK_ROOT}/v9.9.9"));
        std::fs::create_dir_all(&work).unwrap();
        let rel = fake_release(&work);
        std::fs::create_dir_all(fs.path("/var/lib/waydroid")).unwrap();
        std::fs::write(fs.path(WAYDROID_CFG), "[waydroid]\narch = x86_64\n\n[properties]\nro.hardware.gralloc = default\nro.hardware.egl = swiftshader\n").unwrap();
        let src = work.join("src");
        let runner = FakeRunner::new()
            .with_program("dpkg-query")
            .respond("dpkg-query -W -f=${Status} waydroid", CmdOutput::ok("install ok installed"))
            .respond("env DEBIAN_FRONTEND=noninteractive apt-get purge -y waydroid", CmdOutput::ok("Purging configuration files for waydroid\n"))
            .respond(
                &format!("tar -xzf {} -C {} --strip-components=1", work.join(source_tarball_name(rel.tag)).display(), src.display()),
                CmdOutput::ok(""),
            )
            .respond(&format!("bash {} --tag v9.9.9 --from {}", src.join("packaging/install.sh").display(), work.display()), CmdOutput::ok("==> installed 412 files\n"))
            .respond("/usr/bin/waydroid-nvidia-setup --mode venus", CmdOutput::ok("Done (venus mode).\n"));
        let out = install(&rel, NvidiaMode::Venus, &fs, &runner).unwrap();
        let calls = runner.calls();
        let pos = |needle: &str| calls.iter().position(|c| c.contains(needle)).unwrap_or_else(|| panic!("{needle} not in {calls:?}"));
        assert!(pos("apt-get purge") < pos("install.sh") && pos("install.sh") < pos("waydroid-nvidia-setup"));
        let cfg = std::fs::read_to_string(fs.path(WAYDROID_CFG)).unwrap();
        assert!(!cfg.contains("swiftshader") && !cfg.contains("gralloc"));
        let state = read_state(&fs).unwrap();
        assert_eq!(state.mode, NvidiaMode::Venus);
        assert!(out.stdout.contains("Done (venus mode)"));
    }

    /// This PC after a reboot: the NVIDIA GPU became renderD128, the config still says renderD129.
    fn swapped_nodes(mode: &str) -> (tempfile::TempDir, HostFs) {
        let root = tempfile::tempdir().unwrap();
        let fs = HostFs::rooted(root.path());
        for (node, vendor) in [("renderD128", "0x10de"), ("renderD129", "0x1002")] {
            std::fs::create_dir_all(fs.path(format!("/sys/class/drm/{node}/device"))).unwrap();
            std::fs::write(fs.path(format!("/sys/class/drm/{node}/device/vendor")), format!("{vendor}\n")).unwrap();
        }
        std::fs::create_dir_all(fs.path("/usr/bin")).unwrap();
        std::fs::write(fs.path(SETUP_BIN), "#!/bin/sh").unwrap();
        std::fs::create_dir_all(fs.path("/var/lib/waydroid")).unwrap();
        let cfg = format!("[waydroid]\narch = x86_64\ndrm_device = /dev/dri/renderD129\nnvidia_driver_mode = {mode}\n\n[properties]\nx = 1\n");
        std::fs::write(fs.path(WAYDROID_CFG), cfg).unwrap();
        (root, fs)
    }

    #[test]
    fn swapped_render_node_is_repinned_without_touching_systemd() {
        let (_root, fs) = swapped_nodes("venus");
        assert_eq!(nvidia_render_node(&fs).as_deref(), Some("/dev/dri/renderD128"));
        let cfg = std::fs::read_to_string(fs.path(WAYDROID_CFG)).unwrap();
        let stale = stale_render_node(&fs, &cfg).unwrap();
        assert_eq!((stale.configured.as_str(), stale.nvidia.as_str()), ("/dev/dri/renderD129", "/dev/dri/renderD128"));

        let runner = FakeRunner::new().with_program("waydroid").respond("/usr/bin/waydroid upgrade -o", CmdOutput::ok(""));
        let out = repin_render_node(false, &fs, &runner).unwrap();
        let cfg = std::fs::read_to_string(fs.path(WAYDROID_CFG)).unwrap();
        assert!(cfg.contains("drm_device = /dev/dri/renderD128") && cfg.contains("x = 1"));
        assert!(stale_render_node(&fs, &cfg).is_none());
        assert!(out.stdout.contains("renderD129 → /dev/dri/renderD128"));
        assert_eq!(runner.calls(), vec!["/usr/bin/waydroid upgrade -o"], "boot path must not call systemctl");

        // Already right: nothing to regenerate; from the UI the container is started afterwards.
        let runner = FakeRunner::new()
            .respond("systemctl reset-failed waydroid-container.service", CmdOutput::ok(""))
            .respond("systemctl start waydroid-container.service", CmdOutput::ok(""));
        repin_render_node(true, &fs, &runner).unwrap();
        assert_eq!(runner.calls(), vec!["systemctl reset-failed waydroid-container.service", "systemctl start waydroid-container.service"]);
    }

    #[test]
    fn other_modes_keep_their_render_node() {
        let (_root, fs) = swapped_nodes("nvidia-prolapse");
        let cfg = std::fs::read_to_string(fs.path(WAYDROID_CFG)).unwrap();
        assert!(stale_render_node(&fs, &cfg).is_none());
        let runner = FakeRunner::new();
        repin_render_node(false, &fs, &runner).unwrap();
        assert!(runner.calls().is_empty());
        assert!(std::fs::read_to_string(fs.path(WAYDROID_CFG)).unwrap().contains("renderD129"));
    }

    #[test]
    fn pinned_release_is_well_formed() {
        for r in PINNED_RELEASES {
            assert!(r.tag.starts_with('v'));
            assert_eq!(r.sha256sums_sha256.len(), 64);
            assert_eq!(r.source_commit.len(), 40);
        }
        assert_eq!(configured_mode("[waydroid]\nnvidia_driver_mode = venus\n"), Some(NvidiaMode::Venus));
        assert_eq!(configured_mode("[waydroid]\n"), None);
    }
}
