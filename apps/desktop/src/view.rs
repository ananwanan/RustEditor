#![forbid(unsafe_code)]
use crate::{save::SaveDialog, tabs::Workspace};
use eframe::egui::{self, Color32, Key, Modifiers};
use rusteditor_editor_core::DocumentId;
use std::{path::PathBuf, sync::Arc};

pub struct EditorApp {
    workspace: Workspace,
    diagnostics: Vec<String>,
    allow_close: bool,
    font_status: String,
    dialog_ime_active: bool,
}

#[derive(Clone, Copy)]
struct Actions {
    new: bool,
    open: bool,
    save: bool,
    save_as: bool,
    close: bool,
    undo: bool,
    redo: bool,
}

pub(crate) fn cancel_native_ime(ctx: &egui::Context) {
    // Viewport commands bypass egui-winit 0.33's cached allow_ime flag.
    // Leaving the native context disabled while that cache is true prevents
    // subsequent focused editors from receiving composition events. Cancel the
    // departing session, then restore the context for the arriving editor.
    ctx.send_viewport_cmd(egui::ViewportCommand::IMEAllowed(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::IMEAllowed(true));
}

impl Actions {
    fn available(workspace: &Workspace, blocked: bool) -> Self {
        let enabled = !blocked && !workspace.closing_all;
        let tab = workspace.active();
        let idle = enabled && tab.is_some_and(|tab| !tab.save.busy());
        Self {
            new: enabled && workspace.can_add_tab(),
            open: enabled && workspace.can_add_tab() && !workspace.open.busy(),
            save: idle
                && tab.is_some_and(|tab| {
                    tab.document.borrow().is_modified() || tab.save.association.is_none()
                }),
            save_as: idle,
            close: enabled && tab.is_some(),
            undo: enabled && tab.is_some_and(|tab| tab.document.borrow().can_undo()),
            redo: enabled && tab.is_some_and(|tab| tab.document.borrow().can_redo()),
        }
    }
}

fn action_button(ui: &mut egui::Ui, label: &str, key: Key, shift: bool, enabled: bool) -> bool {
    let modifiers = if shift {
        Modifiers::COMMAND | Modifiers::SHIFT
    } else {
        Modifiers::COMMAND
    };
    let shortcut = ui
        .ctx()
        .format_shortcut(&egui::KeyboardShortcut::new(modifiers, key));
    if ui
        .add_enabled(enabled, egui::Button::new(label).shortcut_text(shortcut))
        .clicked()
    {
        ui.close();
        true
    } else {
        false
    }
}

impl EditorApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        workspace: Workspace,
        diagnostics: Vec<String>,
    ) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.selection.bg_fill = Color32::from_rgb(27, 94, 122);
        style.visuals.panel_fill = Color32::from_rgb(20, 25, 33);
        style.visuals.extreme_bg_color = Color32::from_rgb(15, 19, 26);
        style.spacing.item_spacing = egui::vec2(10.0, 9.0);
        cc.egui_ctx.set_style(style);
        let font_status = crate::fonts::install(&cc.egui_ctx);
        Self {
            workspace,
            diagnostics,
            allow_close: false,
            font_status,
            dialog_ime_active: false,
        }
    }

    fn core_action(&mut self, redo: bool) {
        let Some(tab) = self.workspace.active_mut() else {
            return;
        };
        let result = if redo {
            tab.document.borrow_mut().redo()
        } else {
            tab.document.borrow_mut().undo()
        };
        match result {
            Ok(_) => {
                tab.editor.request_focus();
                tab.status = if redo { "Redo" } else { "Undo" }.into();
            }
            Err(error) => tab.status = error.to_string(),
        }
    }

    fn show_save_dialog(&mut self, ctx: &egui::Context, id: DocumentId, ime_input: bool) {
        let Some(tab) = self.workspace.tab_mut(id) else {
            return;
        };
        let Some(dialog) = tab.save.dialog.as_ref() else {
            return;
        };
        let mut accept = false;
        let mut cancel = false;
        let busy = tab.save.busy();
        let focus_path = std::mem::take(&mut tab.save.focus_path);
        let title = tab.title();
        let modal = egui::Modal::new(egui::Id::new(("save_destination", id.0))).show(ctx, |ui| {
            ui.label(&title);
            match dialog {
                SaveDialog::Path => {
                    ui.heading("Save As");
                    ui.label("Destination path");
                    let input = ui.add(
                        egui::TextEdit::singleline(&mut tab.save.path_input).desired_width(420.0),
                    );
                    if focus_path {
                        input.request_focus();
                    }
                    accept = !tab.save.path_input.is_empty()
                        && !ime_input
                        && input.lost_focus()
                        && ui.input(|input| input.key_pressed(Key::Enter));
                }
                SaveDialog::Overwrite { path, .. } => {
                    ui.heading("Replace existing file?");
                    ui.label(path.display().to_string());
                }
            }
            ui.label(&tab.status);
            ui.horizontal(|ui| {
                let label = match dialog {
                    SaveDialog::Path => "Continue",
                    SaveDialog::Overwrite { .. } => "Replace",
                };
                accept |= ui
                    .add_enabled(
                        !busy && !tab.save.path_input.is_empty(),
                        egui::Button::new(label),
                    )
                    .clicked();
                cancel = ui.add_enabled(!busy, egui::Button::new("Cancel")).clicked();
            });
        });
        if busy {
            return;
        }
        if cancel || modal.should_close() {
            tab.save.dialog = None;
            tab.status = "Save As canceled".into();
            self.workspace.cancel_close(id);
        } else if accept {
            match tab.save.dialog.take().expect("shown dialog") {
                SaveDialog::Path => {
                    tab.save.dialog = Some(SaveDialog::Path);
                    match tab.save.inspect(PathBuf::from(&tab.save.path_input)) {
                        Ok(()) => tab.status = "Checking destination…".into(),
                        Err(error) => tab.save_failed(error),
                    }
                }
                SaveDialog::Overwrite { path, target } => tab.start_write(path, Arc::new(target)),
            }
        }
    }

    fn show_open_dialog(&mut self, ctx: &egui::Context, ime_input: bool) {
        let open = &mut self.workspace.open;
        let focus = std::mem::take(&mut open.focus_path);
        let mut accept = false;
        let mut cancel = false;
        let modal = egui::Modal::new(egui::Id::new("open_file")).show(ctx, |ui| {
            ui.heading("Open UTF-8 file");
            ui.label("File path (UTF-8 without BOM, up to 4 MiB)");
            let input =
                ui.add(egui::TextEdit::singleline(&mut open.path_input).desired_width(460.0));
            if focus {
                input.request_focus();
            }
            accept = !ime_input
                && !open.path_input.is_empty()
                && input.lost_focus()
                && ui.input(|input| input.key_pressed(Key::Enter));
            ui.label(&open.status);
            ui.horizontal(|ui| {
                accept |= ui
                    .add_enabled(!open.path_input.is_empty(), egui::Button::new("Open"))
                    .clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if cancel || modal.should_close() {
            self.workspace.open.cancel();
            if let Some(tab) = self.workspace.active_mut() {
                tab.editor.request_focus();
            }
        } else if accept {
            let path = PathBuf::from(&self.workspace.open.path_input);
            if let Err(error) = self.workspace.start_open(path) {
                self.workspace.open.status = format!("Open failed: {error}");
            }
            if let Some(tab) = self.workspace.active_mut() {
                tab.editor.request_focus();
            }
        }
    }

    fn show_close_dialog(&mut self, ctx: &egui::Context, id: DocumentId) {
        let tab = self.workspace.tab(id).expect("modal owner");
        let mut save = false;
        let mut discard = false;
        let mut cancel = false;
        let modal = egui::Modal::new(egui::Id::new(("close_document", id.0))).show(ctx, |ui| {
            ui.heading("Save changes before closing?");
            ui.label(tab.title());
            ui.label(&tab.status);
            ui.horizontal(|ui| {
                save = ui.button("Save").clicked();
                discard = ui.button("Discard").clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if cancel || modal.should_close() {
            self.workspace.cancel_close(id);
        } else if discard {
            self.workspace.remove(id);
        } else if save {
            let tab = self.workspace.tab_mut(id).expect("modal owner");
            tab.confirm_close = false;
            tab.save.close_after_write = true;
            tab.start_save(false);
        }
    }
}

impl EditorApp {
    fn update_frame(&mut self, ctx: &egui::Context) {
        let ime_events = ctx.input(|input| {
            input
                .events
                .iter()
                .any(|e| matches!(e, egui::Event::Ime(_)))
        });
        let dialog_ime_input = self.dialog_ime_active || ime_events;
        if self.workspace.open.dialog || self.workspace.modal_owner().is_some() {
            ctx.input(|input| {
                for event in &input.events {
                    match event {
                        egui::Event::Ime(egui::ImeEvent::Enabled | egui::ImeEvent::Preedit(_)) => {
                            self.dialog_ime_active = true
                        }
                        egui::Event::Ime(egui::ImeEvent::Commit(_) | egui::ImeEvent::Disabled)
                        | egui::Event::WindowFocused(false) => self.dialog_ime_active = false,
                        _ => {}
                    }
                }
            });
        } else {
            self.dialog_ime_active = false;
        }
        let composing = self
            .workspace
            .active()
            .is_some_and(|tab| tab.editor.blocks_shortcuts(ctx));
        let modal_open = self.workspace.modal_owner().is_some() || self.workspace.open.dialog;
        let available = Actions::available(&self.workspace, modal_open || composing || ime_events);
        let can_command = available.close;
        let mut action = None;
        let mut undo = false;
        let mut redo = false;
        let mut save = false;
        let mut save_as = false;
        let mut new_tab = false;
        let mut open = false;
        let mut close_tab = None;
        let mut switch_tab = None;
        let editor_focused = self.workspace.active_id().is_some_and(|id| {
            ctx.memory(|memory| memory.has_focus(egui::Id::new(("document_editor", id.0))))
        });
        if !modal_open && !composing && !ime_events && !self.workspace.closing_all {
            ctx.input_mut(|input| {
                new_tab = input
                    .consume_shortcut(&egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::N))
                    && available.new;
                open = input
                    .consume_shortcut(&egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::O))
                    && available.open;
                if can_command {
                    if input
                        .consume_shortcut(&egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::W))
                    {
                        close_tab = self.workspace.active_id();
                    }
                    let previous = input.consume_shortcut(&egui::KeyboardShortcut::new(
                        Modifiers::CTRL | Modifiers::SHIFT,
                        Key::Tab,
                    ));
                    let next = !previous
                        && input.consume_shortcut(&egui::KeyboardShortcut::new(
                            Modifiers::CTRL,
                            Key::Tab,
                        ));
                    if previous || next {
                        let len = self.workspace.tabs.len();
                        let index = self
                            .workspace
                            .tabs
                            .iter()
                            .position(|tab| Some(tab.id()) == self.workspace.active_id())
                            .expect("active tab");
                        switch_tab = Some(
                            self.workspace.tabs[(index + if previous { len - 1 } else { 1 }) % len]
                                .id(),
                        );
                    }
                    {
                        let save_as_pressed = input.consume_shortcut(&egui::KeyboardShortcut::new(
                            Modifiers::COMMAND | Modifiers::SHIFT,
                            Key::S,
                        ));
                        save_as = save_as_pressed && available.save_as;
                        save = !save_as_pressed
                            && input.consume_shortcut(&egui::KeyboardShortcut::new(
                                Modifiers::COMMAND,
                                Key::S,
                            ))
                            && available.save_as;
                    }
                    // The focused editor preserves ordered text/undo events.
                    // Route shortcuts here only when a menu/toolbar owns focus.
                    if !editor_focused {
                        let redo_pressed = input.consume_shortcut(&egui::KeyboardShortcut::new(
                            Modifiers::COMMAND | Modifiers::SHIFT,
                            Key::Z,
                        )) || input.consume_shortcut(
                            &egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::Y),
                        );
                        redo = redo_pressed && available.redo;
                        undo = !redo_pressed
                            && input.consume_shortcut(&egui::KeyboardShortcut::new(
                                Modifiers::COMMAND,
                                Key::Z,
                            ))
                            && available.undo;
                    }
                    if input.consume_shortcut(&egui::KeyboardShortcut::new(
                        Modifiers::COMMAND | Modifiers::SHIFT,
                        Key::U,
                    )) {
                        action = Some(crate::UPPERCASE.to_owned());
                    }
                }
            });
        }

        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("RustEditor")
                        .size(23.0)
                        .strong()
                        .color(Color32::from_rgb(119, 217, 234)),
                );
                ui.label(
                    egui::RichText::new("TECHNICAL PREVIEW")
                        .small()
                        .color(Color32::GRAY),
                );
            });
            egui::ScrollArea::horizontal()
                .id_salt("tabs")
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for tab in &self.workspace.tabs {
                            ui.push_id(tab.id().0, |ui| {
                                if ui
                                    .add_enabled(
                                        !modal_open,
                                        egui::Button::selectable(
                                            self.workspace.active_id() == Some(tab.id()),
                                            tab.title(),
                                        ),
                                    )
                                    .on_hover_text(tab.save.association.as_ref().map_or_else(
                                        || tab.status.clone(),
                                        |file| format!("{}\n{}", file.path.display(), tab.status),
                                    ))
                                    .clicked()
                                {
                                    switch_tab = Some(tab.id());
                                }
                                if tab.save.busy() {
                                    ui.spinner();
                                }
                                if ui
                                    .add_enabled(available.close, egui::Button::new("×"))
                                    .on_hover_text("Close tab (Ctrl+W)")
                                    .clicked()
                                {
                                    close_tab = Some(tab.id());
                                }
                            });
                        }
                    });
                });
            ui.separator();
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    new_tab |= action_button(ui, "New", Key::N, false, available.new);
                    open |= action_button(ui, "Open…", Key::O, false, available.open);
                    ui.separator();
                    save |= action_button(ui, "Save", Key::S, false, available.save);
                    save_as |= action_button(ui, "Save As…", Key::S, true, available.save_as);
                    ui.separator();
                    if action_button(ui, "Close tab", Key::W, false, available.close) {
                        close_tab = self.workspace.active_id();
                    }
                });
                ui.menu_button("Edit", |ui| {
                    undo |= action_button(ui, "Undo", Key::Z, false, available.undo);
                    redo |= action_button(ui, "Redo", Key::Y, false, available.redo);
                });
                new_tab |= ui
                    .add_enabled(available.new, egui::Button::new("New"))
                    .clicked();
                open |= ui
                    .add_enabled(available.open, egui::Button::new("Open"))
                    .on_hover_text("Open file (Ctrl+O)")
                    .clicked();
                save |= ui
                    .add_enabled(available.save, egui::Button::new("Save"))
                    .clicked();
                save_as |= ui
                    .add_enabled(available.save_as, egui::Button::new("Save As"))
                    .clicked();
                if self.workspace.active().is_some() {
                    undo |= ui
                        .add_enabled(available.undo, egui::Button::new("Undo"))
                        .clicked();
                    redo |= ui
                        .add_enabled(available.redo, egui::Button::new("Redo"))
                        .clicked();
                }
                ui.menu_button("Plugins", |ui| {
                    for command in self.workspace.host.commands() {
                        if ui
                            .add_enabled(can_command, egui::Button::new(&command.title))
                            .clicked()
                        {
                            action = Some(command.id);
                            ui.close();
                        }
                    }
                    if self.workspace.host.commands().is_empty() {
                        ui.label("No plugin commands loaded");
                    }
                });
                if self.workspace.active().is_some_and(|tab| tab.save.busy()) {
                    ui.spinner();
                    ui.label("Save in progress");
                }
            });
            ui.add_space(3.0);
        });

        if undo {
            self.core_action(false);
        }
        if redo {
            self.core_action(true);
        }
        egui::SidePanel::right("plugin_panel").default_width(230.0).resizable(true).show(ctx, |ui| {
            ui.add_space(8.0);
            ui.heading("Extensions");
            ui.label(format!("{} native plugin(s) loaded", self.workspace.host.loaded_count()));
            ui.label("Updates apply after restart.");
            ui.separator();
            for command in self.workspace.host.commands() {
                ui.label(egui::RichText::new(&command.title).strong());
                ui.label(egui::RichText::new(&command.plugin_id).small());
                if ui.add_enabled(can_command, egui::Button::new("Run command")).clicked() { action = Some(command.id); }
            }
            if self.workspace.host.commands().is_empty() { ui.label("Run cargo xtask stage, then restart to enable text-tools."); }
            ui.separator();
            ui.label("Ctrl+N  New tab\nCtrl+O  Open file\nCtrl+S / Ctrl+Shift+S  Save / Save As\nCtrl+W  Close tab\nCtrl+Tab / Ctrl+Shift+Tab  Switch tab\nCtrl+Shift+U  Uppercase\nCtrl+Z  Undo\nCtrl+Y / Ctrl+Shift+Z  Redo\nCtrl+C / X / V  Clipboard");
            ui.separator();
            ui.label(egui::RichText::new(&self.font_status).small());
            for error in &self.diagnostics { ui.colored_label(Color32::LIGHT_RED, error); }
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            if !self.workspace.open.status.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(&self.workspace.open.status);
                    if self.workspace.open.busy() {
                        ui.spinner();
                        if ui.button("Cancel Open").clicked() { self.workspace.open.cancel(); }
                    }
                });
            }
            if let Some(tab) = self.workspace.active_mut() {
                let doc = tab.document.borrow();
                let status = tab.editor.status(&doc);
                let path = tab.save.association.as_ref().map_or_else(|| tab.title(), |file| file.path.display().to_string());
                ui.add(egui::Label::new(path).truncate()).on_hover_text(tab.save.association.as_ref().map_or_else(|| "No file path".into(), |file| file.path.display().to_string()));
                ui.add(egui::Label::new(&tab.status).wrap());
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!(
                        "{} · UTF-8 · {} · {} bytes · {} lines · Ln {}, Col {} · selection {} graphemes / {} bytes · revision {}",
                        if doc.is_modified() { "Modified" } else { "Unmodified" },
                        status.line_endings,
                        doc.text().len(),
                        status.lines, status.line, status.column,
                        status.selection_graphemes, status.selection_bytes, doc.revision().0,
                    )).on_hover_text("Lines split at LF (CRLF counts once). Columns and selection count Unicode graphemes; a tab counts once. None means no line ending is present. Enter inserts LF; existing bytes are preserved.");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(format!("{:.0}% scale", ctx.pixels_per_point() * 100.0));
                        if composing {
                            ui.label("IME composing");
                        }
                    });
                });
            } else {
                ui.label("No documents. Use New or Open.");
            }
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(tab) = self.workspace.active_mut() {
                let id = tab.id().0;
                ui.add_enabled_ui(!modal_open, |ui| {
                    egui::ScrollArea::both()
                        .id_salt(("document_scroll", id))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let output = tab.editor.show(
                                ui,
                                egui::Id::new(("document_editor", id)),
                                &mut tab.document.borrow_mut(),
                            );
                            if output.response.changed() {
                                tab.status = "Edited".into();
                            }
                            if let Some(error) = output.error {
                                tab.status = error;
                            }
                        });
                });
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("New: Ctrl+N · Open: Ctrl+O");
                });
            }
        });

        if let Some(command) = action {
            let result = self.workspace.host.invoke(&command);
            if let Some(tab) = self.workspace.active_mut() {
                tab.status = match result {
                    Ok(()) => "Plugin transaction applied".into(),
                    Err(error) => error.to_string(),
                };
                tab.editor.request_focus();
            }
            ctx.request_repaint();
        }
        if (save_as || (save && Actions::available(&self.workspace, false).save))
            && let Some(tab) = self.workspace.active_mut()
        {
            tab.start_save(save_as);
        }
        // Consume this frame's edits before a completed save can close its tab.
        // Otherwise queued text could be delivered to the newly activated tab.
        self.workspace.poll_saves(ctx);
        // Finish this frame against its original document before changing focus.
        if let Some(id) = switch_tab
            && self.workspace.active_id() != Some(id)
        {
            cancel_native_ime(ctx);
            self.workspace.activate(id);
            ctx.request_repaint();
        }
        if new_tab {
            cancel_native_ime(ctx);
            if let Err(error) = self.workspace.new_tab()
                && let Some(tab) = self.workspace.active_mut()
            {
                tab.status = error;
            }
            ctx.request_repaint();
        }
        if open {
            self.workspace.open.choose_path();
            ctx.request_repaint();
        }
        if let Some(id) = close_tab {
            self.workspace.request_close(id);
        }
        if ctx.input(|input| input.viewport().close_requested()) && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if !composing && !modal_open {
                self.workspace.open.cancel();
                self.workspace.closing_all = true;
            }
        }
        self.workspace.advance_close_all();
        if self.workspace.closing_all && self.workspace.tabs.is_empty() {
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if let Some(id) = self.workspace.modal_owner() {
            if self.workspace.tab(id).is_some_and(|tab| tab.confirm_close) {
                self.show_close_dialog(ctx, id);
            } else {
                self.show_save_dialog(ctx, id, dialog_ime_input);
            }
        } else if self.workspace.open.dialog {
            self.show_open_dialog(ctx, dialog_ime_input);
        }
        self.workspace.poll_open(ctx, composing || ime_events);
        if self.workspace.closing_all {
            ctx.request_repaint();
        }
        if self.workspace.tabs.iter().any(|tab| tab.save.busy()) {
            ctx.request_repaint_after(std::time::Duration::from_millis(25));
        }
    }
}

impl eframe::App for EditorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_frame(ctx);
    }
}

#[cfg(test)]
mod tests;
