//! Compatibility report: host facts turned into pass/warn/fail checks, plus the GPU plan.

use serde::{Deserialize, Serialize};

use crate::tr;

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
    fn new(status: CheckStatus, id: &str, label: impl AsRef<str>, detail: impl AsRef<str>) -> Self {
        Self { id: id.into(), label: label.as_ref().into(), status, detail: detail.as_ref().into(), remedy: None }
    }
    pub fn pass(id: &str, label: impl AsRef<str>, detail: impl AsRef<str>) -> Self {
        Self::new(CheckStatus::Pass, id, label, detail)
    }
    pub fn info(id: &str, label: impl AsRef<str>, detail: impl AsRef<str>) -> Self {
        Self::new(CheckStatus::Info, id, label, detail)
    }
    pub fn unknown(id: &str, label: impl AsRef<str>, detail: impl AsRef<str>) -> Self {
        Self::new(CheckStatus::Unknown, id, label, detail)
    }
    pub fn warn(id: &str, label: impl AsRef<str>, detail: impl AsRef<str>) -> Self {
        Self::new(CheckStatus::Warn, id, label, detail)
    }
    pub fn missing(id: &str, label: impl AsRef<str>, detail: impl AsRef<str>) -> Self {
        Self::new(CheckStatus::Missing, id, label, detail)
    }
    pub fn fail(id: &str, label: impl AsRef<str>, detail: impl AsRef<str>) -> Self {
        Self::new(CheckStatus::Fail, id, label, detail)
    }
    pub fn remedy(mut self, text: impl AsRef<str>) -> Self {
        self.remedy = Some(text.as_ref().into());
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
    let distro = tr!("Distribuzione", "Distribution");
    c.push(match r.os.family {
        OsFamily::UbuntuBased | OsFamily::DebianBased => Check::pass("os", &distro, &os_detail),
        OsFamily::Other => Check::warn(
            "os",
            &distro,
            tr!("{os_detail}: non derivata da Debian/Ubuntu, supporto non garantito", "{os_detail}: not Debian/Ubuntu based, support not guaranteed"),
        ),
    });
    let unknown_cpu = tr!("CPU sconosciuta", "unknown CPU");
    let cpu = format!("{} · {} thread · {}", r.cpu.model.as_deref().unwrap_or(&unknown_cpu), r.cpu.logical_cpus, r.cpu.arch);
    c.push(match r.cpu.arch.as_str() {
        "x86_64" if r.cpu.has_ssse3 => Check::pass("cpu", "CPU", &cpu),
        "x86_64" => Check::fail("cpu", "CPU", tr!("{cpu}: SSSE3 assente, richiesto dalle immagini Android x86", "{cpu}: no SSSE3, required by the x86 Android images")),
        "aarch64" => Check::warn("cpu", "CPU", tr!("{cpu}: ARM64 previsto in una fase successiva", "{cpu}: ARM64 is planned for a later release")),
        _ => Check::fail("cpu", "CPU", tr!("{cpu}: architettura non supportata", "{cpu}: unsupported architecture")),
    });
    let mem = gib(r.memory.total_bytes);
    let avail = gib(r.memory.available_bytes);
    let mem_detail = tr!("{mem:.1} GiB totali, {avail:.1} GiB disponibili", "{mem:.1} GiB total, {avail:.1} GiB available");
    let ram = tr!("Memoria RAM", "Memory");
    c.push(if mem >= 4.0 {
        Check::pass("memory", &ram, &mem_detail)
    } else if mem >= 2.0 {
        Check::warn("memory", &ram, tr!("{mem_detail}: sotto i 4 GiB consigliati", "{mem_detail}: below the recommended 4 GiB"))
    } else {
        Check::fail("memory", &ram, tr!("{mem_detail}: insufficiente", "{mem_detail}: not enough"))
    });
    let disk = tr!("Spazio disco", "Disk space");
    let path = &r.storage.path;
    c.push(match r.storage.available_bytes.map(gib) {
        Some(g) if g >= 8.0 => Check::pass("storage", &disk, tr!("{g:.1} GiB liberi per {path}", "{g:.1} GiB free for {path}")),
        Some(g) if g >= 4.0 => Check::warn(
            "storage",
            &disk,
            tr!("{g:.1} GiB liberi: immagini (~1,5 GiB) e dati app possono esaurirli", "{g:.1} GiB free: images (~1.5 GiB) and app data may fill it"),
        ),
        Some(g) => Check::fail("storage", &disk, tr!("{g:.1} GiB liberi: insufficienti", "{g:.1} GiB free: not enough")),
        None => Check::unknown("storage", &disk, tr!("non determinabile", "cannot be determined")),
    });
    c.push(match &r.kernel.systemd_version {
        Some(v) => Check::pass("systemd", "systemd", v),
        None => Check::warn("systemd", "systemd", tr!("assente: il servizio del container va avviato manualmente", "missing: the container service has to be started by hand")),
    });
    c
}

fn kernel_checks(r: &HostReport) -> Vec<Check> {
    let k = &r.kernel;
    let mut c = vec![Check::info("kernel", "Kernel", &k.release)];
    let b = &k.binder;
    c.push(if b.binderfs_registered || !b.devices.is_empty() {
        let devices = if b.devices.is_empty() { String::new() } else { format!(" ({})", b.devices.join(", ")) };
        Check::pass("binder", "Binder / binderfs", tr!("attivo{devices}", "active{devices}"))
    } else if b.module_available {
        Check::pass(
            "binder",
            "Binder / binderfs",
            tr!("modulo binder_linux disponibile, caricato da Waydroid all'avvio del container", "binder_linux module available, loaded by Waydroid when the container starts"),
        )
    } else if b.config_binder == Tristate::Builtin {
        Check::pass("binder", "Binder / binderfs", tr!("integrato nel kernel", "built into the kernel"))
    } else {
        Check::fail("binder", "Binder / binderfs", tr!("non disponibile in questo kernel", "not available in this kernel"))
            .remedy(tr!("Usa il kernel della distribuzione (Ubuntu ≥ 22.04, Debian ≥ 12 includono binder_linux)", "Use the distribution kernel (Ubuntu ≥ 22.04 and Debian ≥ 12 ship binder_linux)"))
    });
    let needed = ["mnt", "pid", "net", "ipc", "uts"];
    let missing: Vec<&str> = needed.iter().copied().filter(|n| !k.namespaces.iter().any(|x| x == n)).collect();
    let ns = tr!("Namespace Linux", "Linux namespaces");
    c.push(if missing.is_empty() {
        Check::pass("namespaces", &ns, k.namespaces.join(", "))
    } else {
        let list = missing.join(", ");
        Check::fail("namespaces", &ns, tr!("mancano: {list}", "missing: {list}"))
    });
    c.push(if k.cgroup_v2 {
        Check::pass("cgroups", "cgroup v2", k.cgroup_controllers.join(" "))
    } else {
        Check::warn("cgroups", "cgroup v2", tr!("gerarchia v1/ibrida", "v1/hybrid hierarchy"))
    });
    c.push(if k.psi {
        Check::pass("psi", "Pressure Stall Information", tr!("attivo (necessario a lmkd di Android)", "active (needed by Android's lmkd)"))
    } else {
        Check::warn("psi", "Pressure Stall Information", tr!("assente: lmkd può fallire", "missing: lmkd may fail"))
            .remedy(tr!("Aggiungi psi=1 ai parametri del kernel", "Add psi=1 to the kernel parameters"))
    });
    let apparmor = if k.apparmor_enabled { tr!(" · AppArmor attivo (profili Waydroid applicati)", " · AppArmor active (Waydroid profiles applied)") } else { String::new() };
    c.push(Check::info("lsm", tr!("Moduli di sicurezza", "Security modules"), format!("{}{apparmor}", k.lsm.join(", "))));
    c.push(Check::info(
        "kvm",
        "KVM",
        if k.kvm {
            tr!("disponibile (non necessario: Android gira in container)", "available (not needed: Android runs in a container)")
        } else {
            tr!("assente (non necessario)", "missing (not needed)")
        },
    ));
    c
}

fn graphics_checks(r: &HostReport) -> Vec<Check> {
    let mut c = Vec::new();
    let s = &r.session;
    let session = tr!("Sessione grafica", "Graphical session");
    c.push(match s.kind {
        SessionKind::Wayland => {
            let compositor = s.compositor.clone().unwrap_or_else(|| tr!("compositor sconosciuto", "unknown compositor"));
            Check::pass("session", &session, format!("Wayland · {} · {compositor}", s.desktop.as_deref().unwrap_or("?")))
        }
        SessionKind::X11 => Check::warn(
            "session",
            &session,
            tr!("X11: Waydroid richiede Wayland, DroidNative userà un compositor annidato", "X11: Waydroid needs Wayland, DroidNative will use a nested compositor"),
        ),
        _ => Check::warn("session", &session, tr!("nessuna sessione grafica rilevata", "no graphical session detected")),
    });
    for d in &r.gpu.devices {
        let no_node = tr!("nessun nodo render", "no render node");
        let mut detail = format!(
            "{} · driver {} · {}",
            d.pci_slot.as_deref().unwrap_or("?"),
            d.kernel_driver.as_deref().unwrap_or("?"),
            d.render_node.as_deref().unwrap_or(&no_node)
        );
        if let Some(v) = d.vram_bytes {
            detail.push_str(&format!(" · {:.1} GiB VRAM", gib(v)));
        }
        if d.drives_display() {
            let outs = d.connected_outputs.join(", ");
            detail.push_str(&tr!(" · pilota il display ({outs})", " · drives the display ({outs})"));
        }
        c.push(Check::info(&format!("gpu_{}", d.pci_slot.as_deref().unwrap_or("x")), d.display_name(), &detail));
    }
    if r.gpu.devices.is_empty() {
        c.push(Check::warn("gpu", "GPU", tr!("nessun dispositivo DRM rilevato", "no DRM device detected")));
    }
    if let Some(n) = &r.gpu.nvidia {
        let flavor = match n.module_flavor {
            crate::gpu::NvidiaModuleFlavor::Open => "open".to_string(),
            crate::gpu::NvidiaModuleFlavor::Proprietary => tr!("chiuso", "closed"),
            crate::gpu::NvidiaModuleFlavor::Unknown => "?".to_string(),
        };
        let modeset = match n.modeset {
            Some(true) => tr!("attivo", "on"),
            Some(false) => tr!("disattivo", "off"),
            None => "?".to_string(),
        };
        let yes = |b: bool| if b { tr!("sì", "yes") } else { "no".to_string() };
        let (gbm, eglw) = (yes(n.gbm_backend), yes(n.egl_wayland));
        let version = n.version.as_deref().unwrap_or("?");
        c.push(Check::info(
            "nvidia",
            tr!("Driver NVIDIA", "NVIDIA driver"),
            tr!(
                "{version} · modulo {flavor} · modeset {modeset} · backend GBM {gbm} · egl-wayland {eglw}",
                "{version} · {flavor} module · modeset {modeset} · GBM backend {gbm} · egl-wayland {eglw}"
            ),
        ));
    }
    c.push(match &r.gpu.mesa {
        Some(m) => Check::info("mesa", "Mesa", format!("{} ({})", m.version, m.source)),
        None => Check::info("mesa", "Mesa", tr!("non rilevato tramite pacchetti", "not found among packages")),
    });
    let v = &r.gpu.vulkan;
    let icds: Vec<String> = v
        .icds
        .iter()
        .map(|i| format!("{}{}", i.file.trim_end_matches(".json"), i.api_version.as_deref().map(|a| format!(" {a}")).unwrap_or_default()))
        .collect();
    let icd_list = if icds.is_empty() { tr!("nessuno", "none") } else { icds.join(", ") };
    c.push(Check::info("vulkan", "Vulkan", format!("loader {} · ICD: {icd_list}", v.loader_version.as_deref().unwrap_or("?"))));
    for dev in &v.devices {
        c.push(Check::info(
            "vulkan_device",
            tr!("Dispositivo Vulkan", "Vulkan device"),
            format!("{} · {} · API {}", dev.name, dev.driver.as_deref().unwrap_or("?"), dev.api_version.as_deref().unwrap_or("?")),
        ));
    }
    if !v.vulkaninfo_available {
        c.push(Check::info(
            "vulkaninfo",
            "vulkaninfo",
            tr!("non installato: dispositivi Vulkan non enumerati (pacchetto vulkan-tools)", "not installed: Vulkan devices not listed (vulkan-tools package)"),
        ));
    }
    let gl_label = tr!("OpenGL host", "Host OpenGL");
    c.push(match &r.gpu.opengl {
        Some(gl) => Check::info("opengl", &gl_label, format!("{} · {}", gl.renderer.as_deref().unwrap_or("?"), gl.version.as_deref().unwrap_or("?"))),
        None => Check::info(
            "opengl",
            &gl_label,
            tr!("glxinfo/eglinfo non installati: versione non rilevabile (pacchetto mesa-utils)", "glxinfo/eglinfo not installed: version unknown (mesa-utils package)"),
        ),
    });
    c.push(Check::info(
        "udmabuf",
        "/dev/udmabuf",
        match (r.gpu.udmabuf.exists, r.gpu.udmabuf.accessible) {
            (true, true) => tr!("presente e accessibile", "present and accessible"),
            (true, false) => tr!("presente, non accessibile all'utente", "present, not accessible to the user"),
            _ => tr!("assente", "missing"),
        },
    ));
    c
}

fn runtime_checks(r: &HostReport) -> Vec<Check> {
    let t = &r.tools;
    let w = &r.waydroid;
    let mut c = Vec::new();
    c.push(match &t.lxc.path {
        Some(_) => Check::pass("lxc", "LXC", t.lxc.version.clone().unwrap_or_else(|| tr!("installato", "installed"))),
        None => Check::missing("lxc", "LXC", tr!("non installato (dipendenza di Waydroid)", "not installed (Waydroid dependency)")),
    });
    c.push(if w.installed {
        Check::pass("waydroid", "Waydroid", format!("{} ({})", w.version.as_deref().unwrap_or("?"), w.binary.as_deref().unwrap_or("")))
    } else {
        Check::missing("waydroid", "Waydroid", tr!("non installato", "not installed"))
            .remedy(tr!("Repository ufficiale repo.waydro.id, disponibile per questa release", "Official repo.waydro.id repository, available for this release"))
    });
    let images = tr!("Immagini Android", "Android images");
    c.push(if w.initialized {
        let n = w.images.len();
        let vendor = w.config.as_ref().and_then(|c| c.vendor_type.clone()).unwrap_or_default();
        Check::pass("images", &images, tr!("{n} immagini · {vendor}", "{n} images · {vendor}"))
    } else {
        Check::missing(
            "images",
            &images,
            tr!("non inizializzate (download con verifica SHA-256 dal canale OTA ufficiale)", "not initialized (download with SHA-256 check from the official OTA channel)"),
        )
    });
    if w.installed {
        let svc = tr!("Servizio container", "Container service");
        c.push(if w.container_service.is_active() {
            Check::pass("container_service", &svc, tr!("waydroid-container.service attivo", "waydroid-container.service active"))
        } else if let Some(n) = &w.stale_render_node {
            let (old, new) = (&n.configured, &n.nvidia);
            Check::fail(
                "container_service",
                &svc,
                tr!(
                    "non parte: la GPU NVIDIA ora è {new}, Waydroid usa ancora {old} (numerazione cambiata al riavvio)",
                    "won't start: the NVIDIA GPU is now {new}, Waydroid still uses {old} (numbering changed at reboot)"
                ),
            )
            .remedy(tr!("Installazione guidata → «Avviare il servizio del container»", "Guided setup → “Start the container service”"))
        } else {
            let state = w.container_service.active.clone().unwrap_or_else(|| tr!("non trovato", "not found"));
            Check::missing("container_service", &svc, format!("waydroid-container.service: {state}"))
        });
    }
    let net_ok = t.dnsmasq.path.is_some() && (t.nft.path.is_some() || t.iptables.path.is_some());
    let net = tr!("Rete del container", "Container network");
    c.push(if net_ok {
        Check::pass("network", &net, tr!("dnsmasq + nftables/iptables presenti", "dnsmasq + nftables/iptables present"))
    } else {
        Check::missing("network", &net, tr!("servono dnsmasq e nftables o iptables", "dnsmasq and nftables or iptables are needed"))
    });
    let fw = &r.firewall;
    let anet = tr!("Rete di Android", "Android network");
    c.push(match (&w.dhcp_lease, fw.may_block_container()) {
        (Some(ip), _) => Check::pass("android_network", &anet, tr!("indirizzo {ip} assegnato dal DHCP del container", "address {ip} assigned by the container DHCP")),
        (None, true) => {
            let (i, f) = (fw.input_policy.as_deref().unwrap_or("?"), fw.forward_policy.as_deref().unwrap_or("?"));
            Check::warn(
                "android_network",
                &anet,
                tr!(
                    "firewall UFW attivo (ingresso {i}, inoltro {f}): blocca DHCP, DNS e inoltro su waydroid0, Android resta senza rete",
                    "UFW firewall active (input {i}, forward {f}): it blocks DHCP, DNS and forwarding on waydroid0, so Android has no network"
                ),
            )
            .remedy(tr!(
                "Installazione guidata → «Consentire la rete del container» (regole limitate a waydroid0)",
                "Guided setup → “Allow the container network” (rules limited to waydroid0)"
            ))
        }
        (None, false) => Check::info(
            "android_network",
            &anet,
            tr!("nessun blocco del firewall rilevato; l'indirizzo compare al primo avvio", "no firewall block detected; the address appears on first start"),
        ),
    });
    if w.nvidia_setup.is_some() && w.subsurface == Some(true) && !crate::nvidia_stack::kwin_compositor(r.session.compositor.as_deref()) {
        let comp = r.session.compositor.clone().unwrap_or_else(|| "?".into());
        c.push(
            Check::warn(
                "subsurface",
                tr!("Composizione di waydroid-nvidia", "waydroid-nvidia composition"),
                tr!(
                    "subsurface diretto attivo con {comp}: è testato solo su KWin e qui può far cadere Android e lasciare scie (waydroid-nvidia #21)",
                    "direct subsurface on with {comp}: it is tested on KWin only and here it can bring Android down and leave trails (waydroid-nvidia #21)"
                ),
            )
            .remedy(tr!("Installazione guidata → «Configurare il backend GPU»", "Guided setup → “Set up the GPU backend”")),
        );
    }
    if w.initialized {
        let google = tr!("Servizi Google", "Google services");
        c.push(match w.system_image {
            Some(crate::extras::SystemImage::Gapps) => Check::pass("google_services", &google, tr!("Play Store e Google Play Services presenti (immagine GAPPS)", "Play Store and Google Play Services present (GAPPS image)")),
            _ => Check::info(
                "google_services",
                &google,
                tr!("assenti (immagine senza Google): le app che li richiedono non funzionano", "missing (image without Google): apps that need them won't work"),
            )
            .remedy(tr!("Installazione guidata → «Servizi Google (Play Store)»", "Guided setup → “Google services (Play Store)”")),
        });
        let arm = tr!("Traduzione ARM", "ARM translation");
        c.push(if let Some(t) = w.arm_translator {
            let name = t.as_arg();
            Check::pass("arm_translation", &arm, tr!("{name} attiva: app con solo codice ARM supportate", "{name} active: ARM-only apps supported"))
        } else {
            Check::info("arm_translation", &arm, tr!("assente: le app con solo codice ARM non partono", "missing: ARM-only apps won't start"))
                .remedy(tr!("Installazione guidata → «Traduzione ARM»", "Guided setup → “ARM translation”"))
        });
    }
    let pk = tr!("Elevazione privilegi (polkit)", "Privilege elevation (polkit)");
    c.push(match &t.pkexec.path {
        Some(p) => Check::pass("pkexec", &pk, p),
        None => Check::missing("pkexec", &pk, tr!("pkexec assente: le operazioni amministrative non sono eseguibili dalla GUI", "pkexec missing: administrative tasks cannot run from the app")),
    });
    let existing = match (w.config_present, w.user_data_present, &w.nvidia_setup) {
        (false, false, None) => tr!("nessuna", "none"),
        (cfg, data, nv) => format!(
            "{}{}{}",
            if cfg { tr!("config Waydroid presente; ", "Waydroid config present; ") } else { String::new() },
            if data { tr!("dati utente in ~/.local/share/waydroid; ", "user data in ~/.local/share/waydroid; ") } else { String::new() },
            nv.as_deref().map(|p| format!("waydroid-nvidia ({p})")).unwrap_or_default()
        ),
    };
    c.push(Check::info("existing", tr!("Installazioni preesistenti", "Existing installations"), existing));
    c
}

pub fn build_report(host: HostReport) -> CompatibilityReport {
    let groups = vec![
        CheckGroup { title: tr!("Sistema", "System"), checks: system_checks(&host) },
        CheckGroup { title: tr!("Kernel e container", "Kernel and container"), checks: kernel_checks(&host) },
        CheckGroup { title: tr!("Grafica", "Graphics"), checks: graphics_checks(&host) },
        CheckGroup { title: tr!("Runtime Android", "Android runtime"), checks: runtime_checks(&host) },
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
        md.push_str(&tr!("# Report di compatibilità DroidNative\n\n", "# DroidNative compatibility report\n\n"));
        md.push_str(&format!(
            "Host: **{}** · {} · kernel {} · DroidNative {}\n\n",
            h.hostname.as_deref().unwrap_or("?"),
            h.os.pretty_name,
            h.kernel.release,
            h.droidnative_version
        ));
        let verdict = match self.verdict {
            Verdict::Compatible => tr!("compatibile", "compatible"),
            Verdict::CompatibleAfterSetup => tr!("compatibile dopo l'installazione dei componenti mancanti", "compatible once the missing components are installed"),
            Verdict::Incompatible => tr!("non compatibile", "not compatible"),
        };
        md.push_str(&tr!("**Esito: {verdict}**\n\n", "**Result: {verdict}**\n\n"));
        let (check_col, detail_col) = (tr!("Controllo", "Check"), tr!("Dettaglio", "Detail"));
        for g in &self.groups {
            md.push_str(&format!("## {}\n\n| | {check_col} | {detail_col} |\n|---|---|---|\n", g.title));
            for c in &g.checks {
                let mut detail = c.detail.replace('|', "\\|");
                if let Some(r) = &c.remedy {
                    detail.push_str(&format!(" — *{}*", r.replace('|', "\\|")));
                }
                md.push_str(&format!("| {} | {} | {} |\n", c.status.symbol(), c.label, detail));
            }
            md.push('\n');
        }
        md.push_str(&tr!("## Backend GPU\n\n", "## GPU backends\n\n"));
        let rec = crate::gpu::backends::descriptor(self.gpu_plan.recommended);
        let (name, reason) = (rec.name, &self.gpu_plan.reason);
        md.push_str(&tr!("**Consigliato: {name}** — {reason}\n\n", "**Recommended: {name}** — {reason}\n\n"));
        for a in &self.gpu_plan.assessments {
            let readiness = match a.readiness {
                Readiness::Ready => tr!("pronto (accelerazione da verificare all'avvio)", "ready (acceleration to be verified at start)"),
                Readiness::NeedsSetup => tr!("compatibile, richiede installazione", "compatible, needs installation"),
                Readiness::Unsupported => tr!("non supportato su questo sistema", "not supported on this system"),
            };
            md.push_str(&format!("### {} — {readiness}\n\n{}\n\n", a.descriptor.name, a.descriptor.summary));
            if let Some(t) = &a.target_device {
                let node = a.target_render_node.as_deref().map(|n| format!(" ({n})")).unwrap_or_default();
                md.push_str(&tr!("Dispositivo: {t}{node}\n\n", "Device: {t}{node}\n\n"));
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
        md.push_str(&tr!(
            "> Nessun backend è dichiarato attivo finché DroidNative non legge il renderer reale di SurfaceFlinger (`droidnative verify-gpu`).\n",
            "> No backend is reported active until DroidNative reads SurfaceFlinger's real renderer (`droidnative verify-gpu`).\n"
        ));
        md
    }
}
