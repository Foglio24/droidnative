//! Android runtimes behind one interface, so Waydroid today and ReDroid later are interchangeable.

pub mod waydroid;

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::apk::ApkInfo;
use crate::exec::CmdOutput;
use crate::gpu::AccelVerdict;
use crate::privileged::{Escalator, PrivError};
use crate::tr;

#[derive(Debug)]
pub enum RuntimeError {
    NotInstalled(&'static str),
    NotInitialized,
    SessionNotRunning,
    InvalidInput(String),
    CommandFailed { command: String, output: String },
    Timeout(String),
    Verification(String),
    /// waydroid-nvidia refuses to start the container: see `nvidia_stack::StaleRenderNode`.
    StaleRenderNode(crate::nvidia_stack::StaleRenderNode),
    Privileged(PrivError),
    Io(std::io::Error),
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::NotInstalled(what) => tr!("{what} non è installato", "{what} is not installed"),
            Self::NotInitialized => tr!(
                "Android non è inizializzato: esegui prima l'inizializzazione delle immagini",
                "Android is not initialized: download the images first"
            ),
            Self::SessionNotRunning => tr!("la sessione Android non è attiva", "the Android session is not running"),
            Self::InvalidInput(m) => tr!("input non valido: {m}", "invalid input: {m}"),
            Self::CommandFailed { command, output } => tr!("comando `{command}` fallito: {output}", "command `{command}` failed: {output}"),
            Self::Timeout(what) => tr!("tempo scaduto: {what}", "timed out: {what}"),
            Self::Verification(m) => tr!("verifica fallita: {m}", "verification failed: {m}"),
            Self::StaleRenderNode(n) => {
                let (old, new) = (&n.configured, &n.nvidia);
                tr!(
                    "Android non può partire: dopo il riavvio la GPU NVIDIA è diventata {new}, ma Waydroid usa ancora {old}. Apri Installazione e premi Esegui su «Avviare il servizio del container»",
                    "Android cannot start: after the reboot the NVIDIA GPU became {new}, but Waydroid still uses {old}. Open Setup and press Run on “Start the container service”"
                )
            }
            Self::Privileged(e) => e.to_string(),
            Self::Io(e) => e.to_string(),
        };
        f.write_str(&text)
    }
}

impl std::error::Error for RuntimeError {}

impl From<PrivError> for RuntimeError {
    fn from(e: PrivError) -> Self {
        Self::Privileged(e)
    }
}

impl From<std::io::Error> for RuntimeError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl RuntimeError {
    pub fn failed(command: &str, out: &CmdOutput) -> Self {
        let output = if out.timed_out { "timeout".to_string() } else { out.combined() };
        Self::CommandFailed { command: command.to_string(), output }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Stopped,
    Running,
    Frozen,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeStatus {
    pub backend: String,
    pub session: SessionState,
    pub container: Option<String>,
    pub vendor_type: Option<String>,
    pub ip_address: Option<String>,
    pub session_user: Option<String>,
    pub wayland_display: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AndroidApp {
    pub name: String,
    pub package: String,
    pub categories: Vec<String>,
    /// Launcher icon exported by Waydroid's session (`…/waydroid/data/icons/<package>.png`).
    pub icon: Option<String>,
}

/// What Android was configured to use for graphics (not proof of acceleration).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GraphicsConfig {
    pub egl: Option<String>,
    pub gralloc: Option<String>,
    pub vulkan: Option<String>,
    pub gbm_device: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallOutcome {
    pub package: String,
    /// The package appeared in the installed app list after the install call.
    pub verified: bool,
    pub was_update: bool,
    pub output: String,
}

pub trait AndroidRuntime: Send + Sync {
    fn id(&self) -> &'static str;
    fn status(&self) -> Result<RuntimeStatus, RuntimeError>;
    /// Start the user session and wait until Android answers.
    fn start_session(&self) -> Result<RuntimeStatus, RuntimeError>;
    fn stop_session(&self) -> Result<(), RuntimeError>;
    fn restart_session(&self) -> Result<RuntimeStatus, RuntimeError> {
        if self.status()?.session != SessionState::Stopped {
            self.stop_session()?;
        }
        self.start_session()
    }
    fn show_full_ui(&self) -> Result<(), RuntimeError>;
    fn list_apps(&self) -> Result<Vec<AndroidApp>, RuntimeError>;
    fn install_apk(&self, apk: &ApkInfo) -> Result<InstallOutcome, RuntimeError>;
    /// Remove an app; `escalator` is used only if the unprivileged path fails.
    /// Returns how the removal was performed.
    fn uninstall(&self, package: &str, escalator: &dyn Escalator) -> Result<String, RuntimeError>;
    fn launch(&self, package: &str) -> Result<(), RuntimeError>;
    fn get_prop(&self, key: &str) -> Result<Option<String>, RuntimeError>;
    fn graphics_config(&self) -> Result<GraphicsConfig, RuntimeError>;
    /// Read the renderer SurfaceFlinger really uses (needs privileges).
    fn verify_acceleration(&self, escalator: &dyn Escalator) -> Result<AccelVerdict, RuntimeError>;
    fn log_tail(&self, lines: usize) -> Result<String, RuntimeError>;
}

/// Android package names: dot-separated Java identifiers. Also rejects anything starting with `-`,
/// so a crafted name can never be read as a command-line option.
pub fn validate_package_name(name: &str) -> Result<(), RuntimeError> {
    let ok = !name.is_empty()
        && name.len() <= 255
        && name.split('.').all(|seg| {
            let mut chars = seg.chars();
            chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        });
    if ok {
        Ok(())
    } else {
        Err(RuntimeError::InvalidInput(tr!("nome package non valido: {name:?}", "invalid package name: {name:?}")))
    }
}

pub fn validate_prop_key(key: &str) -> Result<(), RuntimeError> {
    let ok = !key.is_empty()
        && key.len() <= 256
        && !key.starts_with('-')
        && key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if ok {
        Ok(())
    } else {
        Err(RuntimeError::InvalidInput(tr!("chiave di proprietà non valida: {key:?}", "invalid property key: {key:?}")))
    }
}

pub fn validate_apk_path(path: &Path) -> Result<std::path::PathBuf, RuntimeError> {
    let canonical = std::fs::canonicalize(path).map_err(|e| RuntimeError::InvalidInput(format!("{}: {e}", path.display())))?;
    if !canonical.is_file() {
        let shown = canonical.display();
        return Err(RuntimeError::InvalidInput(tr!("{shown} non è un file", "{shown} is not a file")));
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_names() {
        for ok in ["com.example.app", "org.fdroid.fdroid", "android", "a_b.c1"] {
            assert!(validate_package_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", "-rf", "com..x", "1com.x", "com.x;rm", "com.x y", "../etc"] {
            assert!(validate_package_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn prop_keys() {
        assert!(validate_prop_key("ro.hardware.egl").is_ok());
        assert!(validate_prop_key("persist.waydroid.multi_windows").is_ok());
        assert!(validate_prop_key("--help").is_err());
        assert!(validate_prop_key("a b").is_err());
    }
}
