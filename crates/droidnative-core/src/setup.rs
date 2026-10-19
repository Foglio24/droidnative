//! Guided setup: the ordered steps that take this host from its current state to a running,
//! GPU-accelerated Android. Each step is either done, runnable (a privileged op), or blocked.

use serde::Serialize;

use crate::gpu::{GpuBackendKind, GpuPlan};
use crate::nvidia_stack::NvidiaMode;
use crate::host::HostReport;
use crate::privileged::{validate_suite, GpuSelection, PrivilegedOp, SystemType, DIAGNOSTIC_PACKAGES};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Done,
    Pending,
    Blocked,
    Optional,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetupStep {
    pub id: &'static str,
    pub title: String,
    pub detail: String,
    pub state: StepState,
    /// Present when DroidNative can perform the step itself (after polkit authentication).
    pub op: Option<PrivilegedOp>,
    /// Human readable description of `op`, shown before asking for confirmation.
    pub action: Option<String>,
}

impl SetupStep {
    fn with_action(mut self) -> Self {
        self.action = self.op.as_ref().map(PrivilegedOp::description);
        self
    }
}

pub fn plan(host: &HostReport, gpu: &GpuPlan) -> Vec<SetupStep> {
    let w = &host.waydroid;
    let mut steps = Vec::new();

    let suite = host.os.upstream_codename.clone().filter(|s| validate_suite(s).is_ok());
    steps.push(match (w.installed, &suite) {
        (true, _) => SetupStep {
            id: "install_waydroid",
            title: "Installare Waydroid".into(),
            detail: if w.nvidia_setup.is_some() {
                format!("Waydroid patchata da waydroid-nvidia: {}", w.version.as_deref().unwrap_or("?"))
            } else {
                format!("installato: {}", w.version.as_deref().unwrap_or("?"))
            },
            state: StepState::Done,
            op: None,
            action: None,
        },
        (false, Some(s)) => SetupStep {
            id: "install_waydroid",
            title: "Installare Waydroid".into(),
            detail: format!("repository ufficiale repo.waydro.id, suite {s}; la chiave viene verificata prima dell'uso"),
            state: StepState::Pending,
            op: Some(PrivilegedOp::InstallWaydroid { suite: s.clone() }),
            action: None,
        },
        (false, None) => SetupStep {
            id: "install_waydroid",
            title: "Installare Waydroid".into(),
            detail: format!(
                "la release {:?} non è pubblicata su repo.waydro.id",
                host.os.upstream_codename.as_deref().unwrap_or("sconosciuta")
            ),
            state: StepState::Blocked,
            op: None,
            action: None,
        },
    });

    steps.push(SetupStep {
        id: "init_images",
        title: "Scaricare le immagini Android".into(),
        detail: if w.initialized {
            format!("{} immagini presenti", w.images.len())
        } else {
            "LineageOS 20 (Android 13) dal canale OTA ufficiale, verificate con SHA-256 da Waydroid".into()
        },
        state: if w.initialized {
            StepState::Done
        } else if w.installed {
            StepState::Pending
        } else {
            StepState::Blocked
        },
        op: (!w.initialized && w.installed).then_some(PrivilegedOp::InitWaydroid { system_type: SystemType::Vanilla }),
        action: None,
    });

    let service_active = w.container_service.is_active();
    steps.push(SetupStep {
        id: "container_service",
        title: "Avviare il servizio del container".into(),
        detail: format!("waydroid-container.service: {}", w.container_service.active.as_deref().unwrap_or("non presente")),
        state: if service_active {
            StepState::Done
        } else if w.initialized {
            StepState::Pending
        } else {
            StepState::Blocked
        },
        op: (!service_active && w.initialized).then_some(PrivilegedOp::EnableContainerService),
        action: None,
    });

    let blocked_by_fw = host.firewall.may_block_container();
    steps.push(SetupStep {
        id: "container_network",
        title: "Consentire la rete del container".into(),
        detail: match (&w.dhcp_lease, blocked_by_fw) {
            (Some(ip), _) => format!("Android ha l'indirizzo {ip}"),
            (None, true) => "il firewall UFW scarta DHCP e DNS di Android e il suo traffico inoltrato: verranno aggiunte 3 regole limitate all'interfaccia waydroid0".into(),
            (None, false) => "nessun firewall che blocchi il bridge waydroid0".into(),
        },
        state: match (&w.dhcp_lease, blocked_by_fw, w.installed) {
            (Some(_), _, _) | (None, false, _) => StepState::Done,
            (None, true, true) => StepState::Pending,
            (None, true, false) => StepState::Blocked,
        },
        op: (w.dhcp_lease.is_none() && blocked_by_fw && w.installed).then_some(PrivilegedOp::AllowContainerNetwork),
        action: None,
    });

    let rec = gpu.assessments.iter().find(|a| a.descriptor.kind == gpu.recommended);
    let gpu_step = match (gpu.recommended, rec) {
        (GpuBackendKind::MesaNative, Some(a)) => {
            let node = a.target_render_node.clone();
            let configured = w.config.as_ref().and_then(|c| c.drm_device.clone());
            let done = node.is_some() && configured == node;
            SetupStep {
                id: "gpu_backend",
                title: "Configurare il backend GPU Mesa Native".into(),
                detail: format!("{} → {}", a.target_device.as_deref().unwrap_or("?"), node.as_deref().unwrap_or("?")),
                state: if done {
                    StepState::Done
                } else if w.initialized {
                    StepState::Pending
                } else {
                    StepState::Blocked
                },
                op: (!done && w.initialized).then(|| node.map(|n| PrivilegedOp::SelectGpu { selection: GpuSelection::RenderNode(n) })).flatten(),
                action: None,
            }
        }
        (kind @ (GpuBackendKind::NvidiaAccelerated | GpuBackendKind::NvidiaExperimental), Some(a)) => {
            let want = if kind == GpuBackendKind::NvidiaAccelerated { NvidiaMode::Venus } else { NvidiaMode::Prolapse };
            let release = crate::nvidia_stack::latest_release();
            let (state, detail, op) = match (w.nvidia_setup.is_some(), w.nvidia_mode) {
                (true, Some(m)) if m == want => (
                    StepState::Done,
                    format!("waydroid-nvidia attivo in modalità {}; dopo ogni aggiornamento del driver NVIDIA ripeti questo passo", m.as_arg()),
                    None,
                ),
                (true, _) => (
                    StepState::Pending,
                    format!("waydroid-nvidia installato: attivare la modalità {}", want.as_arg()),
                    Some(PrivilegedOp::NvidiaSetMode { mode: want }),
                ),
                (false, _) => (
                    StepState::Pending,
                    format!(
                        "installa waydroid-nvidia {} (modalità {}) sostituendo il pacchetto waydroid della distribuzione; immagini e dati restano",
                        release.tag,
                        want.as_arg()
                    ),
                    Some(PrivilegedOp::InstallNvidiaStack { tag: release.tag.to_string(), mode: want }),
                ),
            };
            SetupStep { id: "gpu_backend", title: format!("Configurare il backend GPU {}", a.descriptor.name), detail, state, op, action: None }
        }
        _ => SetupStep {
            id: "gpu_backend",
            title: "Backend GPU".into(),
            detail: "nessun backend hardware compatibile: verrà usato il rendering software".into(),
            state: StepState::Done,
            op: None,
            action: None,
        },
    };
    steps.push(gpu_step);

    let have_diag = host.gpu.opengl.is_some() && host.gpu.vulkan.vulkaninfo_available;
    steps.push(SetupStep {
        id: "diagnostics",
        title: "Strumenti diagnostici (facoltativo)".into(),
        detail: format!("{} per leggere versioni OpenGL e dispositivi Vulkan dell'host", DIAGNOSTIC_PACKAGES.join(", ")),
        state: if have_diag { StepState::Done } else { StepState::Optional },
        op: (!have_diag).then_some(PrivilegedOp::InstallDiagnostics),
        action: None,
    });
    steps.into_iter().map(SetupStep::with_action).collect()
}
