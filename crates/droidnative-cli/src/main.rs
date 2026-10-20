//! `droidnative`: command line front end and the privileged helper run through pkexec.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
use droidnative_core::apk::inspect_apk;
use droidnative_core::exec::SystemRunner;
use droidnative_core::fs::HostFs;
use droidnative_core::gpu::assess_backends;
use droidnative_core::host::probe_local;
use droidnative_core::i18n::{lang, Lang};
use droidnative_core::tr;
use droidnative_core::extras::Translator;
use droidnative_core::nvidia_stack::NvidiaMode;
use droidnative_core::privileged::{escalator_from_env, execute, Escalator, GpuSelection, PrivilegedOp, SystemType};
use droidnative_core::report::build_report;
use droidnative_core::runtime::waydroid::WaydroidRuntime;
use droidnative_core::runtime::{AndroidRuntime, WindowMode};
use droidnative_core::setup;

#[derive(Parser)]
#[command(name = "droidnative", version, about = "Native Android apps on Debian/Ubuntu Linux")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Text,
    Json,
    Markdown,
}

#[derive(Subcommand)]
enum Command {
    /// Analyze the system and print the compatibility report.
    Doctor {
        #[arg(long, value_enum, default_value = "text")]
        format: Format,
    },
    /// Assess the available GPU backends.
    Gpu {
        #[arg(long)]
        json: bool,
    },
    /// Switch waydroid-nvidia mode: `venus` (NVIDIA Accelerated) or `nvidia-prolapse` (Experimental).
    NvidiaMode {
        #[arg(value_parser = parse_mode)]
        mode: NvidiaMode,
    },
    /// Change Android's GPU backend: a /dev/dri/renderD* node, `software` or `auto`
    /// (needs authentication; restarts the container if running).
    GpuSelect {
        target: String,
    },
    /// Guided setup steps and their state.
    Setup {
        /// Run the given step (needs administrator authentication).
        #[arg(long)]
        run: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Android runtime status.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Control the Android session.
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },
    /// Show the full Android UI in a window.
    Ui,
    /// Inspect an APK without installing it.
    Apk {
        file: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Manage Android apps.
    App {
        #[command(subcommand)]
        action: AppAction,
    },
    /// Verify the renderer Android really uses (needs authentication).
    VerifyGpu {
        #[arg(long)]
        json: bool,
    },
    /// Show the Google device ID to register at google.com/android/uncertified (needs authentication).
    GoogleId,
    /// Show or change how apps reach the desktop: `single` (one Android window) or `multi`
    /// (every app in its own window); changing it restarts Android.
    Windows {
        #[arg(value_enum)]
        mode: Option<WindowModeArg>,
    },
    /// Switch the Android system image: GAPPS (Google Play) or VANILLA; apps and data are kept
    /// (needs authentication).
    SystemImage {
        #[arg(value_enum)]
        system_type: SystemTypeArg,
    },
    /// Install, switch or remove the ARM translation (needs authentication).
    ArmTranslation {
        #[command(subcommand)]
        action: ArmTranslationAction,
    },
    /// Last lines of the runtime logs.
    Logs {
        #[arg(short, long, default_value_t = 80)]
        lines: usize,
    },
    /// Administrative operations (through pkexec only).
    #[command(hide = true)]
    Priv {
        /// Language of the messages (pkexec clears the caller's locale).
        #[arg(long)]
        lang: Option<String>,
        #[command(subcommand)]
        op: PrivCommand,
    },
}

#[derive(Subcommand)]
enum SessionAction {
    Start,
    Stop,
    Restart,
}

#[derive(Subcommand)]
enum ArmTranslationAction {
    /// Download the pinned translator, verify it and activate it in place of the other one.
    Install {
        /// libndk or libhoudini (default: the one recommended for this CPU).
        #[arg(long, value_parser = parse_translator)]
        translator: Option<Translator>,
    },
    /// Remove its files, properties and binfmt_misc handlers.
    Remove,
}

#[derive(Subcommand)]
enum AppAction {
    List {
        #[arg(long)]
        json: bool,
    },
    Install {
        file: PathBuf,
    },
    Launch {
        package: String,
    },
    Remove {
        package: String,
    },
    /// Deliver mouse clicks to an app as touch (for games that ignore the mouse); applies the
    /// next time the app opens. Without arguments, list the apps that have it.
    Touch {
        package: Option<String>,
        #[arg(value_enum, requires = "package")]
        state: Option<OnOff>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum OnOff {
    On,
    Off,
}

#[derive(Clone, Copy, ValueEnum)]
enum WindowModeArg {
    Single,
    Multi,
}

#[derive(Subcommand)]
enum PrivCommand {
    InstallWaydroid {
        #[arg(long)]
        suite: String,
    },
    InstallDiagnostics,
    AllowContainerNetwork,
    LoadBinder,
    EnableContainer,
    RestartContainer,
    StopContainer,
    Init {
        #[arg(long, value_enum, default_value = "vanilla")]
        system_type: SystemTypeArg,
    },
    SelectGpu(SelectGpuArgs),
    GraphicsDiagnostics,
    UninstallPackage {
        package: String,
    },
    InstallNvidiaStack {
        #[arg(long)]
        tag: String,
        #[arg(long, value_parser = parse_mode)]
        mode: NvidiaMode,
    },
    NvidiaMode {
        #[arg(long, value_parser = parse_mode)]
        mode: NvidiaMode,
    },
    SwitchImage {
        #[arg(long, value_enum)]
        system_type: SystemTypeArg,
    },
    InstallArmTranslation {
        #[arg(long, value_parser = parse_translator, default_value = "libndk")]
        translator: Translator,
    },
    RemoveArmTranslation,
    GoogleDeviceId,
    SetSubsurface {
        #[arg(value_enum)]
        state: OnOff,
    },
    /// Runs as root from droidnative-render-node.service before every container start.
    RepinRenderNode {
        #[arg(long)]
        start: bool,
    },
}

fn parse_translator(s: &str) -> Result<Translator, String> {
    Translator::from_arg(s).ok_or_else(|| tr!("traduttore sconosciuto {s:?}: usa libndk o libhoudini", "unknown translator {s:?}: use libndk or libhoudini"))
}

fn parse_mode(s: &str) -> Result<NvidiaMode, String> {
    NvidiaMode::from_arg(s).ok_or_else(|| tr!("modalità sconosciuta {s:?}: usa venus o nvidia-prolapse", "unknown mode {s:?}: use venus or nvidia-prolapse"))
}

#[derive(Clone, Copy, ValueEnum)]
enum SystemTypeArg {
    #[value(name = "VANILLA", alias = "vanilla")]
    Vanilla,
    #[value(name = "GAPPS", alias = "gapps")]
    Gapps,
}

impl From<SystemTypeArg> for SystemType {
    fn from(a: SystemTypeArg) -> Self {
        match a {
            SystemTypeArg::Vanilla => SystemType::Vanilla,
            SystemTypeArg::Gapps => SystemType::Gapps,
        }
    }
}

#[derive(Args)]
#[group(required = true, multiple = false)]
struct SelectGpuArgs {
    #[arg(long)]
    render_node: Option<String>,
    #[arg(long)]
    software: bool,
    #[arg(long)]
    auto: bool,
}

impl PrivCommand {
    fn into_op(self) -> PrivilegedOp {
        match self {
            Self::InstallWaydroid { suite } => PrivilegedOp::InstallWaydroid { suite },
            Self::InstallDiagnostics => PrivilegedOp::InstallDiagnostics,
            Self::AllowContainerNetwork => PrivilegedOp::AllowContainerNetwork,
            Self::LoadBinder => PrivilegedOp::LoadBinder,
            Self::EnableContainer => PrivilegedOp::EnableContainerService,
            Self::RestartContainer => PrivilegedOp::RestartContainerService,
            Self::StopContainer => PrivilegedOp::StopContainerService,
            Self::Init { system_type } => PrivilegedOp::InitWaydroid { system_type: system_type.into() },
            Self::SelectGpu(a) => PrivilegedOp::SelectGpu {
                selection: match (a.render_node, a.software) {
                    (Some(n), _) => GpuSelection::RenderNode(n),
                    (None, true) => GpuSelection::Software,
                    _ => GpuSelection::Auto,
                },
            },
            Self::GraphicsDiagnostics => PrivilegedOp::GraphicsDiagnostics,
            Self::UninstallPackage { package } => PrivilegedOp::UninstallPackage { package },
            Self::InstallNvidiaStack { tag, mode } => PrivilegedOp::InstallNvidiaStack { tag, mode },
            Self::NvidiaMode { mode } => PrivilegedOp::NvidiaSetMode { mode },
            Self::SwitchImage { system_type } => PrivilegedOp::SwitchSystemImage { system_type: system_type.into() },
            Self::InstallArmTranslation { translator } => PrivilegedOp::InstallArmTranslation { translator },
            Self::RemoveArmTranslation => PrivilegedOp::RemoveArmTranslation,
            Self::GoogleDeviceId => PrivilegedOp::GoogleDeviceId,
            Self::RepinRenderNode { start } => PrivilegedOp::RepinRenderNode { start_container: start },
            Self::SetSubsurface { state } => PrivilegedOp::SetSubsurface { enabled: matches!(state, OnOff::On) },
        }
    }
}

/// Runs a privileged operation, echoing what it does and its output.
fn run_op(op: &PrivilegedOp) -> Result<(), String> {
    eprintln!("→ {}", op.description());
    let out = escalator().run(op).map_err(|x| x.to_string())?;
    println!("{}", out.combined());
    if out.success() {
        Ok(())
    } else {
        Err(tr!("operazione non riuscita", "operation failed"))
    }
}

fn escalator() -> Box<dyn Escalator> {
    let helper = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("/usr/bin/droidnative"));
    escalator_from_env(helper, Arc::new(SystemRunner))
}

fn print_json<T: serde::Serialize>(v: &T) {
    println!("{}", serde_json::to_string_pretty(v).expect("serializable"));
}

fn run(cli: Cli) -> Result<(), String> {
    let e = |err: &dyn std::fmt::Display| err.to_string();
    match cli.command {
        Command::Doctor { format } => {
            let report = build_report(probe_local());
            match format {
                Format::Json => print_json(&report),
                Format::Markdown => print!("{}", report.to_markdown()),
                Format::Text => {
                    for g in &report.groups {
                        println!("\n== {} ==", g.title);
                        for c in &g.checks {
                            println!("{} {:<34} {}", c.status.symbol(), c.label, c.detail);
                            if let Some(r) = &c.remedy {
                                println!("   ↳ {r}");
                            }
                        }
                    }
                    let name = droidnative_core::gpu::backends::descriptor(report.gpu_plan.recommended).name;
                    let reason = &report.gpu_plan.reason;
                    println!("\n{}", tr!("Backend GPU consigliato: {name} — {reason}", "Recommended GPU backend: {name} — {reason}"));
                    let verdict = format!("{:?}", report.verdict);
                    println!("{}", tr!("Esito: {verdict}", "Result: {verdict}"));
                }
            }
        }
        Command::Gpu { json } => {
            let plan = assess_backends(&probe_local());
            if json {
                print_json(&plan);
            } else {
                for a in &plan.assessments {
                    println!("\n{} [{:?}] — {:?}", a.descriptor.name, a.descriptor.stability, a.readiness);
                    if let Some(t) = &a.target_device {
                        let node = a.target_render_node.as_deref().unwrap_or("");
                        println!("  {}", tr!("dispositivo: {t} {node}", "device: {t} {node}"));
                    }
                    for c in &a.checks {
                        println!("  {} {}: {}", c.status.symbol(), c.label, c.detail);
                    }
                }
                let (rec, reason) = (format!("{:?}", plan.recommended), &plan.reason);
                println!("\n{}", tr!("Consigliato: {rec} — {reason}", "Recommended: {rec} — {reason}"));
            }
        }
        Command::NvidiaMode { mode } => {
            let op = PrivilegedOp::NvidiaSetMode { mode };
            eprintln!("→ {}", op.description());
            let out = escalator().run(&op).map_err(|x| e(&x))?;
            println!("{}", out.combined());
            if !out.success() {
                return Err(tr!("cambio modalità fallito", "mode switch failed"));
            }
        }
        Command::GpuSelect { target } => {
            let selection = match target.as_str() {
                "software" => GpuSelection::Software,
                "auto" => GpuSelection::Auto,
                node => GpuSelection::RenderNode(node.to_string()),
            };
            let op = PrivilegedOp::SelectGpu { selection };
            eprintln!("→ {}", op.description());
            let out = escalator().run(&op).map_err(|x| e(&x))?;
            if !out.success() {
                return Err(out.combined());
            }
            println!(
                "{}",
                tr!(
                    "Configurazione applicata. Verifica il renderer reale con `droidnative verify-gpu` dopo l'avvio.",
                    "Configuration applied. Check the real renderer with `droidnative verify-gpu` after starting Android."
                )
            );
        }
        Command::Setup { run, json } => {
            let host = probe_local();
            let plan = assess_backends(&host);
            let steps = setup::plan(&host, &plan);
            if let Some(id) = run {
                let step = steps.iter().find(|s| s.id == id).ok_or_else(|| tr!("passo sconosciuto: {id}", "unknown step: {id}"))?;
                let detail = &step.detail;
                let op = step.op.as_ref().ok_or_else(|| tr!("il passo {id} non è eseguibile: {detail}", "step {id} cannot be run: {detail}"))?;
                eprintln!("→ {}", op.description());
                let out = escalator().run(op).map_err(|x| e(&x))?;
                println!("{}", out.combined());
                if !out.success() {
                    return Err(tr!("passo {id} fallito", "step {id} failed"));
                }
            } else if json {
                print_json(&steps);
            } else {
                for s in &steps {
                    println!("[{:?}] {:<18} {} — {}", s.state, s.id, s.title, s.detail);
                }
            }
        }
        Command::Status { json } => {
            let st = WaydroidRuntime::local().status().map_err(|x| e(&x))?;
            if json {
                print_json(&st);
            } else {
                let session = format!("{:?}", st.session);
                println!("{}", tr!("Sessione: {session}", "Session: {session}"));
                println!("Container: {}\nIP: {}", st.container.unwrap_or("-".into()), st.ip_address.unwrap_or("-".into()));
            }
        }
        Command::Session { action } => {
            let rt = WaydroidRuntime::local();
            match action {
                SessionAction::Start => println!("{:?}", rt.start_session().map_err(|x| e(&x))?.session),
                SessionAction::Stop => rt.stop_session().map_err(|x| e(&x))?,
                SessionAction::Restart => println!("{:?}", rt.restart_session().map_err(|x| e(&x))?.session),
            }
        }
        Command::Ui => WaydroidRuntime::local().show_full_ui().map_err(|x| e(&x))?,
        Command::Apk { file, json } => {
            let host = probe_local();
            let mut info = inspect_apk(&file, host.cpu.android_abi.as_deref()).map_err(|x| e(&x))?;
            info.abi_compat = droidnative_core::apk::with_translation(info.abi_compat, host.waydroid.arm_translation);
            if json {
                print_json(&info);
            } else {
                let q = |v: Option<String>| v.unwrap_or_else(|| "?".into());
                let version = format!("{} ({})", q(info.version_name.clone()), q(info.version_code.map(|v| v.to_string())));
                let abis = if info.native_abis.is_empty() { tr!("nessuna", "none") } else { info.native_abis.join(", ") };
                println!("Package:    {}", info.package);
                println!("{:<12}{version}", tr!("Versione:", "Version:"));
                println!("SDK:        min {} · target {}", q(info.min_sdk.map(|v| v.to_string())), q(info.target_sdk.map(|v| v.to_string())));
                println!("ABI:        {abis} → {:?}", info.abi_compat);
                println!("{:<12}{}", tr!("Permessi:", "Permissions:"), info.permissions.len());
                println!("SHA-256:    {}", info.sha256);
            }
        }
        Command::App { action } => {
            let rt = WaydroidRuntime::local();
            match action {
                AppAction::List { json } => {
                    let apps = rt.list_apps().map_err(|x| e(&x))?;
                    if json {
                        print_json(&apps);
                    } else {
                        for a in apps {
                            println!("{:<40} {}", a.package, a.name);
                        }
                    }
                }
                AppAction::Install { file } => {
                    let abi = probe_local().cpu.android_abi;
                    let info = inspect_apk(&file, abi.as_deref()).map_err(|x| e(&x))?;
                    let (pkg, sha) = (&info.package, &info.sha256[..16]);
                    eprintln!("{}", tr!("Installazione di {pkg} (sha256 {sha})", "Installing {pkg} (sha256 {sha})"));
                    let out = rt.install_apk(&info).map_err(|x| e(&x))?;
                    let pkg = &out.package;
                    println!(
                        "{}",
                        if out.was_update {
                            tr!("{pkg} aggiornato e verificato", "{pkg} updated and verified")
                        } else {
                            tr!("{pkg} installato e verificato", "{pkg} installed and verified")
                        }
                    );
                }
                AppAction::Launch { package } => rt.launch(&package).map_err(|x| e(&x))?,
                AppAction::Touch { package: None, .. } => {
                    for p in rt.fake_touch().map_err(|x| e(&x))? {
                        println!("{p}");
                    }
                }
                AppAction::Touch { package: Some(package), state } => {
                    let on = !matches!(state, Some(OnOff::Off));
                    rt.set_fake_touch(&package, on).map_err(|x| e(&x))?;
                    println!(
                        "{}",
                        if on {
                            tr!("{package}: mouse come tocco attivo; vale dalla prossima apertura dell'app", "{package}: mouse as touch on; applies the next time the app opens")
                        } else {
                            tr!("{package}: mouse come tocco disattivato; vale dalla prossima apertura dell'app", "{package}: mouse as touch off; applies the next time the app opens")
                        }
                    );
                }
                AppAction::Remove { package } => {
                    let method = rt.uninstall(&package, escalator().as_ref()).map_err(|x| e(&x))?;
                    println!("{}", tr!("{package} rimosso e verificato ({method})", "{package} removed and verified ({method})"));
                }
            }
        }
        Command::VerifyGpu { json } => {
            let rt = WaydroidRuntime::local();
            let verdict = rt.verify_acceleration(escalator().as_ref()).map_err(|x| e(&x))?;
            if json {
                print_json(&verdict);
            } else {
                println!("Rendering: {:?} — {}", verdict.status, verdict.detail);
                if let Some(l) = &verdict.gles_line {
                    println!("  {l}");
                }
                let (status, detail) = (format!("{:?}", verdict.display.status), &verdict.display.detail);
                println!("{}", tr!("Visualizzazione: {status} — {detail}", "Display: {status} — {detail}"));
                for l in &verdict.display.evidence {
                    println!("  {l}");
                }
            }
        }
        Command::GoogleId => {
            let out = escalator().run(&PrivilegedOp::GoogleDeviceId).map_err(|x| e(&x))?;
            if !out.success() {
                return Err(out.combined());
            }
            let id = out.stdout.trim();
            let url = droidnative_core::extras::UNCERTIFIED_URL;
            println!("{id}");
            eprintln!(
                "{}",
                tr!(
                    "Registra questo ID con il tuo account Google su {url}, poi riavvia Android.",
                    "Register this ID with your Google account at {url}, then restart Android."
                )
            );
        }
        Command::Windows { mode } => {
            let rt = WaydroidRuntime::local();
            if let Some(m) = mode {
                let m = match m {
                    WindowModeArg::Single => WindowMode::Single,
                    WindowModeArg::Multi => WindowMode::Multi,
                };
                if m == WindowMode::Multi {
                    let host = probe_local();
                    if let Some(why) = droidnative_core::nvidia_stack::per_app_windows_refusal(host.waydroid.nvidia_setup.is_some(), host.session.compositor.as_deref()) {
                        return Err(why);
                    }
                }
                rt.set_window_mode(m).map_err(|x| e(&x))?;
            }
            println!(
                "{}",
                match rt.window_mode().map_err(|x| e(&x))? {
                    WindowMode::Single => tr!("single: Android in un'unica finestra", "single: Android in one window"),
                    WindowMode::Multi => tr!("multi: ogni app nella sua finestra", "multi: every app in its own window"),
                }
            );
        }
        Command::SystemImage { system_type } => run_op(&PrivilegedOp::SwitchSystemImage { system_type: system_type.into() })?,
        Command::ArmTranslation { action } => run_op(&match action {
            ArmTranslationAction::Install { translator } => PrivilegedOp::InstallArmTranslation {
                translator: translator.unwrap_or_else(|| Translator::recommended(probe_local().cpu.vendor.as_deref())),
            },
            ArmTranslationAction::Remove => PrivilegedOp::RemoveArmTranslation,
        })?,
        Command::Logs { lines } => print!("{}", WaydroidRuntime::local().log_tail(lines).map_err(|x| e(&x))?),
        Command::Priv { lang: requested, op } => {
            if let Some(l) = requested.as_deref().and_then(Lang::from_code) {
                droidnative_core::i18n::set_override(l);
            }
            // SAFETY: geteuid has no preconditions.
            if unsafe { libc::geteuid() } != 0 {
                return Err(tr!("i comandi priv vanno eseguiti come root (tramite pkexec)", "priv commands must run as root (through pkexec)"));
            }
            let op = op.into_op();
            let out = execute(&op, &HostFs::real(), &SystemRunner).map_err(|x| format!("droidnative: {x}"))?;
            print!("{}", out.stdout);
            eprint!("{}", out.stderr);
        }
    }
    Ok(())
}

/// Help texts are written in English; in an Italian session they are swapped at runtime.
const IT_ABOUT: &[(&str, &str)] = &[
    ("doctor", "Analizza il sistema e produce il report di compatibilità."),
    ("gpu", "Valuta i backend GPU disponibili."),
    ("nvidia-mode", "Cambia la modalità di waydroid-nvidia: `venus` (NVIDIA Accelerated) o `nvidia-prolapse` (Experimental)."),
    ("gpu-select", "Cambia il backend GPU di Android: un nodo /dev/dri/renderD*, `software` o `auto` (richiede autenticazione)."),
    ("setup", "Passi di installazione guidata e loro stato."),
    ("status", "Stato del runtime Android."),
    ("session", "Controllo della sessione Android."),
    ("ui", "Mostra l'interfaccia Android completa in una finestra."),
    ("apk", "Ispeziona un APK senza installarlo."),
    ("app", "Gestione delle applicazioni Android."),
    ("verify-gpu", "Verifica il renderer reale usato da Android (richiede autenticazione)."),
    ("google-id", "Mostra l'ID dispositivo Google da registrare su google.com/android/uncertified (richiede autenticazione)."),
    ("windows", "Mostra o cambia come le app compaiono sul desktop: `single` (un'unica finestra Android) o `multi` (una finestra per app); il cambio riavvia Android."),
    ("system-image", "Cambia l'immagine di sistema Android: GAPPS (Google Play) o VANILLA; app e dati restano (richiede autenticazione)."),
    ("arm-translation", "Installa, cambia o rimuove la traduzione ARM (richiede autenticazione)."),
    ("logs", "Ultime righe dei log del runtime."),
];

fn command() -> clap::Command {
    let mut cmd = Cli::command();
    if lang() == Lang::It {
        cmd = cmd.about("Applicazioni Android native su Linux Debian/Ubuntu");
        for (name, about) in IT_ABOUT {
            cmd = cmd.mut_subcommand(*name, |c| c.about(*about));
        }
    }
    cmd
}

fn main() -> ExitCode {
    let cli = Cli::from_arg_matches(&command().get_matches()).unwrap_or_else(|err| err.exit());
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("{} {msg}", droidnative_core::i18n::pick("errore:", "error:"));
            ExitCode::FAILURE
        }
    }
}
