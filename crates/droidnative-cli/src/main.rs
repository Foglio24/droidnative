//! `droidnative`: command line front end and the privileged helper run through pkexec.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Args, Parser, Subcommand, ValueEnum};
use droidnative_core::apk::inspect_apk;
use droidnative_core::exec::SystemRunner;
use droidnative_core::fs::HostFs;
use droidnative_core::gpu::assess_backends;
use droidnative_core::host::probe_local;
use droidnative_core::nvidia_stack::NvidiaMode;
use droidnative_core::privileged::{escalator_from_env, execute, Escalator, GpuSelection, PrivilegedOp, SystemType};
use droidnative_core::report::build_report;
use droidnative_core::runtime::waydroid::WaydroidRuntime;
use droidnative_core::runtime::AndroidRuntime;
use droidnative_core::setup;

#[derive(Parser)]
#[command(name = "droidnative", version, about = "Applicazioni Android native su Linux Debian/Ubuntu")]
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
    /// Analizza il sistema e produce il report di compatibilità.
    Doctor {
        #[arg(long, value_enum, default_value = "text")]
        format: Format,
    },
    /// Valuta i backend GPU disponibili.
    Gpu {
        #[arg(long)]
        json: bool,
    },
    /// Cambia la modalità di waydroid-nvidia: `venus` (NVIDIA Accelerated) o `nvidia-prolapse` (Experimental).
    NvidiaMode {
        #[arg(value_parser = parse_mode)]
        mode: NvidiaMode,
    },
    /// Cambia il backend GPU di Android: un nodo /dev/dri/renderD*, `software` o `auto`
    /// (richiede autenticazione; riavvia il container se attivo).
    GpuSelect {
        target: String,
    },
    /// Passi di installazione guidata e loro stato.
    Setup {
        /// Esegue il passo indicato (richiede autenticazione amministratore).
        #[arg(long)]
        run: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Stato del runtime Android.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Controllo della sessione Android.
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },
    /// Mostra l'interfaccia Android completa in una finestra.
    Ui,
    /// Ispeziona un APK senza installarlo.
    Apk {
        file: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Gestione delle applicazioni Android.
    App {
        #[command(subcommand)]
        action: AppAction,
    },
    /// Verifica il renderer reale usato da Android (richiede autenticazione).
    VerifyGpu {
        #[arg(long)]
        json: bool,
    },
    /// Ultime righe dei log del runtime.
    Logs {
        #[arg(short, long, default_value_t = 80)]
        lines: usize,
    },
    /// Operazioni amministrative (solo tramite pkexec).
    #[command(hide = true)]
    Priv {
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
}

fn parse_mode(s: &str) -> Result<NvidiaMode, String> {
    NvidiaMode::from_arg(s).ok_or_else(|| format!("modalità sconosciuta {s:?}: usa venus o nvidia-prolapse"))
}

#[derive(Clone, Copy, ValueEnum)]
enum SystemTypeArg {
    #[value(name = "VANILLA", alias = "vanilla")]
    Vanilla,
    #[value(name = "GAPPS", alias = "gapps")]
    Gapps,
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
            Self::Init { system_type } => PrivilegedOp::InitWaydroid {
                system_type: match system_type {
                    SystemTypeArg::Vanilla => SystemType::Vanilla,
                    SystemTypeArg::Gapps => SystemType::Gapps,
                },
            },
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
        }
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
                    let rec = droidnative_core::gpu::backends::descriptor(report.gpu_plan.recommended);
                    println!("\nBackend GPU consigliato: {} — {}", rec.name, report.gpu_plan.reason);
                    println!("Esito: {:?}", report.verdict);
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
                        println!("  dispositivo: {t} {}", a.target_render_node.as_deref().unwrap_or(""));
                    }
                    for c in &a.checks {
                        println!("  {} {}: {}", c.status.symbol(), c.label, c.detail);
                    }
                }
                println!("\nConsigliato: {:?} — {}", plan.recommended, plan.reason);
            }
        }
        Command::NvidiaMode { mode } => {
            let op = PrivilegedOp::NvidiaSetMode { mode };
            eprintln!("→ {}", op.description());
            let out = escalator().run(&op).map_err(|x| e(&x))?;
            println!("{}", out.combined());
            if !out.success() {
                return Err("cambio modalità fallito".into());
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
            println!("Configurazione applicata. Verifica il renderer reale con `droidnative verify-gpu` dopo l'avvio.");
        }
        Command::Setup { run, json } => {
            let host = probe_local();
            let plan = assess_backends(&host);
            let steps = setup::plan(&host, &plan);
            if let Some(id) = run {
                let step = steps.iter().find(|s| s.id == id).ok_or_else(|| format!("passo sconosciuto: {id}"))?;
                let op = step.op.as_ref().ok_or_else(|| format!("il passo {id} non è eseguibile: {}", step.detail))?;
                eprintln!("→ {}", op.description());
                let out = escalator().run(op).map_err(|x| e(&x))?;
                println!("{}", out.combined());
                if !out.success() {
                    return Err(format!("passo {id} fallito"));
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
                println!("Sessione: {:?}\nContainer: {}\nIP: {}", st.session, st.container.unwrap_or("-".into()), st.ip_address.unwrap_or("-".into()));
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
            let abi = probe_local().cpu.android_abi;
            let info = inspect_apk(&file, abi.as_deref()).map_err(|x| e(&x))?;
            if json {
                print_json(&info);
            } else {
                println!("Package:    {}", info.package);
                println!("Versione:   {} ({})", info.version_name.as_deref().unwrap_or("?"), info.version_code.map(|v| v.to_string()).unwrap_or("?".into()));
                println!("SDK:        min {} · target {}", info.min_sdk.map(|v| v.to_string()).unwrap_or("?".into()), info.target_sdk.map(|v| v.to_string()).unwrap_or("?".into()));
                println!("ABI:        {} → {:?}", if info.native_abis.is_empty() { "nessuna".into() } else { info.native_abis.join(", ") }, info.abi_compat);
                println!("Permessi:   {}", info.permissions.len());
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
                    eprintln!("Installazione di {} (sha256 {})", info.package, &info.sha256[..16]);
                    let out = rt.install_apk(&info).map_err(|x| e(&x))?;
                    println!("{} {} e verificato", out.package, if out.was_update { "aggiornato" } else { "installato" });
                }
                AppAction::Launch { package } => rt.launch(&package).map_err(|x| e(&x))?,
                AppAction::Remove { package } => {
                    let method = rt.uninstall(&package, escalator().as_ref()).map_err(|x| e(&x))?;
                    println!("{package} rimosso e verificato ({method})");
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
                println!("Visualizzazione: {:?} — {}", verdict.display.status, verdict.display.detail);
                for l in &verdict.display.evidence {
                    println!("  {l}");
                }
            }
        }
        Command::Logs { lines } => print!("{}", WaydroidRuntime::local().log_tail(lines).map_err(|x| e(&x))?),
        Command::Priv { op } => {
            // SAFETY: geteuid has no preconditions.
            if unsafe { libc::geteuid() } != 0 {
                return Err("i comandi priv vanno eseguiti come root (tramite pkexec)".into());
            }
            let op = op.into_op();
            let out = execute(&op, &HostFs::real(), &SystemRunner).map_err(|x| format!("droidnative: {x}"))?;
            print!("{}", out.stdout);
            eprint!("{}", out.stderr);
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("errore: {msg}");
            ExitCode::FAILURE
        }
    }
}
