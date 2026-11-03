#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![deny(unsafe_code)]

mod fonts;
mod native;
mod view;

use rusteditor_editor_core::{Document, DocumentId, Selection};
use rusteditor_file_io::open_utf8;
use rusteditor_plugin_host::{PluginHost, SharedDocument};
use std::{cell::RefCell, ffi::OsString, path::PathBuf, rc::Rc};

const UPPERCASE: &str = "org.rusteditor.text-tools.uppercase";
const SMOKE_TEXT: &str = "before | 你好 straße🙂 e\u{301} | after";

#[derive(Default)]
struct Options {
    plugin_dir: Option<PathBuf>,
    no_plugins: bool,
    smoke: bool,
    empty: bool,
    open: Option<PathBuf>,
}

fn options(mut args: impl Iterator<Item = OsString>) -> Result<Option<Options>, String> {
    let mut result = Options::default();
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
            Some("--open") => {
                if result.open.is_some() {
                    return Err("--open may be supplied once".into());
                }
                let path = args
                    .next()
                    .filter(|arg| !arg.is_empty() && !arg.as_encoded_bytes().starts_with(b"-"))
                    .ok_or("--open requires a path (prefix dash-leading names with ./)")?;
                result.open = Some(PathBuf::from(path));
            }
            Some("--no-plugins") => result.no_plugins = true,
            Some("--smoke-test") => result.smoke = true,
            Some("--empty") => result.empty = true,
            Some("--help" | "-h") => {
                println!(
                    "RustEditor\n\n  --plugin-dir PATH  Enable trusted native plugins in PATH at startup\n  --no-plugins       Start without loading any plugins\n  --smoke-test       Verify the core/host path without opening a window\n  --empty            Open an empty scratch buffer (for GUI validation)\n  --open PATH        Open a UTF-8 file without BOM (up to 4 MiB)\n\n--open cannot be combined with --empty or --smoke-test. Saving is not yet available.\nDefault: plugins next to the executable. Native code has host-process privileges."
                );
                return Ok(None);
            }
            _ => return Err(format!("unknown argument: {}", arg.to_string_lossy())),
        }
    }
    if result.no_plugins && result.plugin_dir.is_some() {
        return Err("choose either --no-plugins or --plugin-dir".into());
    }
    if result.open.is_some() && (result.empty || result.smoke) {
        return Err("--open cannot be combined with --empty or --smoke-test".into());
    }
    if result.empty && result.smoke {
        return Err("--empty requires a GUI run; --smoke-test uses its fixed fixture".into());
    }
    Ok(Some(result))
}

fn main() {
    if let Err(error) = run() {
        eprintln!("RustEditor: {error}");
        std::process::exit(1);
    }
}

fn initial_document(options: &Options) -> Result<Document, String> {
    if let Some(path) = &options.open {
        let text = open_utf8(path).map_err(|error| format!("{}: {error}", path.display()))?;
        return Document::new(DocumentId(1), text).map_err(|error| error.to_string());
    }
    let text = if options.smoke {
        SMOKE_TEXT
    } else if options.empty {
        ""
    } else {
        "Hello, RustEditor! 你好，世界！ straße 👩‍💻 e\u{301}\n\nSelect text, then use Plugins → Uppercase selection (Ctrl+Shift+U).\n\nThis scratch buffer is temporary. Start with --open PATH to open a file.\n"
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
        text.find('!').map_or(0, |index| index + 1)
    };
    document
        .set_selection(Selection {
            anchor: start,
            head: end,
        })
        .map_err(|e| e.to_string())?;
    Ok(document)
}

fn run() -> Result<(), String> {
    let Some(options) = options(std::env::args_os().skip(1))? else {
        return Ok(());
    };
    let document = initial_document(&options)?;
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
            if doc.text() != SMOKE_TEXT {
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
    let filename = options
        .open
        .as_deref()
        .and_then(std::path::Path::file_name)
        .map(|name| name.to_string_lossy().into_owned());
    let native = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1120.0, 760.0])
            .with_min_inner_size([720.0, 480.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "RustEditor",
        native,
        Box::new(move |cc| {
            Ok(Box::new(view::EditorApp::new(
                cc,
                document,
                host,
                diagnostics,
                filename,
            )))
        }),
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_rejects_conflicting_or_incomplete_arguments() {
        for args in [
            &["--open", "file", "--empty"][..],
            &["--smoke-test", "--open", "file"],
            &["--open", "file", "--open", "other"],
            &["--open"],
            &["--open", "--no-plugins"],
            &["--open", ""],
        ] {
            assert!(
                options(args.iter().map(OsString::from)).is_err(),
                "{args:?}"
            );
        }
    }

    #[test]
    fn open_accepts_path_with_spaces_and_either_plugin_mode() {
        let path = "some directory/中文.rs";
        for plugin_options in [&["--no-plugins"][..], &["--plugin-dir", "trusted"]] {
            let options = options(
                ["--open", path]
                    .into_iter()
                    .chain(plugin_options.iter().copied())
                    .map(OsString::from),
            )
            .unwrap()
            .unwrap();
            assert_eq!(options.open, Some(PathBuf::from(path)));
            assert_eq!(options.no_plugins, plugin_options == ["--no-plugins"]);
            assert_eq!(options.plugin_dir.is_some(), !options.no_plugins);
        }
    }

    #[test]
    fn fixture_opens_in_the_core_and_retains_edit_history() {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/probe.rs");
        let original = std::fs::read(&path).unwrap();
        let mut doc = initial_document(&Options {
            open: Some(path.clone()),
            ..Options::default()
        })
        .unwrap();
        assert_eq!(doc.text().as_bytes(), original);
        assert_eq!(doc.selection(), Selection::default());
        doc.replace(0..0, "你好🙂 e\u{301}").unwrap();
        assert!(doc.text().starts_with("你好🙂 e\u{301}"));
        assert!(doc.undo().unwrap());
        assert_eq!(doc.text().as_bytes(), original);
        assert!(doc.redo().unwrap());
        assert!(doc.text().starts_with("你好🙂 e\u{301}"));
        assert_eq!(std::fs::read(path).unwrap(), original);
    }
}
