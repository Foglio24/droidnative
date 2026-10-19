//! Distribution, CPU, memory, storage and desktop session.

use serde::{Deserialize, Serialize};

use crate::fs::{lookup, HostFs};
use crate::host::Env;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OsFamily {
    UbuntuBased,
    DebianBased,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OsInfo {
    pub id: String,
    pub id_like: Vec<String>,
    pub pretty_name: String,
    pub version_id: Option<String>,
    pub codename: Option<String>,
    /// Codename of the Ubuntu/Debian release this distribution is built on (what third-party
    /// repositories such as repo.waydro.id are keyed by): UBUNTU_CODENAME, DEBIAN_CODENAME,
    /// VERSION_CODENAME, then /etc/debian_version — the same order Waydroid's repo script uses.
    pub upstream_codename: Option<String>,
    pub family: OsFamily,
    /// Human readable base release, e.g. "Ubuntu 26.04 (resolute)".
    pub base_release: Option<String>,
}

const UBUNTU_RELEASES: &[(&str, &str)] = &[
    ("focal", "20.04"),
    ("jammy", "22.04"),
    ("lunar", "23.04"),
    ("mantic", "23.10"),
    ("noble", "24.04"),
    ("oracular", "24.10"),
    ("plucky", "25.04"),
    ("questing", "25.10"),
    ("resolute", "26.04"),
];

const DEBIAN_RELEASES: &[(&str, &str)] =
    &[("buster", "10"), ("bullseye", "11"), ("bookworm", "12"), ("trixie", "13"), ("forky", "14")];

pub fn probe_os(fs: &HostFs) -> OsInfo {
    let pairs = {
        let p = fs.key_values("/etc/os-release");
        if p.is_empty() {
            fs.key_values("/usr/lib/os-release")
        } else {
            p
        }
    };
    let get = |k: &str| lookup(&pairs, k).map(str::to_string).filter(|v| !v.is_empty());
    let id = get("ID").unwrap_or_else(|| "linux".into());
    let id_like: Vec<String> = get("ID_LIKE").map(|v| v.split_whitespace().map(str::to_string).collect()).unwrap_or_default();
    let ubuntu_codename = get("UBUNTU_CODENAME");
    let codename = get("VERSION_CODENAME").or_else(|| ubuntu_codename.clone());

    let family = if id == "ubuntu" || ubuntu_codename.is_some() || id_like.iter().any(|l| l == "ubuntu") {
        OsFamily::UbuntuBased
    } else if id == "debian" || id_like.iter().any(|l| l == "debian") {
        OsFamily::DebianBased
    } else {
        OsFamily::Other
    };

    let base_release = match family {
        OsFamily::UbuntuBased => {
            let cn = ubuntu_codename.clone().or_else(|| codename.clone());
            cn.map(|cn| match UBUNTU_RELEASES.iter().find(|(c, _)| *c == cn) {
                Some((_, v)) => format!("Ubuntu {v} ({cn})"),
                None => format!("Ubuntu ({cn})"),
            })
        }
        OsFamily::DebianBased => {
            let dv = fs.read_trimmed("/etc/debian_version");
            let by_codename = codename
                .as_deref()
                .and_then(|cn| DEBIAN_RELEASES.iter().find(|(c, _)| *c == cn))
                .map(|(c, v)| format!("Debian {v} ({c})"));
            by_codename.or(dv.map(|v| format!("Debian {v}")))
        }
        OsFamily::Other => None,
    };

    let upstream_codename = ubuntu_codename
        .clone()
        .or_else(|| get("DEBIAN_CODENAME"))
        .or_else(|| get("VERSION_CODENAME"))
        .or_else(|| fs.read_trimmed("/etc/debian_version").map(|v| v.split('/').next().unwrap_or_default().to_string()));

    OsInfo {
        upstream_codename,
        pretty_name: get("PRETTY_NAME").unwrap_or_else(|| id.clone()),
        version_id: get("VERSION_ID"),
        id,
        id_like,
        codename,
        family,
        base_release,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CpuInfo {
    pub arch: String,
    pub vendor: Option<String>,
    pub model: Option<String>,
    pub logical_cpus: usize,
    /// Required by Waydroid's x86 images (`tools/helpers/arch.py`).
    pub has_ssse3: bool,
    pub has_avx2: bool,
    pub hw_virtualization: bool,
    /// Android ABI that matches this CPU (`x86_64`, `arm64-v8a`, ...).
    pub android_abi: Option<String>,
}

pub fn probe_cpu(fs: &HostFs, arch: &str) -> CpuInfo {
    let text = fs.read("/proc/cpuinfo").unwrap_or_default();
    let field = |name: &str| {
        text.lines()
            .find(|l| l.split(':').next().map(str::trim) == Some(name))
            .and_then(|l| l.split_once(':'))
            .map(|(_, v)| v.trim().to_string())
    };
    let flags: Vec<&str> = text
        .lines()
        .find(|l| l.starts_with("flags") || l.starts_with("Features"))
        .and_then(|l| l.split_once(':'))
        .map(|(_, v)| v.split_whitespace().collect())
        .unwrap_or_default();
    let logical_cpus = text.lines().filter(|l| l.split(':').next().map(str::trim) == Some("processor")).count();
    let android_abi = match arch {
        "x86_64" => Some("x86_64"),
        "aarch64" => Some("arm64-v8a"),
        "x86" | "i686" => Some("x86"),
        "arm" | "armv7l" => Some("armeabi-v7a"),
        _ => None,
    };
    CpuInfo {
        arch: arch.to_string(),
        vendor: field("vendor_id").or_else(|| field("CPU implementer")),
        model: field("model name").or_else(|| field("Model")),
        logical_cpus,
        has_ssse3: flags.contains(&"ssse3"),
        has_avx2: flags.contains(&"avx2"),
        hw_virtualization: flags.contains(&"svm") || flags.contains(&"vmx"),
        android_abi: android_abi.map(str::to_string),
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemoryInfo {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub swap_total_bytes: u64,
}

pub fn probe_memory(fs: &HostFs) -> MemoryInfo {
    let text = fs.read("/proc/meminfo").unwrap_or_default();
    let kb = |name: &str| -> u64 {
        text.lines()
            .find(|l| l.starts_with(&format!("{name}:")))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
            * 1024
    };
    MemoryInfo { total_bytes: kb("MemTotal"), available_bytes: kb("MemAvailable"), swap_total_bytes: kb("SwapTotal") }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageInfo {
    /// Directory checked (where Waydroid keeps images and data).
    pub path: String,
    pub total_bytes: Option<u64>,
    pub available_bytes: Option<u64>,
}

pub fn probe_storage(fs: &HostFs, host_path: &str) -> StorageInfo {
    // Walk up to the nearest existing ancestor: /var/lib/waydroid may not exist yet.
    let mut p = fs.path(host_path);
    while !p.exists() {
        if !p.pop() {
            break;
        }
    }
    let (total, avail) = statvfs(&p).map(|(t, a)| (Some(t), Some(a))).unwrap_or((None, None));
    StorageInfo { path: host_path.to_string(), total_bytes: total, available_bytes: avail }
}

fn statvfs(path: &std::path::Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: c_path is a valid NUL-terminated string and st is a properly sized out-parameter.
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), &mut st) };
    if rc != 0 {
        return None;
    }
    let frsize = st.f_frsize as u64;
    Some((st.f_blocks as u64 * frsize, st.f_bavail as u64 * frsize))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    Wayland,
    X11,
    Tty,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub kind: SessionKind,
    pub desktop: Option<String>,
    /// Compositor implied by the desktop (Mutter, KWin, ...), when known.
    pub compositor: Option<String>,
    pub wayland_display: Option<String>,
    pub wayland_socket_present: bool,
    pub x11_display: Option<String>,
}

pub fn probe_session(fs: &HostFs, env: &Env) -> SessionInfo {
    let wayland_display = env.get("WAYLAND_DISPLAY");
    let runtime_dir = env.get("XDG_RUNTIME_DIR");
    let wayland_socket_present = match (&runtime_dir, &wayland_display) {
        (_, Some(d)) if d.starts_with('/') => fs.exists(d),
        (Some(dir), Some(d)) => fs.exists(format!("{dir}/{d}")),
        _ => false,
    };
    let kind = match env.get("XDG_SESSION_TYPE").as_deref() {
        Some("wayland") => SessionKind::Wayland,
        Some("x11") => SessionKind::X11,
        Some("tty") => SessionKind::Tty,
        _ if wayland_display.is_some() => SessionKind::Wayland,
        _ if env.get("DISPLAY").is_some() => SessionKind::X11,
        _ => SessionKind::Unknown,
    };
    let desktop = env.get("XDG_CURRENT_DESKTOP");
    let compositor = desktop.as_deref().and_then(|d| {
        let d = d.to_ascii_lowercase();
        let name = if d.contains("gnome") || d.contains("unity") || d.contains("pantheon") {
            "Mutter"
        } else if d.contains("kde") {
            "KWin"
        } else if d.contains("sway") {
            "Sway"
        } else if d.contains("hyprland") {
            "Hyprland"
        } else if d.contains("cosmic") {
            "cosmic-comp"
        } else if d.contains("wayfire") {
            "Wayfire"
        } else if d.contains("xfce") {
            "xfwm4"
        } else if d.contains("cinnamon") {
            "Muffin"
        } else {
            return None;
        };
        Some(name.to_string())
    });
    SessionInfo { kind, desktop, compositor, wayland_display, wayland_socket_present, x11_display: env.get("DISPLAY") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tree(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, content) in files {
            let p = dir.path().join(path.trim_start_matches('/'));
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, content).unwrap();
        }
        dir
    }

    #[test]
    fn ubuntu_derivative_with_debian_id_like_is_ubuntu_based() {
        // AnduinOS declares ID_LIKE=debian but carries UBUNTU_CODENAME.
        let d = tree(&[(
            "/etc/os-release",
            "PRETTY_NAME=\"AnduinOS 2.0.4\"\nID=anduinos\nID_LIKE=debian\nVERSION_ID=\"2.0.4\"\nVERSION_CODENAME=resolute\nUBUNTU_CODENAME=resolute\n",
        )]);
        let os = probe_os(&HostFs::rooted(d.path()));
        assert_eq!(os.family, OsFamily::UbuntuBased);
        assert_eq!(os.base_release.as_deref(), Some("Ubuntu 26.04 (resolute)"));
        assert_eq!(os.pretty_name, "AnduinOS 2.0.4");
    }

    #[test]
    fn linux_mint_and_debian() {
        let d = tree(&[("/etc/os-release", "ID=linuxmint\nID_LIKE=\"ubuntu debian\"\nUBUNTU_CODENAME=noble\n")]);
        let os = probe_os(&HostFs::rooted(d.path()));
        assert_eq!(os.family, OsFamily::UbuntuBased);
        assert_eq!(os.base_release.as_deref(), Some("Ubuntu 24.04 (noble)"));

        let d = tree(&[("/etc/os-release", "ID=debian\nVERSION_CODENAME=bookworm\n"), ("/etc/debian_version", "12.7\n")]);
        let os = probe_os(&HostFs::rooted(d.path()));
        assert_eq!(os.family, OsFamily::DebianBased);
        assert_eq!(os.base_release.as_deref(), Some("Debian 12 (bookworm)"));

        let d = tree(&[("/etc/os-release", "ID=kali\nID_LIKE=debian\nVERSION_CODENAME=kali-rolling\n"), ("/etc/debian_version", "kali-rolling\n")]);
        let os = probe_os(&HostFs::rooted(d.path()));
        assert_eq!(os.family, OsFamily::DebianBased);
        assert_eq!(os.base_release.as_deref(), Some("Debian kali-rolling"));
    }

    #[test]
    fn cpu_and_memory() {
        let d = tree(&[
            ("/proc/cpuinfo", "processor\t: 0\nvendor_id\t: AuthenticAMD\nmodel name\t: AMD Ryzen 7 7700X 8-Core Processor\nflags\t\t: fpu ssse3 avx2 svm\n\nprocessor\t: 1\nvendor_id\t: AuthenticAMD\n"),
            ("/proc/meminfo", "MemTotal:       32000000 kB\nMemAvailable:   20000000 kB\nSwapTotal:      1000 kB\n"),
        ]);
        let fs = HostFs::rooted(d.path());
        let cpu = probe_cpu(&fs, "x86_64");
        assert_eq!(cpu.logical_cpus, 2);
        assert!(cpu.has_ssse3 && cpu.has_avx2 && cpu.hw_virtualization);
        assert_eq!(cpu.model.as_deref(), Some("AMD Ryzen 7 7700X 8-Core Processor"));
        assert_eq!(cpu.android_abi.as_deref(), Some("x86_64"));
        let mem = probe_memory(&fs);
        assert_eq!(mem.total_bytes, 32_000_000 * 1024);
        assert_eq!(mem.swap_total_bytes, 1000 * 1024);
    }

    #[test]
    fn session_detection() {
        let d = tree(&[("/run/user/1000/wayland-0", "")]);
        let env = Env::from_pairs(&[
            ("XDG_SESSION_TYPE", "wayland"),
            ("WAYLAND_DISPLAY", "wayland-0"),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
            ("XDG_CURRENT_DESKTOP", "ubuntu:GNOME"),
        ]);
        let s = probe_session(&HostFs::rooted(d.path()), &env);
        assert_eq!(s.kind, SessionKind::Wayland);
        assert!(s.wayland_socket_present);
        assert_eq!(s.compositor.as_deref(), Some("Mutter"));
    }

    #[test]
    fn storage_walks_up_to_existing_parent() {
        let d = tree(&[("/var/lib/.keep", "")]);
        let s = probe_storage(&HostFs::rooted(d.path()), "/var/lib/waydroid");
        assert!(s.available_bytes.is_some());
    }
}
