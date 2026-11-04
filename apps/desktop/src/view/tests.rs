use super::*;
use crate::tabs::Tab;
use rusteditor_editor_core::Document;
use rusteditor_file_io::SaveTarget;
use rusteditor_plugin_host::PluginHost;
use std::{cell::RefCell, rc::Rc};

fn frame(app: &mut EditorApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1120.0, 760.0),
            )),
            events,
            ..Default::default()
        },
        |ctx| app.update_frame(ctx),
    );
}

fn app() -> EditorApp {
    let document = Rc::new(RefCell::new(Document::new(DocumentId(1), "").unwrap()));
    EditorApp {
        workspace: Workspace::new(
            vec![Tab::new(document.clone(), None)],
            PluginHost::new(document),
        ),
        diagnostics: Vec::new(),
        allow_close: false,
        font_status: String::new(),
        dialog_ime_active: false,
    }
}

fn shortcut(key: Key, shift: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: if shift {
            Modifiers::COMMAND | Modifiers::SHIFT
        } else {
            Modifiers::COMMAND
        },
    }
}

#[test]
fn new_open_save_close_and_history_availability_follow_the_active_tab() {
    let mut app = app();
    let ctx = egui::Context::default();
    let available = Actions::available(&app.workspace, false);
    assert!(
        available.new && available.open && available.save && available.save_as && available.close
    );
    assert!(!available.undo && !available.redo);
    frame(&mut app, &ctx, vec![]);
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::Text("a".into()), shortcut(Key::Z, false)],
    );
    assert_eq!(app.workspace.active().unwrap().document.borrow().text(), "");
    assert!(Actions::available(&app.workspace, false).redo);
    frame(&mut app, &ctx, vec![shortcut(Key::Z, true)]);
    assert_eq!(
        app.workspace.active().unwrap().document.borrow().text(),
        "a"
    );
    frame(&mut app, &ctx, vec![shortcut(Key::N, false)]);
    assert_eq!(app.workspace.tabs.len(), 2);
    assert!(!Actions::available(&app.workspace, false).undo);
    frame(&mut app, &ctx, vec![shortcut(Key::W, false)]);
    assert_eq!(app.workspace.tabs.len(), 1);
    assert!(Actions::available(&app.workspace, false).undo);
    let blocked = Actions::available(&app.workspace, true);
    assert!(
        !blocked.new
            && !blocked.open
            && !blocked.save
            && !blocked.save_as
            && !blocked.close
            && !blocked.undo
            && !blocked.redo
    );
    app.workspace.remove(DocumentId(1));
    let empty = Actions::available(&app.workspace, false);
    assert!(empty.new && empty.open);
    assert!(!empty.close && !empty.save && !empty.save_as && !empty.undo && !empty.redo);
}

#[test]
fn application_shortcuts_do_not_steal_enabled_preedit_or_commit_frames() {
    for ime in [
        egui::ImeEvent::Enabled,
        egui::ImeEvent::Preedit("ni".into()),
        egui::ImeEvent::Commit("你".into()),
    ] {
        for (key, shift) in [
            (Key::N, false),
            (Key::O, false),
            (Key::S, false),
            (Key::S, true),
            (Key::W, false),
            (Key::Z, false),
            (Key::Y, false),
            (Key::Z, true),
        ] {
            let mut app = app();
            let ctx = egui::Context::default();
            frame(&mut app, &ctx, vec![]);
            frame(
                &mut app,
                &ctx,
                vec![egui::Event::Ime(ime.clone()), shortcut(key, shift)],
            );
            assert_eq!(app.workspace.tabs.len(), 1, "{ime:?} {key:?}");
            assert!(!app.workspace.open.dialog);
            assert!(app.workspace.active().unwrap().save.dialog.is_none());
            if matches!(ime, egui::ImeEvent::Commit(_)) {
                assert_eq!(
                    app.workspace.active().unwrap().document.borrow().text(),
                    "你"
                );
            }
        }
    }
}

#[test]
fn ctrl_o_dialog_opens_a_new_tab_and_ready_read_waits_for_original_frame_input() {
    let path = std::env::temp_dir().join(format!(
        "rusteditor-open-frame-{} 中文.txt",
        std::process::id()
    ));
    std::fs::write(&path, "from disk\r\n").unwrap();
    let mut app = app();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![shortcut(Key::O, false)]);
    assert!(app.workspace.open.dialog);
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::Text(path.display().to_string())],
    );
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }],
    );
    assert!(!app.workspace.open.dialog);
    app.workspace.open.wait_until_ready();
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::Text("pending source text".into())],
    );
    assert_eq!(
        app.workspace
            .tab(DocumentId(1))
            .unwrap()
            .document
            .borrow()
            .text(),
        "pending source text"
    );
    assert_eq!(
        app.workspace.active().unwrap().document.borrow().text(),
        "from disk\r\n"
    );
    assert_eq!(app.workspace.tabs.len(), 2);
    // A clean named document needs no ordinary save, but Save As is available.
    let available = Actions::available(&app.workspace, false);
    assert!(!available.save && available.save_as);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn switching_tabs_cancels_preedit_and_never_replays_it_into_either_document() {
    let document = Rc::new(RefCell::new(
        Document::new(DocumentId(1), "original").unwrap(),
    ));
    let mut app = EditorApp {
        workspace: Workspace::new(
            vec![Tab::new(document.clone(), None)],
            PluginHost::new(document.clone()),
        ),
        diagnostics: Vec::new(),
        allow_close: false,
        font_status: String::new(),
        dialog_ime_active: false,
    };
    let b = app.workspace.new_tab().unwrap();
    app.workspace.activate(DocumentId(1));
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, Vec::new());
    frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::Ime(egui::ImeEvent::Enabled),
            egui::Event::Ime(egui::ImeEvent::Preedit("ni".into())),
        ],
    );
    assert!(app.workspace.active().unwrap().editor.is_composing());
    app.workspace.activate(b);
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::Ime(egui::ImeEvent::Commit("你".into()))],
    );
    assert_eq!(app.workspace.active().unwrap().document.borrow().text(), "");
    frame(
        &mut app,
        &ctx,
        vec![
            egui::Event::Ime(egui::ImeEvent::Enabled),
            egui::Event::Ime(egui::ImeEvent::Preedit("hao".into())),
            egui::Event::Ime(egui::ImeEvent::Commit("好".into())),
            egui::Event::Ime(egui::ImeEvent::Disabled),
        ],
    );
    assert_eq!(
        app.workspace.active().unwrap().document.borrow().text(),
        "好"
    );
    app.workspace.activate(DocumentId(1));
    frame(
        &mut app,
        &ctx,
        vec![egui::Event::Ime(egui::ImeEvent::Commit("你".into()))],
    );
    assert_eq!(document.borrow().text(), "original");
    assert!(!document.borrow().can_undo());
}

#[test]
fn ready_save_cannot_close_a_tab_before_same_frame_text_is_processed() {
    let path =
        std::env::temp_dir().join(format!("rusteditor-frame-save-{}.txt", std::process::id()));
    let document = Rc::new(RefCell::new(Document::new(DocumentId(1), "").unwrap()));
    let mut app = EditorApp {
        workspace: Workspace::new(
            vec![Tab::new(document.clone(), None)],
            PluginHost::new(document.clone()),
        ),
        diagnostics: Vec::new(),
        allow_close: false,
        font_status: String::new(),
        dialog_ime_active: false,
    };
    let b = app.workspace.new_tab().unwrap();
    app.workspace.activate(DocumentId(1));
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, Vec::new());
    document.borrow_mut().replace(0..0, "snapshot").unwrap();
    let tab = app.workspace.active_mut().unwrap();
    tab.start_write(path.clone(), Arc::new(SaveTarget::New));
    tab.save.wait_until_ready();
    app.workspace.request_close(DocumentId(1));
    frame(&mut app, &ctx, vec![egui::Event::Text("new input".into())]);
    assert_eq!(app.workspace.active_id(), Some(DocumentId(1)));
    assert!(app.workspace.tab(DocumentId(1)).unwrap().confirm_close);
    assert!(document.borrow().is_modified());
    assert!(document.borrow().text().contains("new input"));
    assert_eq!(app.workspace.tab(b).unwrap().document.borrow().text(), "");
    assert_eq!(std::fs::read(&path).unwrap(), b"snapshot");
    std::fs::remove_file(path).unwrap();
}
