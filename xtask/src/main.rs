#![forbid(unsafe_code)]
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn cargo(args: &[&str]) -> Result<(), String> {
    let status = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .current_dir(root())
        .args(args)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("cargo {} failed: {status}", args.join(" ")))
    }
}

fn target_directory() -> Result<PathBuf, String> {
    // Respect Cargo's target-dir setting without guessing a relative path.
    let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .current_dir(root())
        .args(["metadata", "--no-deps", "--format-version=1", "--locked"])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
    value["target_directory"]
        .as_str()
        .map(PathBuf::from)
        .ok_or("Cargo target directory missing".into())
}

fn stage() -> Result<PathBuf, String> {
    cargo(&[
        "build",
        "--locked",
        "-p",
        "rusteditor-text-tools",
        "-p",
        "rusteditor-desktop",
    ])?;
    let directory = target_directory()?.join("debug");
    let plugins = directory.join("plugins");
    let destination = plugins.join("text-tools");
    fs::create_dir_all(&destination).map_err(|e| e.to_string())?;
    let library = format!(
        "{}rusteditor_text_tools{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    fs::copy(directory.join(&library), destination.join(&library)).map_err(|e| {
        format!("could not stage plugin; close the editor before replacing a loaded DLL: {e}")
    })?;
    let manifest = format!(
        "id = \"org.rusteditor.text-tools\"\nname = \"Text tools\"\nversion = \"0.1.0\"\nabi_major = 1\nabi_minor = 0\ntarget = {:?}\nlibrary = {:?}\ncapabilities = [\"read-selection\", \"replace-text\", \"commands\"]\n",
        rusteditor_plugin_api::TARGET,
        library
    );
    fs::write(destination.join("plugin.toml"), manifest).map_err(|e| e.to_string())?;
    println!("Staged text-tools in {}", destination.display());
    Ok(plugins)
}

fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => {
            println!(
                "cargo xtask stage   Build desktop and stage the native text-tools plugin\ncargo xtask verify  Run fmt, clippy, tests, then real-plugin/no-plugin headless checks\nClose the editor before staging. All commands use the locked debug build for the host target."
            );
            Ok(())
        }
        [command] if command == "--help" || command == "help" => {
            println!("Subcommands: stage, verify (debug / host target only)");
            Ok(())
        }
        [command] if command == "stage" => stage().map(|_| ()),
        [command] if command == "verify" => {
            cargo(&["fmt", "--all", "--", "--check"])?;
            cargo(&[
                "clippy",
                "--workspace",
                "--all-targets",
                "--locked",
                "--",
                "-D",
                "warnings",
            ])?;
            cargo(&["test", "--workspace", "--locked"])?;
            let _plugins = stage()?;
            cargo(&[
                "run",
                "--locked",
                "-p",
                "rusteditor-desktop",
                "--",
                "--smoke-test",
            ])?;
            cargo(&[
                "run",
                "--locked",
                "-p",
                "rusteditor-desktop",
                "--",
                "--smoke-test",
                "--no-plugins",
            ])
        }
        _ => Err("unknown xtask arguments; use cargo xtask --help".into()),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask: {error}");
            ExitCode::FAILURE
        }
    }
}
