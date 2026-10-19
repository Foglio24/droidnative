//! GPU discovery through sysfs, independent of any userspace graphics tool.

pub mod backends;
pub mod stack;
pub mod verify;

use serde::{Deserialize, Serialize};

use crate::exec::CommandRunner;
use crate::fs::{lookup, parse_key_values, HostFs};
use crate::host::Probe;

pub use backends::{assess_backends, BackendAssessment, GpuBackendKind, GpuPlan, Readiness};
pub use stack::{DeviceAccess, MesaInfo, NvidiaDriver, NvidiaModuleFlavor, OpenGlInfo, VulkanInfo};
pub use verify::{classify_display, classify_gles, classify_graphics, AccelStatus, AccelVerdict, DisplayHealth, DisplayStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuVendor {
    Nvidia,
    Amd,
    Intel,
    VirtIo,
    Other,
}

impl GpuVendor {
    pub fn from_pci_id(id: u16) -> Self {
        match id {
            0x10de => Self::Nvidia,
            0x1002 | 0x1022 => Self::Amd,
            0x8086 => Self::Intel,
            0x1af4 => Self::VirtIo,
            _ => Self::Other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Nvidia => "NVIDIA",
            Self::Amd => "AMD",
            Self::Intel => "Intel",
            Self::VirtIo => "VirtIO",
            Self::Other => "Altro",
        }
    }
}

/// Kernel drivers whose render nodes Mesa can drive (both on the host and in Waydroid's vendor image).
pub const MESA_KERNEL_DRIVERS: &[&str] =
    &["amdgpu", "radeon", "i915", "xe", "nouveau", "virtio-pci", "virtio_gpu", "panfrost", "panthor", "msm", "msm_dpu", "vc4", "v3d"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuDevice {
    pub pci_slot: Option<String>,
    pub vendor: GpuVendor,
    pub vendor_id: String,
    pub device_id: String,
    pub name: Option<String>,
    pub kernel_driver: Option<String>,
    pub render_node: Option<String>,
    pub card_node: Option<String>,
    /// The current user can open the render node (ACL or `render` group).
    pub render_node_accessible: bool,
    pub connected_outputs: Vec<String>,
    pub boot_vga: bool,
    pub vram_bytes: Option<u64>,
}

impl GpuDevice {
    pub fn drives_display(&self) -> bool {
        !self.connected_outputs.is_empty()
    }

    pub fn is_mesa_capable(&self) -> bool {
        self.kernel_driver.as_deref().map(|d| MESA_KERNEL_DRIVERS.contains(&d)).unwrap_or(false)
    }

    pub fn device_id_u16(&self) -> Option<u16> {
        u16::from_str_radix(self.device_id.trim_start_matches("0x"), 16).ok()
    }

    pub fn display_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| format!("{} {}:{}", self.vendor.label(), self.vendor_id, self.device_id))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuReport {
    pub devices: Vec<GpuDevice>,
    /// PCI slot of the GPU that has connected outputs, i.e. where the compositor scans out.
    pub display_device: Option<String>,
    pub nvidia: Option<NvidiaDriver>,
    pub mesa: Option<MesaInfo>,
    pub vulkan: VulkanInfo,
    pub opengl: Option<OpenGlInfo>,
    pub egl_vendors: Vec<String>,
    pub egl_external_platforms: Vec<String>,
    pub gbm_backends: Vec<String>,
    pub udmabuf: DeviceAccess,
}

impl GpuReport {
    pub fn display_gpu(&self) -> Option<&GpuDevice> {
        let slot = self.display_device.as_deref()?;
        self.devices.iter().find(|d| d.pci_slot.as_deref() == Some(slot))
    }

    pub fn nvidia_device(&self) -> Option<&GpuDevice> {
        self.devices.iter().find(|d| d.vendor == GpuVendor::Nvidia)
    }

    pub fn mesa_devices(&self) -> impl Iterator<Item = &GpuDevice> {
        self.devices.iter().filter(|d| d.is_mesa_capable() && d.render_node.is_some())
    }
}

pub fn probe_gpu(probe: &Probe) -> GpuReport {
    let devices = probe_devices(probe.fs, probe.cmd);
    let display_device = devices
        .iter()
        .find(|d| d.drives_display())
        .or_else(|| devices.iter().find(|d| d.boot_vga))
        .and_then(|d| d.pci_slot.clone());
    let multiarch = match std::env::consts::ARCH {
        "x86_64" => "x86_64-linux-gnu",
        "aarch64" => "aarch64-linux-gnu",
        other => other,
    };
    let has_nvidia = devices.iter().any(|d| d.vendor == GpuVendor::Nvidia);
    GpuReport {
        nvidia: has_nvidia.then(|| stack::probe_nvidia(probe.fs, multiarch)),
        mesa: stack::probe_mesa(probe.cmd),
        vulkan: stack::probe_vulkan(probe.fs, probe.cmd),
        opengl: stack::probe_opengl(probe.cmd),
        egl_vendors: [probe.fs.list("/usr/share/glvnd/egl_vendor.d"), probe.fs.list("/etc/glvnd/egl_vendor.d")].concat(),
        egl_external_platforms: probe.fs.list("/usr/share/egl/egl_external_platform.d"),
        gbm_backends: [probe.fs.list(format!("/usr/lib/{multiarch}/gbm")), probe.fs.list("/usr/lib/gbm")].concat(),
        udmabuf: DeviceAccess::check(probe.fs, "/dev/udmabuf"),
        devices,
        display_device,
    }
}

pub fn probe_devices(fs: &HostFs, cmd: &dyn CommandRunner) -> Vec<GpuDevice> {
    let entries = fs.list("/sys/class/drm");
    let mut devices: Vec<(std::path::PathBuf, GpuDevice)> = Vec::new();

    for name in entries.iter().filter(|n| (n.starts_with("card") && !n.contains('-')) || n.starts_with("renderD")) {
        let Ok(dev_dir) = std::fs::canonicalize(fs.path(format!("/sys/class/drm/{name}/device"))) else { continue };
        let read = |file: &str| std::fs::read_to_string(dev_dir.join(file)).ok().map(|s| s.trim().to_string());
        let Some(vendor_id) = read("vendor") else { continue };

        let idx = match devices.iter().position(|(p, _)| *p == dev_dir) {
            Some(i) => i,
            None => {
                let uevent = read("uevent").map(|s| parse_key_values(&s)).unwrap_or_default();
                let slot = lookup(&uevent, "PCI_SLOT_NAME")
                    .map(str::to_string)
                    .or_else(|| dev_dir.file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| n.contains(':')));
                let driver = lookup(&uevent, "DRIVER")
                    .map(str::to_string)
                    .or_else(|| std::fs::read_link(dev_dir.join("driver")).ok().and_then(|t| t.file_name().map(|n| n.to_string_lossy().into_owned())));
                let vid = u16::from_str_radix(vendor_id.trim_start_matches("0x"), 16).unwrap_or(0);
                devices.push((
                    dev_dir.clone(),
                    GpuDevice {
                        pci_slot: slot,
                        vendor: GpuVendor::from_pci_id(vid),
                        vendor_id: vendor_id.clone(),
                        device_id: read("device").unwrap_or_default(),
                        name: None,
                        kernel_driver: driver,
                        render_node: None,
                        card_node: None,
                        render_node_accessible: false,
                        connected_outputs: Vec::new(),
                        boot_vga: read("boot_vga").as_deref() == Some("1"),
                        vram_bytes: read("mem_info_vram_total").and_then(|v| v.parse().ok()),
                    },
                ));
                devices.len() - 1
            }
        };

        let dev = &mut devices[idx].1;
        let node = format!("/dev/dri/{name}");
        if name.starts_with("renderD") {
            dev.render_node_accessible = DeviceAccess::check(fs, &node).accessible;
            dev.render_node = Some(node);
        } else {
            dev.connected_outputs = entries
                .iter()
                .filter_map(|e| e.strip_prefix(&format!("{name}-")).map(|conn| (e, conn)))
                .filter(|(e, _)| fs.read_trimmed(format!("/sys/class/drm/{e}/status")).as_deref() == Some("connected"))
                .map(|(_, conn)| conn.to_string())
                .collect();
            dev.card_node = Some(node);
        }
    }

    let mut devices: Vec<GpuDevice> = devices.into_iter().map(|(_, d)| d).collect();
    let nvidia_smi = nvidia_smi_info(cmd);
    for dev in &mut devices {
        if let Some(slot) = dev.pci_slot.clone() {
            dev.name = lspci_name(cmd, &slot);
            if dev.vendor == GpuVendor::Nvidia {
                if let Some((_, name, vram)) = nvidia_smi.iter().find(|(bus, _, _)| same_pci_slot(bus, &slot)) {
                    dev.name = Some(name.clone());
                    dev.vram_bytes = dev.vram_bytes.or(*vram);
                }
            }
        }
    }
    devices.sort_by(|a, b| a.pci_slot.cmp(&b.pci_slot));
    devices
}

/// `lspci -mm -nn -s 01:00.0` → "NVIDIA GeForce RTX 4070 Ti SUPER".
fn lspci_name(cmd: &dyn CommandRunner, slot: &str) -> Option<String> {
    cmd.which("lspci")?;
    let out = cmd.run("lspci", &["-mm", "-nn", "-s", slot]).ok().filter(|o| o.success())?;
    parse_lspci_mm(out.stdout.lines().next()?)
}

pub(crate) fn parse_lspci_mm(line: &str) -> Option<String> {
    let fields: Vec<&str> = line.split('"').skip(1).step_by(2).collect();
    let strip_id = |s: &str| match s.rfind(" [") {
        Some(i) if s.ends_with(']') => s[..i].to_string(),
        _ => s.to_string(),
    };
    let vendor = strip_id(fields.get(1)?);
    let device = strip_id(fields.get(2)?);
    // "AD103 [GeForce RTX 4070 Ti SUPER]" → marketing name in brackets when present.
    let device = match (device.find('['), device.rfind(']')) {
        (Some(a), Some(b)) if b > a => device[a + 1..b].to_string(),
        _ => device,
    };
    let vendor_short = if vendor.contains("NVIDIA") {
        "NVIDIA"
    } else if vendor.contains("AMD") || vendor.contains("ATI") {
        "AMD"
    } else if vendor.contains("Intel") {
        "Intel"
    } else {
        vendor.as_str()
    };
    Some(format!("{vendor_short} {device}"))
}

fn nvidia_smi_info(cmd: &dyn CommandRunner) -> Vec<(String, String, Option<u64>)> {
    if cmd.which("nvidia-smi").is_none() {
        return Vec::new();
    }
    let Ok(out) = cmd.run("nvidia-smi", &["--query-gpu=pci.bus_id,name,memory.total", "--format=csv,noheader,nounits"]) else {
        return Vec::new();
    };
    if !out.success() {
        return Vec::new();
    }
    out.stdout
        .lines()
        .filter_map(|l| {
            let mut parts = l.split(',').map(str::trim);
            let bus = parts.next()?.to_string();
            let name = parts.next()?.to_string();
            let vram = parts.next().and_then(|m| m.parse::<u64>().ok()).map(|mib| mib * 1024 * 1024);
            Some((bus, name, vram))
        })
        .collect()
}

/// nvidia-smi reports "00000000:01:00.0", sysfs "0000:01:00.0".
fn same_pci_slot(a: &str, b: &str) -> bool {
    let tail = |s: &str| s.split_once(':').map(|(_, t)| t.to_ascii_lowercase()).unwrap_or_default();
    !a.is_empty() && tail(a) == tail(b)
}

#[cfg(test)]
pub(crate) mod fixtures {
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::path::Path;

    /// Build a fake /sys/class/drm for a hybrid RTX 4070 Ti SUPER + AMD Raphael desktop,
    /// with the monitor plugged into the NVIDIA card (the development machine's layout).
    pub fn hybrid_nvidia_display(root: &Path) {
        let w = |p: &str, c: &str| {
            let p = root.join(p.trim_start_matches('/'));
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, c).unwrap();
        };
        let nv = "/sys/devices/pci0000:00/0000:00:01.1/0000:01:00.0";
        let amd = "/sys/devices/pci0000:00/0000:00:08.1/0000:09:00.0";
        w(&format!("{nv}/vendor"), "0x10de\n");
        w(&format!("{nv}/device"), "0x2705\n");
        w(&format!("{nv}/boot_vga"), "1\n");
        w(&format!("{nv}/uevent"), "DRIVER=nvidia\nPCI_SLOT_NAME=0000:01:00.0\n");
        w(&format!("{amd}/vendor"), "0x1002\n");
        w(&format!("{amd}/device"), "0x164e\n");
        w(&format!("{amd}/boot_vga"), "0\n");
        w(&format!("{amd}/uevent"), "DRIVER=amdgpu\nPCI_SLOT_NAME=0000:09:00.0\n");
        w(&format!("{amd}/mem_info_vram_total"), "536870912\n");
        let drm = root.join("sys/class/drm");
        for (name, dev) in [("card1", amd), ("renderD128", amd), ("card2", nv), ("renderD129", nv)] {
            fs::create_dir_all(drm.join(name)).unwrap();
            symlink(root.join(dev.trim_start_matches('/')), drm.join(name).join("device")).unwrap();
        }
        for (conn, status) in [("card1-DP-1", "disconnected"), ("card2-DP-5", "connected"), ("card2-HDMI-A-1", "disconnected")] {
            w(&format!("/sys/class/drm/{conn}/status"), &format!("{status}\n"));
        }
        w("/dev/dri/renderD128", "");
        w("/dev/dri/renderD129", "");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::{CmdOutput, FakeRunner};

    #[test]
    fn enumerates_hybrid_system_and_finds_display_gpu() {
        let dir = tempfile::tempdir().unwrap();
        fixtures::hybrid_nvidia_display(dir.path());
        let runner = FakeRunner::new()
            .with_program("lspci")
            .with_program("nvidia-smi")
            .respond("lspci -mm -nn -s 0000:01:00.0", CmdOutput::ok("01:00.0 \"VGA compatible controller [0300]\" \"NVIDIA Corporation [10de]\" \"AD103 [GeForce RTX 4070 Ti SUPER] [2705]\" -ra1 \"Hewlett-Packard Company [103c]\" \"Device [8cfe]\"\n"))
            .respond("lspci -mm -nn -s 0000:09:00.0", CmdOutput::ok("09:00.0 \"VGA compatible controller [0300]\" \"Advanced Micro Devices, Inc. [AMD/ATI] [1002]\" \"Raphael [164e]\" -rc3 \"Hewlett-Packard Company [103c]\" \"Device [8a98]\"\n"))
            .respond(
                "nvidia-smi --query-gpu=pci.bus_id,name,memory.total --format=csv,noheader,nounits",
                CmdOutput::ok("00000000:01:00.0, NVIDIA GeForce RTX 4070 Ti SUPER, 16376\n"),
            );
        let devs = probe_devices(&HostFs::rooted(dir.path()), &runner);
        assert_eq!(devs.len(), 2);

        let nv = &devs[0];
        assert_eq!(nv.vendor, GpuVendor::Nvidia);
        assert_eq!(nv.render_node.as_deref(), Some("/dev/dri/renderD129"));
        assert_eq!(nv.card_node.as_deref(), Some("/dev/dri/card2"));
        assert_eq!(nv.connected_outputs, vec!["DP-5"]);
        assert_eq!(nv.name.as_deref(), Some("NVIDIA GeForce RTX 4070 Ti SUPER"));
        assert_eq!(nv.vram_bytes, Some(16376 * 1024 * 1024));
        assert!(!nv.is_mesa_capable());

        let amd = &devs[1];
        assert_eq!(amd.vendor, GpuVendor::Amd);
        assert_eq!(amd.kernel_driver.as_deref(), Some("amdgpu"));
        assert_eq!(amd.render_node.as_deref(), Some("/dev/dri/renderD128"));
        assert!(amd.connected_outputs.is_empty());
        assert_eq!(amd.name.as_deref(), Some("AMD Raphael"));
        assert!(amd.is_mesa_capable());
    }

    #[test]
    fn lspci_parsing() {
        assert_eq!(
            parse_lspci_mm("00:02.0 \"VGA compatible controller [0300]\" \"Intel Corporation [8086]\" \"Alder Lake-P GT2 [Iris Xe Graphics] [46a6]\" -r0c"),
            Some("Intel Iris Xe Graphics".into())
        );
    }

    #[test]
    fn pci_slot_normalisation() {
        assert!(same_pci_slot("00000000:01:00.0", "0000:01:00.0"));
        assert!(!same_pci_slot("00000000:02:00.0", "0000:01:00.0"));
    }
}
