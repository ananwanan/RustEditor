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
