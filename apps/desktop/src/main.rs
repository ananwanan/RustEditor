#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![deny(unsafe_code)]

mod fonts;
mod native;
mod view;

use rusteditor_editor_core::{Document, DocumentId, Selection};
use rusteditor_plugin_host::{PluginHost, SharedDocument};
use std::{cell::RefCell, path::PathBuf, rc::Rc};

const UPPERCASE: &str = "org.rusteditor.text-tools.uppercase";

#[derive(Default)]
struct Options {
    plugin_dir: Option<PathBuf>,
    no_plugins: bool,
    smoke: bool,
}

fn options() -> Result<Option<Options>, String> {
    let mut result = Options::default();
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--plugin-dir") => {
                if result.plugin_dir.is_some() {
                    return Err("--plugin-dir may be supplied once".into());
                }
                result.plugin_dir = Some(PathBuf::from(
                    args.next().ok_or("--plugin-dir requires a path")?,
                ));
            }
            Some("--no-plugins") => result.no_plugins = true,
            Some("--smoke-test") => result.smoke = true,
            Some("--help" | "-h") => {
                println!(
                    "RustEditor M0\n\n  --plugin-dir PATH  Enable trusted native plugins in PATH at startup\n  --no-plugins       Start without loading any plugins\n  --smoke-test       Verify the core/host path without opening a window\n\nDefault: plugins next to the executable. Native code has host-process privileges.\nScratch text is temporary; file opening and saving arrive in M1."
                );
                return Ok(None);
            }
            _ => return Err(format!("unknown argument: {}", arg.to_string_lossy())),
        }
    }
    if result.no_plugins && result.plugin_dir.is_some() {
        return Err("choose either --no-plugins or --plugin-dir".into());
    }
    Ok(Some(result))
}

fn main() {
    if let Err(error) = run() {
        eprintln!("RustEditor: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let Some(options) = options()? else {
        return Ok(());
    };
    let text = if options.smoke {
        "before | 你好 straße🙂 e\u{301} | after"
    } else {
        "Hello, RustEditor! 你好，世界！ straße 👩‍💻 e\u{301}\n\nSelect text, then use Plugins → Uppercase selection (Ctrl+Shift+U).\n\nThis M0 scratch buffer is temporary. File opening and saving arrive in M1.\n"
    };
    let mut document = Document::new(DocumentId(1), text).map_err(|e| e.to_string())?;
    let start = if options.smoke {
        text.find("你好").expect("sample")
    } else {
        0
    };
    let end = if options.smoke {
        text.find(" | after").expect("sample")
    } else {
        text.find('!').expect("sample") + 1
    };
    document
        .set_selection(Selection {
            anchor: start,
            head: end,
        })
        .map_err(|e| e.to_string())?;
    let document: SharedDocument = Rc::new(RefCell::new(document));
    let mut host = PluginHost::new(document.clone());
    let explicit_dir = options.plugin_dir.is_some();
    let directory = options.plugin_dir.unwrap_or(
        std::env::current_exe()
            .map_err(|e| e.to_string())?
            .parent()
            .ok_or("executable has no parent")?
            .join("plugins"),
    );
    let mut diagnostics = Vec::new();
    if !options.no_plugins && (explicit_dir || directory.exists()) {
        // This adapter is the application's single native-code trust boundary.
        diagnostics = native::load_trusted_plugins(&mut host, &directory)?;
    }
    if options.smoke {
        if !diagnostics.is_empty() {
            return Err(diagnostics.join("\n"));
        }
        if options.no_plugins {
            if host.loaded_count() != 0 || !host.commands().is_empty() {
                return Err("no-plugin startup failed".into());
            }
            println!(
                "[M0:OK] no-plugin startup; document revision {}",
                document.borrow().revision().0
            );
        } else {
            host.invoke(UPPERCASE).map_err(|e| e.to_string())?;
            let mut doc = document.borrow_mut();
            let expected = "before | 你好 STRASSE🙂 E\u{301} | after";
            if doc.text() != expected {
                return Err("dynamic selection transformation failed".into());
            }
            doc.undo().map_err(|e| e.to_string())?;
            if doc.text() != text {
                return Err("plugin transaction undo failed".into());
            }
            doc.redo().map_err(|e| e.to_string())?;
            if doc.text() != expected {
                return Err("plugin transaction redo failed".into());
            }
            println!(
                "[M0:OK] real dynamic plugin; Unicode selection replacement; undo/redo; revision {}",
                doc.revision().0
            );
        }
        host.shutdown();
        return Ok(());
    }
    let native = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1120.0, 760.0])
            .with_min_inner_size([720.0, 480.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "RustEditor · M0",
        native,
        Box::new(move |cc| {
            Ok(Box::new(view::EditorApp::new(
                cc,
                document,
                host,
                diagnostics,
            )))
        }),
    )
    .map_err(|e| e.to_string())
}
