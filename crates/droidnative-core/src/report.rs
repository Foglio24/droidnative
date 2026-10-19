//! Compatibility report: host facts turned into pass/warn/fail checks, plus the GPU plan.

use serde::{Deserialize, Serialize};

use crate::gpu::{assess_backends, GpuPlan, Readiness};
use crate::host::{HostReport, OsFamily, SessionKind, Tristate};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Info,
    Unknown,
    Warn,
    /// An installable component is missing (fixable by setup).
    Missing,
    /// A hardware/kernel/driver requirement is not met.
    Fail,
}

impl CheckStatus {
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Pass => "✅",
            Self::Info => "ℹ️",
            Self::Unknown => "❔",
            Self::Warn => "⚠️",
            Self::Missing => "📦",
            Self::Fail => "❌",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Check {
    pub id: String,
    pub label: String,
    pub status: CheckStatus,
    pub detail: String,
    pub remedy: Option<String>,
}

impl Check {
    fn new(status: CheckStatus, id: &str, label: &str, detail: &str) -> Self {
        Self { id: id.into(), label: label.into(), status, detail: detail.into(), remedy: None }
    }
    pub fn pass(id: &str, label: &str, detail: &str) -> Self {
        Self::new(CheckStatus::Pass, id, label, detail)
    }
    pub fn info(id: &str, label: &str, detail: &str) -> Self {
        Self::new(CheckStatus::Info, id, label, detail)
    }
    pub fn unknown(id: &str, label: &str, detail: &str) -> Self {
        Self::new(CheckStatus::Unknown, id, label, detail)
    }
    pub fn warn(id: &str, label: &str, detail: &str) -> Self {
        Self::new(CheckStatus::Warn, id, label, detail)
    }
    pub fn missing(id: &str, label: &str, detail: &str) -> Self {
        Self::new(CheckStatus::Missing, id, label, detail)
    }
    pub fn fail(id: &str, label: &str, detail: &str) -> Self {
        Self::new(CheckStatus::Fail, id, label, detail)
    }
    pub fn remedy(mut self, text: &str) -> Self {
        self.remedy = Some(text.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckGroup {
    pub title: String,
    pub checks: Vec<Check>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Compatible,
    CompatibleAfterSetup,
    Incompatible,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompatibilityReport {
    pub verdict: Verdict,
    pub groups: Vec<CheckGroup>,
    pub gpu_plan: GpuPlan,
    pub host: HostReport,
}

pub fn gib(bytes: u64) -> f64 {
    bytes as f64 / 1024f64.powi(3)
}

fn system_checks(r: &HostReport) -> Vec<Check> {
    let mut c = Vec::new();
    let os_detail = match &r.os.base_release {
        Some(base) => format!("{} — base {}", r.os.pretty_name, base),
        None => r.os.pretty_name.clone(),
    };
    c.push(match r.os.family {
        OsFamily::UbuntuBased | OsFamily::DebianBased => Check::pass("os", "Distribuzione", &os_detail),
        OsFamily::Other => Check::warn("os", "Distribuzione", &format!("{os_detail}: non derivata da Debian/Ubuntu, supporto non garantito")),
    });
    let cpu = format!("{} · {} thread · {}", r.cpu.model.as_deref().unwrap_or("CPU sconosciuta"), r.cpu.logical_cpus, r.cpu.arch);
    c.push(match r.cpu.arch.as_str() {
        "x86_64" if r.cpu.has_ssse3 => Check::pass("cpu", "CPU", &cpu),
        "x86_64" => Check::fail("cpu", "CPU", &format!("{cpu}: SSSE3 assente, richiesto dalle immagini Android x86")),
        "aarch64" => Check::warn("cpu", "CPU", &format!("{cpu}: ARM64 previsto in una fase successiva")),
        _ => Check::fail("cpu", "CPU", &format!("{cpu}: architettura non supportata")),
    });
    let mem = gib(r.memory.total_bytes);
    let mem_detail = format!("{mem:.1} GiB totali, {:.1} GiB disponibili", gib(r.memory.available_bytes));
    c.push(if mem >= 4.0 {
        Check::pass("memory", "Memoria RAM", &mem_detail)
    } else if mem >= 2.0 {
        Check::warn("memory", "Memoria RAM", &format!("{mem_detail}: sotto i 4 GiB consigliati"))
    } else {
        Check::fail("memory", "Memoria RAM", &format!("{mem_detail}: insufficiente"))
    });
    c.push(match r.storage.available_bytes {
        Some(b) if gib(b) >= 8.0 => Check::pass("storage", "Spazio disco", &format!("{:.1} GiB liberi per {}", gib(b), r.storage.path)),
        Some(b) if gib(b) >= 4.0 => Check::warn("storage", "Spazio disco", &format!("{:.1} GiB liberi: immagini (~1,5 GiB) e dati app possono esaurirli", gib(b))),
        Some(b) => Check::fail("storage", "Spazio disco", &format!("{:.1} GiB liberi: insufficienti", gib(b))),
        None => Check::unknown("storage", "Spazio disco", "non determinabile"),
    });
    c.push(match &r.kernel.systemd_version {
        Some(v) => Check::pass("systemd", "systemd", v),
        None => Check::warn("systemd", "systemd", "assente: il servizio del container va avviato manualmente"),
    });
    c
}

fn kernel_checks(r: &HostReport) -> Vec<Check> {
    let k = &r.kernel;
    let mut c = vec![Check::info("kernel", "Kernel", &k.release)];
    let b = &k.binder;
    c.push(if b.binderfs_registered || !b.devices.is_empty() {
        Check::pass("binder", "Binder / binderfs", &format!("attivo{}", if b.devices.is_empty() { String::new() } else { format!(" ({})", b.devices.join(", ")) }))
    } else if b.module_available {
        Check::pass("binder", "Binder / binderfs", "modulo binder_linux disponibile, caricato da Waydroid all'avvio del container")
    } else if b.config_binder == Tristate::Builtin {
        Check::pass("binder", "Binder / binderfs", "integrato nel kernel")
    } else {
        Check::fail("binder", "Binder / binderfs", "non disponibile in questo kernel").remedy("Usa il kernel della distribuzione (Ubuntu ≥ 22.04, Debian ≥ 12 includono binder_linux)")
    });
    let needed = ["mnt", "pid", "net", "ipc", "uts"];
    let missing: Vec<&str> = needed.iter().copied().filter(|n| !k.namespaces.iter().any(|x| x == n)).collect();
    c.push(if missing.is_empty() {
        Check::pass("namespaces", "Namespace Linux", &k.namespaces.join(", "))
    } else {
        Check::fail("namespaces", "Namespace Linux", &format!("mancano: {}", missing.join(", ")))
    });
    c.push(if k.cgroup_v2 {
        Check::pass("cgroups", "cgroup v2", &k.cgroup_controllers.join(" "))
    } else {
        Check::warn("cgroups", "cgroup v2", "gerarchia v1/ibrida")
    });
    c.push(if k.psi {
        Check::pass("psi", "Pressure Stall Information", "attivo (necessario a lmkd di Android)")
    } else {
        Check::warn("psi", "Pressure Stall Information", "assente: lmkd può fallire").remedy("Aggiungi psi=1 ai parametri del kernel")
    });
    c.push(Check::info(
        "lsm",
        "Moduli di sicurezza",
        &format!("{}{}", k.lsm.join(", "), if k.apparmor_enabled { " · AppArmor attivo (profili Waydroid applicati)" } else { "" }),
    ));
    c.push(Check::info("kvm", "KVM", if k.kvm { "disponibile (non necessario: Android gira in container)" } else { "assente (non necessario)" }));
    c
}

fn graphics_checks(r: &HostReport) -> Vec<Check> {
    let mut c = Vec::new();
    let s = &r.session;
    c.push(match s.kind {
        SessionKind::Wayland => Check::pass(
            "session",
            "Sessione grafica",
            &format!("Wayland · {} · {}", s.desktop.as_deref().unwrap_or("?"), s.compositor.as_deref().unwrap_or("compositor sconosciuto")),
        ),
        SessionKind::X11 => Check::warn("session", "Sessione grafica", "X11: Waydroid richiede Wayland, DroidNative userà un compositor annidato"),
        _ => Check::warn("session", "Sessione grafica", "nessuna sessione grafica rilevata"),
    });
    for d in &r.gpu.devices {
        let mut detail = format!(
            "{} · driver {} · {}",
            d.pci_slot.as_deref().unwrap_or("?"),
            d.kernel_driver.as_deref().unwrap_or("?"),
            d.render_node.as_deref().unwrap_or("nessun nodo render")
        );
        if let Some(v) = d.vram_bytes {
            detail.push_str(&format!(" · {:.1} GiB VRAM", gib(v)));
        }
        if d.drives_display() {
            detail.push_str(&format!(" · pilota il display ({})", d.connected_outputs.join(", ")));
        }
        c.push(Check::info(&format!("gpu_{}", d.pci_slot.as_deref().unwrap_or("x")), &d.display_name(), &detail));
    }
    if r.gpu.devices.is_empty() {
        c.push(Check::warn("gpu", "GPU", "nessun dispositivo DRM rilevato"));
    }
    if let Some(n) = &r.gpu.nvidia {
        c.push(Check::info(
            "nvidia",
            "Driver NVIDIA",
            &format!(
                "{} · modulo {} · modeset {} · backend GBM {} · egl-wayland {}",
                n.version.as_deref().unwrap_or("?"),
                match n.module_flavor {
                    crate::gpu::NvidiaModuleFlavor::Open => "open",
                    crate::gpu::NvidiaModuleFlavor::Proprietary => "chiuso",
                    crate::gpu::NvidiaModuleFlavor::Unknown => "?",
                },
                match n.modeset {
                    Some(true) => "attivo",
                    Some(false) => "disattivo",
                    None => "?",
                },
                if n.gbm_backend { "sì" } else { "no" },
                if n.egl_wayland { "sì" } else { "no" },
            ),
        ));
    }
    c.push(match &r.gpu.mesa {
        Some(m) => Check::info("mesa", "Mesa", &format!("{} ({})", m.version, m.source)),
        None => Check::info("mesa", "Mesa", "non rilevato tramite pacchetti"),
    });
    let v = &r.gpu.vulkan;
    let icds: Vec<String> = v
        .icds
        .iter()
        .map(|i| format!("{}{}", i.file.trim_end_matches(".json"), i.api_version.as_deref().map(|a| format!(" {a}")).unwrap_or_default()))
        .collect();
    c.push(Check::info(
        "vulkan",
        "Vulkan",
        &format!("loader {} · ICD: {}", v.loader_version.as_deref().unwrap_or("?"), if icds.is_empty() { "nessuno".into() } else { icds.join(", ") }),
    ));
    for dev in &v.devices {
        c.push(Check::info("vulkan_device", "Dispositivo Vulkan", &format!("{} · {} · API {}", dev.name, dev.driver.as_deref().unwrap_or("?"), dev.api_version.as_deref().unwrap_or("?"))));
    }
    if !v.vulkaninfo_available {
        c.push(Check::info("vulkaninfo", "vulkaninfo", "non installato: dispositivi Vulkan non enumerati (pacchetto vulkan-tools)"));
    }
    c.push(match &r.gpu.opengl {
        Some(gl) => Check::info("opengl", "OpenGL host", &format!("{} · {}", gl.renderer.as_deref().unwrap_or("?"), gl.version.as_deref().unwrap_or("?"))),
        None => Check::info("opengl", "OpenGL host", "glxinfo/eglinfo non installati: versione non rilevabile (pacchetto mesa-utils)"),
    });
    c.push(Check::info(
        "udmabuf",
        "/dev/udmabuf",
        match (r.gpu.udmabuf.exists, r.gpu.udmabuf.accessible) {
            (true, true) => "presente e accessibile",
            (true, false) => "presente, non accessibile all'utente",
            _ => "assente",
        },
    ));
    c
}

fn runtime_checks(r: &HostReport) -> Vec<Check> {
    let t = &r.tools;
    let w = &r.waydroid;
    let mut c = Vec::new();
    c.push(match &t.lxc.path {
        Some(_) => Check::pass("lxc", "LXC", t.lxc.version.as_deref().unwrap_or("installato")),
        None => Check::missing("lxc", "LXC", "non installato (dipendenza di Waydroid)"),
    });
    c.push(if w.installed {
        Check::pass("waydroid", "Waydroid", &format!("{} ({})", w.version.as_deref().unwrap_or("?"), w.binary.as_deref().unwrap_or("")))
    } else {
        Check::missing("waydroid", "Waydroid", "non installato").remedy("Repository ufficiale repo.waydro.id, disponibile per questa release")
    });
    c.push(if w.initialized {
        Check::pass("images", "Immagini Android", &format!("{} immagini · {}", w.images.len(), w.config.as_ref().and_then(|c| c.vendor_type.clone()).unwrap_or_default()))
    } else {
        Check::missing("images", "Immagini Android", "non inizializzate (download con verifica SHA-256 dal canale OTA ufficiale)")
    });
    if w.installed {
        c.push(if w.container_service.is_active() {
            Check::pass("container_service", "Servizio container", "waydroid-container.service attivo")
        } else {
            Check::missing("container_service", "Servizio container", &format!("waydroid-container.service: {}", w.container_service.active.as_deref().unwrap_or("non trovato")))
        });
    }
    let net_ok = t.dnsmasq.path.is_some() && (t.nft.path.is_some() || t.iptables.path.is_some());
    c.push(if net_ok {
        Check::pass("network", "Rete del container", "dnsmasq + nftables/iptables presenti")
    } else {
        Check::missing("network", "Rete del container", "servono dnsmasq e nftables o iptables")
    });
    let fw = &r.firewall;
    c.push(match (&w.dhcp_lease, fw.may_block_container()) {
        (Some(ip), _) => Check::pass("android_network", "Rete di Android", &format!("indirizzo {ip} assegnato dal DHCP del container")),
        (None, true) => Check::warn(
            "android_network",
            "Rete di Android",
            &format!(
                "firewall UFW attivo (ingresso {}, inoltro {}): blocca DHCP, DNS e inoltro su waydroid0, Android resta senza rete",
                fw.input_policy.as_deref().unwrap_or("?"),
                fw.forward_policy.as_deref().unwrap_or("?")
            ),
        )
        .remedy("Installazione guidata → «Consentire la rete del container» (regole limitate a waydroid0)"),
        (None, false) => Check::info("android_network", "Rete di Android", "nessun blocco del firewall rilevato; l'indirizzo compare al primo avvio"),
    });
    c.push(match &t.pkexec.path {
        Some(p) => Check::pass("pkexec", "Elevazione privilegi (polkit)", p),
        None => Check::missing("pkexec", "Elevazione privilegi (polkit)", "pkexec assente: le operazioni amministrative non sono eseguibili dalla GUI"),
    });
    c.push(Check::info(
        "existing",
        "Installazioni preesistenti",
        &match (w.config_present, w.user_data_present, &w.nvidia_setup) {
            (false, false, None) => "nessuna".to_string(),
            (cfg, data, nv) => format!(
                "{}{}{}",
                if cfg { "config Waydroid presente; " } else { "" },
                if data { "dati utente in ~/.local/share/waydroid; " } else { "" },
                nv.as_deref().map(|p| format!("waydroid-nvidia ({p})")).unwrap_or_default()
            ),
        },
    ));
    c
}

pub fn build_report(host: HostReport) -> CompatibilityReport {
    let groups = vec![
        CheckGroup { title: "Sistema".into(), checks: system_checks(&host) },
        CheckGroup { title: "Kernel e container".into(), checks: kernel_checks(&host) },
        CheckGroup { title: "Grafica".into(), checks: graphics_checks(&host) },
        CheckGroup { title: "Runtime Android".into(), checks: runtime_checks(&host) },
    ];
    let gpu_plan = assess_backends(&host);
    let all = || groups.iter().flat_map(|g| g.checks.iter());
    let verdict = if all().any(|c| c.status == CheckStatus::Fail) {
        Verdict::Incompatible
    } else if all().any(|c| c.status == CheckStatus::Missing) {
        Verdict::CompatibleAfterSetup
    } else {
        Verdict::Compatible
    };
    CompatibilityReport { verdict, groups, gpu_plan, host }
}

impl CompatibilityReport {
    pub fn to_markdown(&self) -> String {
        let h = &self.host;
        let mut md = String::new();
        md.push_str("# Report di compatibilità DroidNative\n\n");
        md.push_str(&format!(
            "Host: **{}** · {} · kernel {} · DroidNative {}\n\n",
            h.hostname.as_deref().unwrap_or("?"),
            h.os.pretty_name,
            h.kernel.release,
            h.droidnative_version
        ));
        md.push_str(&format!(
            "**Esito: {}**\n\n",
            match self.verdict {
                Verdict::Compatible => "compatibile",
                Verdict::CompatibleAfterSetup => "compatibile dopo l'installazione dei componenti mancanti",
                Verdict::Incompatible => "non compatibile",
            }
        ));
        for g in &self.groups {
            md.push_str(&format!("## {}\n\n| | Controllo | Dettaglio |\n|---|---|---|\n", g.title));
            for c in &g.checks {
                let mut detail = c.detail.replace('|', "\\|");
                if let Some(r) = &c.remedy {
                    detail.push_str(&format!(" — *{}*", r.replace('|', "\\|")));
                }
                md.push_str(&format!("| {} | {} | {} |\n", c.status.symbol(), c.label, detail));
            }
            md.push('\n');
        }
        md.push_str("## Backend GPU\n\n");
        let rec = crate::gpu::backends::descriptor(self.gpu_plan.recommended);
        md.push_str(&format!("**Consigliato: {}** — {}\n\n", rec.name, self.gpu_plan.reason));
        for a in &self.gpu_plan.assessments {
            md.push_str(&format!(
                "### {} — {}\n\n{}\n\n",
                a.descriptor.name,
                match a.readiness {
                    Readiness::Ready => "pronto (accelerazione da verificare all'avvio)",
                    Readiness::NeedsSetup => "compatibile, richiede installazione",
                    Readiness::Unsupported => "non supportato su questo sistema",
                },
                a.descriptor.summary
            ));
            if let Some(t) = &a.target_device {
                md.push_str(&format!("Dispositivo: {t}{}\n\n", a.target_render_node.as_deref().map(|n| format!(" ({n})")).unwrap_or_default()));
            }
            for c in &a.checks {
                md.push_str(&format!("- {} **{}**: {}", c.status.symbol(), c.label, c.detail));
                if let Some(r) = &c.remedy {
                    md.push_str(&format!(" — *{r}*"));
                }
                md.push('\n');
            }
            md.push('\n');
        }
        md.push_str("> Nessun backend è dichiarato attivo finché DroidNative non legge il renderer reale di SurfaceFlinger (`droidnative verify-gpu`).\n");
        md
    }
}
