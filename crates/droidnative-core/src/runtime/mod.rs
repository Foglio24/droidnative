//! Android runtimes behind one interface, so Waydroid today and ReDroid later are interchangeable.

pub mod waydroid;

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::apk::ApkInfo;
use crate::exec::CmdOutput;
use crate::gpu::AccelVerdict;
use crate::privileged::{Escalator, PrivError};

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("{0} non è installato")]
    NotInstalled(&'static str),
    #[error("Android non è inizializzato: esegui prima l'inizializzazione delle immagini")]
    NotInitialized,
    #[error("la sessione Android non è attiva")]
    SessionNotRunning,
    #[error("input non valido: {0}")]
    InvalidInput(String),
    #[error("comando `{command}` fallito: {output}")]
    CommandFailed { command: String, output: String },
    #[error("tempo scaduto: {0}")]
    Timeout(String),
    #[error("verifica fallita: {0}")]
    Verification(String),
    #[error(transparent)]
    Privileged(#[from] PrivError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
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
        Err(RuntimeError::InvalidInput(format!("nome package non valido: {name:?}")))
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
        Err(RuntimeError::InvalidInput(format!("chiave di proprietà non valida: {key:?}")))
    }
}

pub fn validate_apk_path(path: &Path) -> Result<std::path::PathBuf, RuntimeError> {
    let canonical = std::fs::canonicalize(path).map_err(|e| RuntimeError::InvalidInput(format!("{}: {e}", path.display())))?;
    if !canonical.is_file() {
        return Err(RuntimeError::InvalidInput(format!("{} non è un file", canonical.display())));
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
