use rusteditor_editor_core::{Document, DocumentId, Selection};
use rusteditor_plugin_api::TARGET;
use rusteditor_plugin_host::{PluginHost, SharedDocument};
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    rc::Rc,
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

struct Binaries {
    text_tools: PathBuf,
    probes: BTreeMap<&'static str, PathBuf>,
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn binaries() -> &'static Binaries {
    static BINARIES: OnceLock<Binaries> = OnceLock::new();
    BINARIES.get_or_init(|| {
        let output = Command::new(env!("CARGO"))
            .current_dir(root())
            .args([
                "build",
                "--locked",
                "-p",
                "rusteditor-text-tools",
                "--message-format=json",
            ])
            .output()
            .expect("build real cdylib");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut artifacts = BTreeMap::new();
        for line in String::from_utf8(output.stdout).unwrap().lines() {
            let Ok(value) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if value["reason"] != "compiler-artifact" {
                continue;
            }
            let name = value["target"]["name"].as_str().unwrap().to_owned();
            let files = value["filenames"].as_array().unwrap();
            for file in files {
                let path = PathBuf::from(file.as_str().unwrap());
                if matches!(path.extension().and_then(|e| e.to_str()), Some("rlib"))
                    || path.extension().and_then(|e| e.to_str())
                        == Some(std::env::consts::DLL_EXTENSION)
                {
                    artifacts.insert(name.clone(), path);
                }
            }
        }
        let api = &artifacts["rusteditor_plugin_api"];
        let sdk = &artifacts["rusteditor_plugin_sdk"];
        let directory = api
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("m0-fixtures")
            .join(std::process::id().to_string());
        fs::create_dir_all(&directory).unwrap();
        let mut probes = BTreeMap::new();
        for mode in [
            "valid",
            "missing-symbol",
            "bad-major",
            "short-table",
            "missing-capabilities",
            "duplicate-command",
            "init-failure",
            "panic",
            "stale",
            "wrong-thread",
            "shutdown-failure",
            "abort",
        ] {
            let name = format!("probe_{}", mode.replace('-', "_"));
            let path = directory.join(format!(
                "{}{name}{}",
                std::env::consts::DLL_PREFIX,
                std::env::consts::DLL_SUFFIX
            ));
            let output = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
                .current_dir(root())
                .args([
                    "--crate-type=cdylib",
                    "--edition=2024",
                    "--crate-name",
                    &name,
                ])
                .arg("--cfg")
                .arg(format!("fixture=\"{mode}\""))
                .arg("--extern")
                .arg(format!("rusteditor_plugin_api={}", api.display()))
                .arg("--extern")
                .arg(format!("rusteditor_plugin_sdk={}", sdk.display()))
                .arg("-L")
                .arg(format!("dependency={}", api.parent().unwrap().display()))
                .arg("tests/fixtures/probe.rs")
                .arg("-o")
                .arg(&path)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "fixture {mode}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            probes.insert(mode, path);
        }
        Binaries {
            text_tools: artifacts["rusteditor_text_tools"].clone(),
            probes,
        }
    })
}

struct Installation(PathBuf);
impl Installation {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "rusteditor-m0-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn plugin(&self, directory: &str, id: &str, binary: &Path) -> PathBuf {
        let path = self.0.join(directory);
        fs::create_dir_all(&path).unwrap();
        let file = binary.file_name().unwrap().to_str().unwrap();
        fs::copy(binary, path.join(file)).unwrap();
        fs::write(path.join("plugin.toml"), format!("id = {id:?}\nname = \"Test fixture\"\nversion = \"0.1.0\"\nabi_major = 1\nabi_minor = 0\ntarget = {TARGET:?}\nlibrary = {file:?}\ncapabilities = [\"read-selection\", \"replace-text\", \"commands\"]\n")).unwrap();
        path
    }
}
impl Drop for Installation {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn host() -> (SharedDocument, PluginHost) {
    let document = Rc::new(RefCell::new(
        Document::new(DocumentId(17), "你好 straße🙂 e\u{301}\r\n").unwrap(),
    ));
    document
        .borrow_mut()
        .set_selection(Selection {
            anchor: 7,
            head: 14,
        })
        .unwrap();
    (document.clone(), PluginHost::new(document))
}

fn load(host: &mut PluginHost, installation: &Installation) -> rusteditor_plugin_host::LoadReport {
    // SAFETY: This test installs only its own, contract-observing fixture code.
    unsafe { host.load_directory(&installation.0) }.unwrap()
}

#[test]
fn real_text_tools_changes_selection_in_one_undoable_transaction() {
    let installation = Installation::new();
    installation.plugin(
        "text-tools",
        "org.rusteditor.text-tools",
        &binaries().text_tools,
    );
    let (document, mut host) = host();
    let original = document.borrow().text().to_owned();
    let report = load(&mut host, &installation);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(host.commands().len(), 1);
    host.invoke("org.rusteditor.text-tools.uppercase").unwrap();
    assert_eq!(document.borrow().text(), "你好 STRASSE🙂 e\u{301}\r\n");
    assert_eq!(document.borrow().revision().0, 2);
    document.borrow_mut().undo().unwrap();
    assert_eq!(document.borrow().text(), original);
    document.borrow_mut().redo().unwrap();
    assert_eq!(document.borrow().selected_text(), "STRASSE");
    assert!(host.disable("org.rusteditor.text-tools"));
    assert_eq!(host.loaded_count(), 1);
    assert!(host.commands().is_empty());
    assert!(host.invoke("org.rusteditor.text-tools.uppercase").is_err());
    host.shutdown();
    host.shutdown();
}

#[test]
fn starts_without_plugins_and_rejects_runtime_loading() {
    let installation = Installation::new();
    let (_, mut host) = host();
    let report = load(&mut host, &installation);
    assert!(report.loaded.is_empty() && report.errors.is_empty());
    assert!(host.commands().is_empty());
    // SAFETY: Empty, trusted fixture directory; second loading must be rejected.
    assert!(unsafe { host.load_directory(&installation.0) }.is_err());
}

#[test]
fn rejects_native_negotiation_and_initialization_failures_without_commands() {
    for mode in [
        "missing-symbol",
        "bad-major",
        "short-table",
        "missing-capabilities",
        "duplicate-command",
        "init-failure",
    ] {
        let installation = Installation::new();
        installation.plugin("probe", "org.rusteditor.probe", &binaries().probes[mode]);
        let (_, mut host) = host();
        let report = load(&mut host, &installation);
        assert!(report.loaded.is_empty(), "{mode}");
        assert_eq!(report.errors.len(), 1, "{mode}");
        assert!(host.commands().is_empty(), "{mode}");
        if mode == "init-failure" || mode == "duplicate-command" {
            let logs = host.logs();
            assert!(logs[logs.len() - 2].ends_with("shutdown"), "{logs:?}");
            assert!(logs[logs.len() - 1].ends_with("destroy"), "{logs:?}");
        }
    }
}

#[test]
fn catches_panics_rejects_stale_replacements_and_wrong_thread_calls() {
    for (mode, status) in [("panic", 5), ("stale", 3), ("wrong-thread", 7)] {
        let installation = Installation::new();
        installation.plugin("probe", "org.rusteditor.probe", &binaries().probes[mode]);
        let (document, mut host) = host();
        assert!(load(&mut host, &installation).errors.is_empty());
        let before = document.borrow().text().to_owned();
        let error = host.invoke("org.rusteditor.probe.run").unwrap_err();
        assert!(
            error.0.ends_with(&format!("status {status}")),
            "{mode}: {error}"
        );
        if mode == "stale" {
            assert!(document.borrow().text().contains("changed"));
            assert!(!document.borrow().text().contains("must not apply"));
            assert_eq!(document.borrow().revision().0, 2);
        } else {
            assert_eq!(document.borrow().text(), before);
        }
        host.shutdown();
    }
}

#[test]
fn cleanup_order_and_failed_shutdown_retention() {
    for mode in ["valid", "shutdown-failure"] {
        let installation = Installation::new();
        installation.plugin("probe", "org.rusteditor.probe", &binaries().probes[mode]);
        let (_, mut host) = host();
        assert!(load(&mut host, &installation).errors.is_empty());
        let logs = host.shutdown();
        assert!(host.commands().is_empty());
        if mode == "valid" {
            assert_eq!(
                logs,
                [
                    "org.rusteditor.probe: initialize",
                    "org.rusteditor.probe: shutdown",
                    "org.rusteditor.probe: destroy"
                ]
            );
        } else {
            assert!(!logs.iter().any(|s| s.ends_with("destroy")));
            assert!(logs.last().unwrap().contains("retained until process exit"));
        }
    }
}

#[test]
fn rejects_duplicate_ids_target_mismatch_and_path_escape_before_loading() {
    let installation = Installation::new();
    let binary = &binaries().probes["valid"];
    installation.plugin("one", "org.rusteditor.probe", binary);
    installation.plugin("two", "org.rusteditor.probe", binary);
    let (_, mut host1) = host();
    let report = load(&mut host1, &installation);
    assert!(report.loaded.is_empty());
    assert!(report.errors[0].0.contains("duplicate plugin ID"));
    assert!(host1.logs().is_empty());
    for (key, value) in [
        ("target", "wrong-target"),
        ("library", "../escape.so"),
        ("library", "..\\escape.dll"),
        ("capabilities", "unknown"),
    ] {
        let installation = Installation::new();
        let dir = installation.plugin("probe", "org.rusteditor.probe", binary);
        let manifest = fs::read_to_string(dir.join("plugin.toml")).unwrap();
        let manifest = manifest
            .lines()
            .map(|line| {
                if line.starts_with(&format!("{key} =")) {
                    if key == "capabilities" {
                        format!("capabilities = [{value:?}]")
                    } else {
                        format!("{key} = {value:?}")
                    }
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(dir.join("plugin.toml"), manifest).unwrap();
        let (_, mut host) = host();
        assert!(!load(&mut host, &installation).errors.is_empty());
        assert!(host.logs().is_empty());
    }
}

#[test]
fn abort_probe_child() {
    let Some(directory) = std::env::var_os("RUSTEDITOR_ABORT_PROBE") else {
        return;
    };
    let (_, mut host) = host();
    // SAFETY: This deliberately aborting fixture is enabled only in a child process.
    let report = unsafe { host.load_directory(Path::new(&directory)) }.unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    println!("[ABORT_PROBE:READY]");
    host.invoke("org.rusteditor.probe.run").unwrap();
    panic!("aborting plugin unexpectedly returned");
}

#[test]
fn native_abort_runs_only_in_a_subprocess() {
    let installation = Installation::new();
    installation.plugin("probe", "org.rusteditor.probe", &binaries().probes["abort"]);
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "abort_probe_child", "--nocapture"])
        .env("RUSTEDITOR_ABORT_PROBE", &installation.0)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&output.stdout).contains("[ABORT_PROBE:READY]"));
    assert!(!output.status.success());
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(output.status.signal(), Some(6));
    }
    #[cfg(windows)]
    assert_ne!(
        output.status.code(),
        Some(101),
        "Rust panic is not a native abort"
    );
}

#[cfg(unix)]
#[test]
fn rejects_symlinks_outside_the_explicit_root() {
    use std::os::unix::fs::symlink;
    let outside = Installation::new();
    let plugin = outside.plugin("probe", "org.rusteditor.probe", &binaries().probes["valid"]);
    let installation = Installation::new();
    symlink(plugin, installation.0.join("escaped")).unwrap();
    let (_, mut host) = host();
    let report = load(&mut host, &installation);
    assert!(report.loaded.is_empty());
    assert!(report.errors[0].0.contains("escapes root"));
}
