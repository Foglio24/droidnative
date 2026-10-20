//! End-to-end checks of the `droidnative` binary against the machine running the tests.

use std::process::Command;

fn droidnative_in(lang: &str, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_droidnative"))
        .args(args)
        .env_remove("LANGUAGE")
        .env_remove("LC_ALL")
        .env_remove("LC_MESSAGES")
        .env("LANG", lang)
        .output()
        .expect("binary runs")
}

fn droidnative(args: &[&str]) -> std::process::Output {
    droidnative_in("it_IT.UTF-8", args)
}

#[test]
fn doctor_json_is_a_complete_report() {
    let out = droidnative(&["doctor", "--format", "json"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    assert!(v["groups"].as_array().is_some_and(|g| g.len() == 4));
    assert_eq!(v["gpu_plan"]["assessments"].as_array().map(Vec::len), Some(4));
    assert!(v["host"]["kernel"]["release"].as_str().is_some_and(|s| !s.is_empty()));
}

#[test]
fn doctor_markdown_never_claims_unverified_acceleration() {
    let out = droidnative(&["doctor", "--format", "markdown"]);
    assert!(out.status.success());
    let md = String::from_utf8_lossy(&out.stdout);
    assert!(md.starts_with("# Report di compatibilità DroidNative"));
    assert!(md.contains("Nessun backend è dichiarato attivo"));
}

#[test]
fn setup_plan_is_json() {
    let out = droidnative(&["setup", "--json"]);
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    let ids: Vec<&str> = v.as_array().unwrap().iter().filter_map(|s| s["id"].as_str()).collect();
    assert_eq!(ids, ["install_waydroid", "init_images", "container_service", "container_network", "gpu_backend", "google_services", "arm_translation", "diagnostics"]);
}

#[test]
fn priv_refuses_to_run_without_root() {
    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let out = droidnative(&["priv", "load-binder"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("root"));
}

#[test]
fn priv_rejects_ambiguous_gpu_selection() {
    let out = droidnative(&["priv", "select-gpu", "--software", "--auto"]);
    assert!(!out.status.success());
}

#[test]
fn apk_inspection_reports_bad_files() {
    let dir = std::env::temp_dir().join(format!("droidnative-cli-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("broken.apk");
    std::fs::write(&f, b"definitely not a zip").unwrap();
    let out = droidnative(&["apk", f.to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("APK"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn english_outside_italian_locales() {
    let out = droidnative_in("de_DE.UTF-8", &["doctor", "--format", "markdown"]);
    assert!(out.status.success());
    let md = String::from_utf8_lossy(&out.stdout);
    assert!(md.starts_with("# DroidNative compatibility report"), "{md}");
    assert!(md.contains("## GPU backends"));
    let help = droidnative_in("C", &["--help"]);
    assert!(String::from_utf8_lossy(&help.stdout).contains("Analyze the system"));
    let help_it = droidnative_in("it_IT.UTF-8", &["--help"]);
    assert!(String::from_utf8_lossy(&help_it.stdout).contains("Analizza il sistema"));
}

/// pkexec clears the locale, so the helper must follow `--lang` even though the CLI already
/// localized its help. The package name is invalid: nothing runs, as root or not.
#[test]
fn priv_helper_follows_lang_flag() {
    let text = |lang: &str, flag: &str| {
        let out = droidnative_in(lang, &["priv", "--lang", flag, "uninstall-package", "not a package"]);
        assert!(!out.status.success());
        String::from_utf8_lossy(&out.stderr).into_owned()
    };
    let en = text("it_IT.UTF-8", "en");
    assert!(en.contains("must run as root") || en.contains("invalid package name"), "{en}");
    let it = text("C", "it");
    assert!(it.contains("vanno eseguiti come root") || it.contains("nome package non valido"), "{it}");
}
