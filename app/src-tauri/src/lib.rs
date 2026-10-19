//! Tauri commands: thin async wrappers around droidnative-core. Blocking work (process
//! execution, polling) runs on the blocking pool so the UI thread never stalls.

use std::path::PathBuf;
use std::sync::Arc;

use droidnative_core::apk::{inspect_apk, ApkInfo};
use droidnative_core::exec::{CmdOutput, SystemRunner};
use droidnative_core::gpu::AccelVerdict;
use droidnative_core::host::probe_local;
use droidnative_core::tr;
use droidnative_core::nvidia_stack::NvidiaMode;
use droidnative_core::privileged::{escalator_from_env, Escalator, GpuSelection, PrivilegedOp};
use droidnative_core::report::{build_report, CompatibilityReport};
use droidnative_core::runtime::waydroid::WaydroidRuntime;
use droidnative_core::runtime::{AndroidApp, AndroidRuntime, GraphicsConfig, InstallOutcome, RuntimeStatus};
use droidnative_core::setup::{self, SetupStep};
use serde::Serialize;

struct AppState {
    runtime: Arc<WaydroidRuntime>,
    /// polkit (pkexec) by default; `DROIDNATIVE_ESCALATOR=sudo` for sessions without an agent.
    escalator: Arc<dyn Escalator>,
}

/// The privileged helper is the `droidnative` CLI: next to this binary in a cargo target
/// directory during development, /usr/bin/droidnative once installed from the .deb.
fn helper_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join("droidnative")))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("/usr/bin/droidnative"))
}

/// Runs `f` on the blocking pool. With `DROIDNATIVE_TRACE=1` every command logs its name,
/// duration and outcome to stderr, which is how IPC wiring is checked on a real desktop.
async fn blocking<T, F>(name: &'static str, f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    let start = std::time::Instant::now();
    let result = tauri::async_runtime::spawn_blocking(f).await.map_err(|e| tr!("errore interno: {e}", "internal error: {e}"))?;
    if std::env::var_os("DROIDNATIVE_TRACE").is_some() {
        match &result {
            Ok(_) => eprintln!("[droidnative] {name} ok in {:?}", start.elapsed()),
            Err(e) => eprintln!("[droidnative] {name} err in {:?}: {e}", start.elapsed()),
        }
    }
    result
}

fn privileged_output(out: CmdOutput) -> Result<String, String> {
    if out.success() {
        Ok(out.combined())
    } else {
        let code = out.status;
        Err(if out.combined().is_empty() { tr!("operazione fallita (codice {code:?})", "operation failed (code {code:?})") } else { out.combined() })
    }
}

/// Language for the UI, decided with the same rule as every backend message.
#[tauri::command]
fn app_language() -> &'static str {
    droidnative_core::i18n::lang().code()
}

#[tauri::command]
async fn get_report() -> Result<CompatibilityReport, String> {
    blocking("get_report", || Ok(build_report(probe_local()))).await
}

#[derive(Serialize)]
struct SetupView {
    steps: Vec<SetupStep>,
    helper: String,
}

#[tauri::command]
async fn get_setup() -> Result<SetupView, String> {
    blocking("get_setup", || {
        let report = build_report(probe_local());
        Ok(SetupView { steps: setup::plan(&report.host, &report.gpu_plan), helper: helper_path().to_string_lossy().into_owned() })
    })
    .await
}

#[tauri::command]
async fn run_setup_step(state: tauri::State<'_, AppState>, id: String) -> Result<String, String> {
    let esc = state.escalator.clone();
    blocking("run_setup_step", move || {
        let report = build_report(probe_local());
        let steps = setup::plan(&report.host, &report.gpu_plan);
        let step = steps.into_iter().find(|s| s.id == id).ok_or_else(|| tr!("passo sconosciuto: {id}", "unknown step: {id}"))?;
        let detail = step.detail.clone();
        let op = step.op.ok_or_else(|| tr!("il passo non è eseguibile: {detail}", "this step cannot be run: {detail}"))?;
        privileged_output(esc.run(&op).map_err(|e| e.to_string())?)
    })
    .await
}

#[tauri::command]
async fn select_gpu(state: tauri::State<'_, AppState>, selection: GpuSelection) -> Result<String, String> {
    let esc = state.escalator.clone();
    blocking("select_gpu", move || privileged_output(esc.run(&PrivilegedOp::SelectGpu { selection }).map_err(|e| e.to_string())?)).await
}

#[tauri::command]
async fn nvidia_install(state: tauri::State<'_, AppState>, mode: NvidiaMode) -> Result<String, String> {
    let esc = state.escalator.clone();
    let tag = droidnative_core::nvidia_stack::latest_release().tag.to_string();
    blocking("nvidia_install", move || privileged_output(esc.run(&PrivilegedOp::InstallNvidiaStack { tag, mode }).map_err(|e| e.to_string())?)).await
}

#[tauri::command]
async fn nvidia_set_mode(state: tauri::State<'_, AppState>, mode: NvidiaMode) -> Result<String, String> {
    let esc = state.escalator.clone();
    blocking("nvidia_set_mode", move || privileged_output(esc.run(&PrivilegedOp::NvidiaSetMode { mode }).map_err(|e| e.to_string())?)).await
}

#[tauri::command]
async fn runtime_status(state: tauri::State<'_, AppState>) -> Result<RuntimeStatus, String> {
    let rt = state.runtime.clone();
    blocking("runtime_status", move || rt.status().map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn session_start(state: tauri::State<'_, AppState>) -> Result<RuntimeStatus, String> {
    let rt = state.runtime.clone();
    blocking("session_start", move || rt.start_session().map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn session_stop(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let rt = state.runtime.clone();
    blocking("session_stop", move || rt.stop_session().map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn session_restart(state: tauri::State<'_, AppState>) -> Result<RuntimeStatus, String> {
    let rt = state.runtime.clone();
    blocking("session_restart", move || rt.restart_session().map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn container_restart(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let esc = state.escalator.clone();
    blocking("container_restart", move || privileged_output(esc.run(&PrivilegedOp::RestartContainerService).map_err(|e| e.to_string())?)).await
}

#[tauri::command]
async fn show_full_ui(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let rt = state.runtime.clone();
    blocking("show_full_ui", move || rt.show_full_ui().map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn list_apps(state: tauri::State<'_, AppState>) -> Result<Vec<AndroidApp>, String> {
    let rt = state.runtime.clone();
    blocking("list_apps", move || rt.list_apps().map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn inspect_apk_file(path: String) -> Result<ApkInfo, String> {
    blocking("inspect_apk_file", move || {
        let abi = probe_local().cpu.android_abi;
        inspect_apk(&PathBuf::from(path), abi.as_deref()).map_err(|e| e.to_string())
    })
    .await
}

/// Re-inspects the file instead of trusting metadata sent by the UI, then installs and verifies.
#[tauri::command]
async fn install_apk(state: tauri::State<'_, AppState>, path: String, expected_sha256: String) -> Result<InstallOutcome, String> {
    let rt = state.runtime.clone();
    blocking("install_apk", move || {
        let abi = probe_local().cpu.android_abi;
        let info = inspect_apk(&PathBuf::from(path), abi.as_deref()).map_err(|e| e.to_string())?;
        if info.sha256 != expected_sha256 {
            return Err(tr!("il file è cambiato dopo l'ispezione: installazione annullata", "the file changed after inspection: installation cancelled"));
        }
        rt.install_apk(&info).map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
async fn launch_app(state: tauri::State<'_, AppState>, package: String) -> Result<(), String> {
    let rt = state.runtime.clone();
    blocking("launch_app", move || rt.launch(&package).map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn uninstall_app(state: tauri::State<'_, AppState>, package: String) -> Result<String, String> {
    let rt = state.runtime.clone();
    let esc = state.escalator.clone();
    blocking("uninstall_app", move || rt.uninstall(&package, esc.as_ref()).map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn graphics_config(state: tauri::State<'_, AppState>) -> Result<GraphicsConfig, String> {
    let rt = state.runtime.clone();
    blocking("graphics_config", move || rt.graphics_config().map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn verify_gpu(state: tauri::State<'_, AppState>) -> Result<AccelVerdict, String> {
    let rt = state.runtime.clone();
    let esc = state.escalator.clone();
    blocking("verify_gpu", move || rt.verify_acceleration(esc.as_ref()).map_err(|e| e.to_string())).await
}

#[tauri::command]
async fn read_logs(state: tauri::State<'_, AppState>, lines: usize) -> Result<String, String> {
    let rt = state.runtime.clone();
    blocking("read_logs", move || rt.log_tail(lines.min(2000)).map_err(|e| e.to_string())).await
}

pub fn run() {
    let state = AppState {
        runtime: Arc::new(WaydroidRuntime::local()),
        escalator: Arc::from(escalator_from_env(helper_path(), Arc::new(SystemRunner))),
    };
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            app_language,
            get_report,
            get_setup,
            run_setup_step,
            select_gpu,
            nvidia_install,
            nvidia_set_mode,
            runtime_status,
            session_start,
            session_stop,
            session_restart,
            container_restart,
            show_full_ui,
            list_apps,
            inspect_apk_file,
            install_apk,
            launch_app,
            uninstall_app,
            graphics_config,
            verify_gpu,
            read_logs,
        ])
        .run(tauri::generate_context!())
        .expect("error while running DroidNative");
}
