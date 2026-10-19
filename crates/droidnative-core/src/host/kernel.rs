//! Kernel features Android containers depend on: binder, namespaces, cgroups, PSI, LSMs.

use std::io::Read;

use serde::{Deserialize, Serialize};

use crate::exec::CommandRunner;
use crate::fs::HostFs;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tristate {
    Builtin,
    Module,
    Disabled,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinderSupport {
    pub config_binder: Tristate,
    pub config_binderfs: Tristate,
    /// `binder_linux` (Debian/Ubuntu name) or `binder` listed in modules.dep.
    pub module_available: bool,
    pub module_loaded: bool,
    /// `binder` registered in /proc/filesystems (binderfs usable).
    pub binderfs_registered: bool,
    pub devices: Vec<String>,
}

impl BinderSupport {
    /// Binder can be made available (possibly after `modprobe binder_linux`).
    pub fn usable(&self) -> bool {
        self.binderfs_registered
            || !self.devices.is_empty()
            || self.module_available
            || matches!(self.config_binder, Tristate::Builtin)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KernelInfo {
    pub release: String,
    pub version: String,
    pub config_source: Option<String>,
    pub binder: BinderSupport,
    pub namespaces: Vec<String>,
    pub user_namespaces: bool,
    pub cgroup_v2: bool,
    pub cgroup_controllers: Vec<String>,
    /// Pressure stall information, required by Android's lmkd.
    pub psi: bool,
    pub systemd_version: Option<String>,
    pub kvm: bool,
    pub lsm: Vec<String>,
    pub apparmor_enabled: bool,
}

pub fn probe_kernel(fs: &HostFs, cmd: &dyn CommandRunner) -> KernelInfo {
    let release = fs.read_trimmed("/proc/sys/kernel/osrelease").unwrap_or_default();
    let (config, config_source) = read_kernel_config(fs, &release);
    let cfg = |key: &str| config_value(config.as_deref(), key);

    let modules_dep = fs.read(format!("/lib/modules/{release}/modules.dep")).unwrap_or_default();
    let modules_builtin = fs.read(format!("/lib/modules/{release}/modules.builtin")).unwrap_or_default();
    let has_module = |text: &str, name: &str| {
        text.lines().any(|l| {
            let path = l.split(':').next().unwrap_or("");
            let base = path.rsplit('/').next().unwrap_or("");
            base.split(".ko").next() == Some(name)
        })
    };

    let filesystems = fs.read("/proc/filesystems").unwrap_or_default();
    let mut devices: Vec<String> = fs
        .list("/dev")
        .into_iter()
        .filter(|n| matches!(n.as_str(), "binder" | "hwbinder" | "vndbinder" | "anbox-binder" | "anbox-hwbinder" | "anbox-vndbinder"))
        .map(|n| format!("/dev/{n}"))
        .collect();
    devices.extend(fs.list("/dev/binderfs").into_iter().filter(|n| n.contains("binder")).map(|n| format!("/dev/binderfs/{n}")));

    let binder = BinderSupport {
        config_binder: cfg("CONFIG_ANDROID_BINDER_IPC"),
        config_binderfs: cfg("CONFIG_ANDROID_BINDERFS"),
        module_available: has_module(&modules_dep, "binder_linux") || has_module(&modules_dep, "binder"),
        module_loaded: fs.exists("/sys/module/binder_linux") || has_module(&modules_builtin, "binder") || fs.exists("/sys/module/binder"),
        binderfs_registered: filesystems.lines().any(|l| l.split_whitespace().last() == Some("binder")),
        devices,
    };

    let max_userns = fs.read_trimmed("/proc/sys/user/max_user_namespaces").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    let unpriv_clone = fs.read_trimmed("/proc/sys/kernel/unprivileged_userns_clone").map(|v| v != "0").unwrap_or(true);

    let cgroup_v2 = fs.exists("/sys/fs/cgroup/cgroup.controllers");
    let cgroup_controllers = fs
        .read("/sys/fs/cgroup/cgroup.controllers")
        .map(|s| s.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default();

    let systemd_version = if fs.exists("/run/systemd/system") {
        cmd.run("systemctl", &["--version"])
            .ok()
            .filter(|o| o.success())
            .and_then(|o| o.stdout.lines().next().map(|l| l.trim().to_string()))
            .or_else(|| Some("systemd".into()))
    } else {
        None
    };

    let lsm: Vec<String> = fs
        .read_trimmed("/sys/kernel/security/lsm")
        .map(|s| s.split(',').map(str::to_string).filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();

    KernelInfo {
        version: fs.read_trimmed("/proc/sys/kernel/version").unwrap_or_default(),
        release,
        config_source,
        binder,
        namespaces: fs.list("/proc/self/ns"),
        user_namespaces: max_userns > 0 && unpriv_clone,
        cgroup_v2,
        cgroup_controllers,
        psi: fs.exists("/proc/pressure/memory"),
        systemd_version,
        kvm: fs.exists("/dev/kvm"),
        apparmor_enabled: fs.read_trimmed("/sys/module/apparmor/parameters/enabled").as_deref() == Some("Y"),
        lsm,
    }
}

fn read_kernel_config(fs: &HostFs, release: &str) -> (Option<String>, Option<String>) {
    let boot = format!("/boot/config-{release}");
    if let Some(text) = fs.read(&boot) {
        return (Some(text), Some(boot));
    }
    if let Some(bytes) = fs.read_bytes("/proc/config.gz") {
        let mut text = String::new();
        if flate2::read::GzDecoder::new(bytes.as_slice()).read_to_string(&mut text).is_ok() {
            return (Some(text), Some("/proc/config.gz".into()));
        }
    }
    (None, None)
}

fn config_value(config: Option<&str>, key: &str) -> Tristate {
    let Some(config) = config else { return Tristate::Unknown };
    for line in config.lines() {
        if line == format!("# {key} is not set") {
            return Tristate::Disabled;
        }
        if let Some(v) = line.strip_prefix(key).and_then(|r| r.strip_prefix('=')) {
            return match v {
                "y" => Tristate::Builtin,
                "m" => Tristate::Module,
                _ => Tristate::Disabled,
            };
        }
    }
    Tristate::Disabled
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::{CmdOutput, FakeRunner};
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
    fn binder_as_unloaded_module() {
        let d = tree(&[
            ("/proc/sys/kernel/osrelease", "7.0.0-38-generic\n"),
            ("/boot/config-7.0.0-38-generic", "CONFIG_ANDROID_BINDER_IPC=m\n# CONFIG_ANDROID_BINDER_IPC_RUST is not set\nCONFIG_ANDROID_BINDERFS=m\nCONFIG_PSI=y\n"),
            ("/lib/modules/7.0.0-38-generic/modules.dep", "kernel/drivers/android/binder_linux.ko.zst:\nkernel/fs/ext4/ext4.ko.zst: kernel/lib/crc16.ko.zst\n"),
            ("/proc/filesystems", "nodev\tsysfs\nnodev\tcgroup2\n"),
            ("/sys/fs/cgroup/cgroup.controllers", "cpuset cpu io memory pids\n"),
            ("/proc/pressure/memory", "some avg10=0.00\n"),
            ("/proc/sys/user/max_user_namespaces", "114057\n"),
            ("/run/systemd/system/.keep", ""),
        ]);
        let runner = FakeRunner::new().respond("systemctl --version", CmdOutput::ok("systemd 259 (259.5-0ubuntu3.4)\n+PAM\n"));
        let k = probe_kernel(&HostFs::rooted(d.path()), &runner);
        assert_eq!(k.binder.config_binder, Tristate::Module);
        assert_eq!(k.binder.config_binderfs, Tristate::Module);
        assert!(k.binder.module_available);
        assert!(!k.binder.module_loaded);
        assert!(!k.binder.binderfs_registered);
        assert!(k.binder.usable());
        assert!(k.cgroup_v2 && k.psi && k.user_namespaces);
        assert_eq!(k.cgroup_controllers, vec!["cpuset", "cpu", "io", "memory", "pids"]);
        assert_eq!(k.systemd_version.as_deref(), Some("systemd 259 (259.5-0ubuntu3.4)"));
    }

    #[test]
    fn binder_missing_entirely() {
        let d = tree(&[
            ("/proc/sys/kernel/osrelease", "6.1.0-custom\n"),
            ("/boot/config-6.1.0-custom", "# CONFIG_ANDROID_BINDER_IPC is not set\n"),
            ("/lib/modules/6.1.0-custom/modules.dep", "kernel/fs/ext4/ext4.ko:\n"),
        ]);
        let k = probe_kernel(&HostFs::rooted(d.path()), &FakeRunner::new());
        assert_eq!(k.binder.config_binder, Tristate::Disabled);
        assert!(!k.binder.usable());
        assert!(k.systemd_version.is_none());
    }

    #[test]
    fn binderfs_mounted_and_devices_listed() {
        let d = tree(&[
            ("/proc/sys/kernel/osrelease", "6.8.0\n"),
            ("/proc/filesystems", "nodev\tbinder\n"),
            ("/dev/binderfs/anbox-binder", ""),
            ("/dev/binderfs/binder-control", ""),
        ]);
        let k = probe_kernel(&HostFs::rooted(d.path()), &FakeRunner::new());
        assert!(k.binder.binderfs_registered);
        assert_eq!(k.binder.devices, vec!["/dev/binderfs/anbox-binder", "/dev/binderfs/binder-control"]);
        assert_eq!(k.binder.config_binder, Tristate::Unknown);
    }
}
