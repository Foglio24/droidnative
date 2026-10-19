//! Host system analysis: everything DroidNative needs to know before running Android.

pub mod kernel;
pub mod os;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::exec::CommandRunner;
use crate::fs::HostFs;
use crate::gpu::{self, GpuReport};
use crate::runtime::waydroid::{self, WaydroidInstall};

pub use kernel::{BinderSupport, KernelInfo, Tristate};
pub use os::{CpuInfo, MemoryInfo, OsFamily, OsInfo, SessionInfo, SessionKind, StorageInfo};

/// Snapshot of environment variables, injectable for tests.
#[derive(Debug, Clone, Default)]
pub struct Env(HashMap<String, String>);

impl Env {
    pub fn from_process() -> Self {
        Self(std::env::vars().collect())
    }

    pub fn from_pairs(pairs: &[(&str, &str)]) -> Self {
        Self(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect())
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.0.get(key).filter(|v| !v.is_empty()).cloned()
    }
}

/// Everything a probe needs: filesystem view, process runner, environment.
pub struct Probe<'a> {
    pub fs: &'a HostFs,
    pub cmd: &'a dyn CommandRunner,
    pub env: &'a Env,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInfo {
    pub name: String,
    pub path: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostTools {
    pub lxc: ToolInfo,
    pub dnsmasq: ToolInfo,
    pub nft: ToolInfo,
    pub iptables: ToolInfo,
    pub pkexec: ToolInfo,
    pub adb: ToolInfo,
    pub docker: ToolInfo,
    pub podman: ToolInfo,
    pub weston: ToolInfo,
    pub gamescope: ToolInfo,
}

fn tool(cmd: &dyn CommandRunner, name: &str, version_args: Option<&[&str]>) -> ToolInfo {
    let path = cmd.which(name);
    let version = match (&path, version_args) {
        (Some(p), Some(args)) => cmd
            .run(&p.to_string_lossy(), args)
            .ok()
            .filter(|o| o.success())
            .and_then(|o| o.combined().lines().next().map(|l| l.trim().to_string()))
            .filter(|l| !l.is_empty()),
        _ => None,
    };
    ToolInfo { name: name.to_string(), path: path.map(|p| p.to_string_lossy().into_owned()), version }
}

pub fn probe_tools(cmd: &dyn CommandRunner) -> HostTools {
    HostTools {
        lxc: tool(cmd, "lxc-start", Some(&["--version"])),
        dnsmasq: tool(cmd, "dnsmasq", None),
        nft: tool(cmd, "nft", None),
        iptables: tool(cmd, "iptables", None),
        pkexec: tool(cmd, "pkexec", None),
        adb: tool(cmd, "adb", Some(&["version"])),
        docker: tool(cmd, "docker", None),
        podman: tool(cmd, "podman", None),
        weston: tool(cmd, "weston", None),
        gamescope: tool(cmd, "gamescope", None),
    }
}

/// Host firewall state as far as an unprivileged user can read it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FirewallInfo {
    pub ufw_enabled: bool,
    pub input_policy: Option<String>,
    pub forward_policy: Option<String>,
}

impl FirewallInfo {
    /// UFW's defaults drop the container's DHCP/DNS requests and its forwarded traffic.
    pub fn may_block_container(&self) -> bool {
        let drops = |p: &Option<String>| matches!(p.as_deref(), Some("DROP") | Some("REJECT"));
        self.ufw_enabled && (drops(&self.input_policy) || drops(&self.forward_policy))
    }
}

pub fn probe_firewall(fs: &HostFs) -> FirewallInfo {
    let conf = fs.key_values("/etc/ufw/ufw.conf");
    let defaults = fs.key_values("/etc/default/ufw");
    FirewallInfo {
        ufw_enabled: crate::fs::lookup(&conf, "ENABLED") == Some("yes"),
        input_policy: crate::fs::lookup(&defaults, "DEFAULT_INPUT_POLICY").map(str::to_string),
        forward_policy: crate::fs::lookup(&defaults, "DEFAULT_FORWARD_POLICY").map(str::to_string),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostReport {
    pub droidnative_version: String,
    pub generated_unix: u64,
    pub hostname: Option<String>,
    pub os: OsInfo,
    pub kernel: KernelInfo,
    pub cpu: CpuInfo,
    pub memory: MemoryInfo,
    pub storage: StorageInfo,
    pub session: SessionInfo,
    pub tools: HostTools,
    pub firewall: FirewallInfo,
    pub gpu: GpuReport,
    pub waydroid: WaydroidInstall,
}

pub fn probe_host(probe: &Probe) -> HostReport {
    let fs = probe.fs;
    HostReport {
        droidnative_version: crate::VERSION.to_string(),
        generated_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
        hostname: fs.read_trimmed("/proc/sys/kernel/hostname"),
        os: os::probe_os(fs),
        kernel: kernel::probe_kernel(fs, probe.cmd),
        cpu: os::probe_cpu(fs, std::env::consts::ARCH),
        memory: os::probe_memory(fs),
        storage: os::probe_storage(fs, "/var/lib/waydroid"),
        session: os::probe_session(fs, probe.env),
        tools: probe_tools(probe.cmd),
        firewall: probe_firewall(fs),
        gpu: gpu::probe_gpu(probe),
        waydroid: waydroid::detect_install(probe),
    }
}

/// Probe the machine this process runs on.
pub fn probe_local() -> HostReport {
    let fs = HostFs::real();
    let cmd = crate::exec::SystemRunner;
    let env = Env::from_process();
    probe_host(&Probe { fs: &fs, cmd: &cmd, env: &env })
}
