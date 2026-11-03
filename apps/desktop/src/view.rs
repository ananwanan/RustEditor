#![forbid(unsafe_code)]
use crate::save::{Completion, FileAssociation, SaveDialog, SaveSession};
use eframe::egui::{self, Color32, Key, Modifiers};
use rusteditor_editor_view::EditorView;
use rusteditor_file_io::SaveTarget;
use rusteditor_plugin_host::{PluginHost, SharedDocument};
use std::{path::PathBuf, sync::Arc};

pub struct EditorApp {
    document: SharedDocument,
    host: PluginHost,
    diagnostics: Vec<String>,
    save: SaveSession,
    status: String,
    editor: EditorView,
    composing: bool,
    confirm_close: bool,
    allow_close: bool,
    font_status: String,
}

impl EditorApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        document: SharedDocument,
        host: PluginHost,
        diagnostics: Vec<String>,
        association: Option<FileAssociation>,
    ) -> Self {
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.selection.bg_fill = Color32::from_rgb(27, 94, 122);
        style.visuals.panel_fill = Color32::from_rgb(20, 25, 33);
        style.visuals.extreme_bg_color = Color32::from_rgb(15, 19, 26);
        style.spacing.item_spacing = egui::vec2(10.0, 9.0);
        cc.egui_ctx.set_style(style);
        let font_status = crate::fonts::install(&cc.egui_ctx);
        let mut save = SaveSession::default();
        save.association = association;
        Self {
            document,
            host,
            diagnostics,
            save,
            status: "Ready".into(),
            editor: EditorView::default(),
            composing: false,
            confirm_close: false,
            allow_close: false,
            font_status,
        }
    }

    fn core_action(&mut self, redo: bool) {
        let result = if redo {
            self.document.borrow_mut().redo()
        } else {
            self.document.borrow_mut().undo()
        };
        match result {
            Ok(changed) => {
                if changed {
                    self.editor.request_focus();
                }
                self.status = if redo { "Redo" } else { "Undo" }.into();
            }
            Err(error) => self.status = error.to_string(),
        }
    }

    fn save_failed(&mut self, error: String) {
        self.save.close_after_write = false;
        self.status = format!("Save failed: {error}");
        if self.save.dialog.is_none() {
            self.editor.request_focus();
        }
    }

    fn start_write(&mut self, path: PathBuf, target: Arc<SaveTarget>) {
        self.save.dialog = None;
        let snapshot = self.document.borrow().capture_save_snapshot();
        match self.save.write(path, target, snapshot) {
            Ok(()) => {
                self.status = "Saving…".into();
                self.editor.request_focus();
            }
            Err(error) => self.save_failed(error),
        }
    }

    fn start_save(&mut self, save_as: bool) {
        if save_as || self.save.association.is_none() {
            self.status = "Choose a save destination".into();
            self.save.choose_path();
        } else if let Some(file) = &self.save.association {
            match &file.target {
                Some(target) => self.start_write(file.path.clone(), Arc::clone(target)),
                None => self.save_failed("destination could not be verified. Use Save As".into()),
            }
        }
    }

    fn close(&mut self, ctx: &egui::Context) {
        self.allow_close = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn finish_save(&mut self, ctx: &egui::Context) {
        let Some(result) = self.save.poll(ctx) else {
            return;
        };
        match result {
            Ok(Completion::Inspected {
                path,
                target: SaveTarget::New,
            }) => {
                self.start_write(path, Arc::new(SaveTarget::New));
            }
            Ok(Completion::Inspected { path, target }) => {
                self.save.dialog = Some(SaveDialog::Overwrite { path, target });
                self.status = "Confirm the replacement destination".into();
            }
            Ok(Completion::Written {
                path,
                snapshot,
                outcome,
            }) => {
                let verified = outcome.baseline.is_some();
                let revision = snapshot.revision().0;
                self.save.association = Some(FileAssociation::new(path.clone(), outcome.baseline));
                if verified {
                    let result = self.document.borrow_mut().mark_snapshot_saved(snapshot);
                    if let Err(error) = result {
                        self.save_failed(error.to_string());
                        return;
                    }
                }
                let dirty = self.document.borrow().is_modified();
                self.status = if let Some(warning) = &outcome.warning {
                    format!("Wrote {}. {warning}", path.display())
                } else if dirty {
                    format!("Saved revision {revision}. Newer edits are unsaved")
                } else {
                    format!(
                        "Saved {}",
                        path.file_name()
                            .unwrap_or(path.as_os_str())
                            .to_string_lossy()
                    )
                };
                if std::mem::take(&mut self.save.close_after_write)
                    && outcome.warning.is_none()
                    && !self.composing
                    && !self.editor.is_composing()
                {
                    if dirty {
                        self.confirm_close = true;
                    } else {
                        self.close(ctx);
                    }
                }
            }
            Err(error) => self.save_failed(error),
        }
        ctx.request_repaint();
    }

    fn show_save_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.save.dialog.as_ref() else {
            return;
        };
        let mut accept = false;
        let mut cancel = false;
        let busy = self.save.busy();
        let focus_path = std::mem::take(&mut self.save.focus_path);
        let modal = egui::Modal::new(egui::Id::new("save_destination")).show(ctx, |ui| {
            match dialog {
                SaveDialog::Path => {
                    ui.heading("Save As");
                    ui.label("Destination path");
                    let input = ui.add(
                        egui::TextEdit::singleline(&mut self.save.path_input).desired_width(420.0),
                    );
                    if focus_path {
                        input.request_focus();
                    }
                    accept = !self.save.path_input.is_empty()
                        && input.lost_focus()
                        && ui.input(|input| input.key_pressed(Key::Enter));
                }
                SaveDialog::Overwrite { path, .. } => {
                    ui.heading("Replace existing file?");
                    ui.label(path.display().to_string());
                }
            }
            ui.label(&self.status);
            ui.horizontal(|ui| {
                let label = match dialog {
                    SaveDialog::Path => "Continue",
                    SaveDialog::Overwrite { .. } => "Replace",
                };
                accept |= ui
                    .add_enabled(
                        !busy && !self.save.path_input.is_empty(),
                        egui::Button::new(label),
                    )
                    .clicked();
                cancel = ui.add_enabled(!busy, egui::Button::new("Cancel")).clicked();
            });
        });
        if self.save.busy() {
            return;
        }
        if cancel || modal.should_close() {
            self.save.dialog = None;
            self.save.close_after_write = false;
            self.status = "Save As canceled".into();
            self.editor.request_focus();
        } else if accept {
            match self.save.dialog.take().expect("shown dialog") {
                SaveDialog::Path => {
                    self.save.dialog = Some(SaveDialog::Path);
                    match self.save.inspect(PathBuf::from(&self.save.path_input)) {
                        Ok(()) => self.status = "Checking destination…".into(),
                        Err(error) => self.save_failed(error),
                    }
                }
                SaveDialog::Overwrite { path, target } => self.start_write(path, Arc::new(target)),
            }
        }
    }
}

impl eframe::App for EditorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.composing = self.editor.blocks_shortcuts(ctx);
        let mut action = None;
        let mut undo = false;
        let mut redo = false;
        let mut save = false;
        let mut save_as = false;
        let modal_open = self.confirm_close || self.save.dialog.is_some();
        let can_command = !modal_open && !self.composing;
        let can_save = can_command && !self.save.busy();
        if can_command {
            ctx.input_mut(|input| {
                if can_save {
                    save_as = input.consume_shortcut(&egui::KeyboardShortcut::new(
                        Modifiers::COMMAND | Modifiers::SHIFT,
                        Key::S,
                    ));
                    if !save_as {
                        save = input.consume_shortcut(&egui::KeyboardShortcut::new(
                            Modifiers::COMMAND,
                            Key::S,
                        ));
                    }
                }
                if input.consume_shortcut(&egui::KeyboardShortcut::new(
                    Modifiers::COMMAND | Modifiers::SHIFT,
                    Key::U,
                )) {
                    action = Some(crate::UPPERCASE.to_owned());
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
            let filename = self
                .save
                .association
                .as_ref()
                .and_then(|file| file.path.file_name())
                .map_or_else(|| "Scratch buffer".into(), |name| name.to_string_lossy());
            let modified = if self.document.borrow().is_modified() {
                " *"
            } else {
                ""
            };
            ui.label(egui::RichText::new(format!("{filename}{modified}")).strong());
            ui.separator();
            ui.horizontal(|ui| {
                save |= ui
                    .add_enabled(can_save, egui::Button::new("Save"))
                    .clicked();
                save_as |= ui
                    .add_enabled(can_save, egui::Button::new("Save As"))
                    .clicked();
                let doc = self.document.borrow();
                undo |= ui
                    .add_enabled(can_command && doc.can_undo(), egui::Button::new("Undo"))
                    .clicked();
                redo |= ui
                    .add_enabled(can_command && doc.can_redo(), egui::Button::new("Redo"))
                    .clicked();
                drop(doc);
                ui.menu_button("Plugins", |ui| {
                    for command in self.host.commands() {
                        if ui
                            .add_enabled(can_command, egui::Button::new(&command.title))
                            .clicked()
                        {
                            action = Some(command.id);
                            ui.close();
                        }
                    }
                    if self.host.commands().is_empty() {
                        ui.label("No plugin commands loaded");
                    }
                });
                ui.separator();
                if self.save.busy() {
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
            ui.label(format!("{} native plugin(s) loaded", self.host.loaded_count()));
            ui.label("Updates apply after restart.");
            ui.separator();
            for command in self.host.commands() {
                ui.label(egui::RichText::new(&command.title).strong());
                ui.label(egui::RichText::new(&command.plugin_id).small());
                if ui.add_enabled(can_command, egui::Button::new("Run command")).clicked() { action = Some(command.id); }
            }
            if self.host.commands().is_empty() {
                ui.label("Run cargo xtask stage, then restart to enable text-tools.");
            }
            ui.separator();
            ui.label("Ctrl+Shift+U  Uppercase\nCtrl+Z  Undo\nCtrl+Y / Ctrl+Shift+Z  Redo\nCtrl+C / X / V  Clipboard");
            ui.separator();
            ui.label(egui::RichText::new(&self.font_status).small());
            for error in &self.diagnostics { ui.colored_label(Color32::LIGHT_RED, error); }
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            let doc = self.document.borrow();
            ui.add(egui::Label::new(&self.status).wrap());
            ui.horizontal(|ui| {
                ui.label(format!(
                    "UTF-8 · {} bytes · revision {} · selection {} bytes",
                    doc.text().len(),
                    doc.revision().0,
                    doc.selected_text().len()
                ));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("{:.0}% scale", ctx.pixels_per_point() * 100.0));
                    if self.composing {
                        ui.label("IME composing");
                    }
                });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_enabled_ui(!modal_open, |ui| {
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let output = self.editor.show(
                            ui,
                            egui::Id::new("scratch_editor"),
                            &mut self.document.borrow_mut(),
                        );
                        if output.response.changed() {
                            self.status = "Edited".into();
                        }
                        if let Some(error) = output.error {
                            self.status = error;
                        }
                    });
            });
        });

        if let Some(command) = action {
            match self.host.invoke(&command) {
                Ok(()) => {
                    self.status = "Plugin transaction applied".into();
                    self.editor.request_focus();
                }
                Err(error) => self.status = error.to_string(),
            }
            ctx.request_repaint();
        }
        if save || save_as {
            self.start_save(save_as);
        }
        if ctx.input(|input| input.viewport().close_requested()) && !self.allow_close {
            let dirty = self.document.borrow().is_modified();
            if self.save.busy() || self.save.dialog.is_some() || dirty || self.composing {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                if self.save.dialog.is_none() && !self.composing {
                    if self.save.busy() {
                        self.save.close_after_write = true;
                    } else {
                        self.confirm_close = dirty;
                    }
                }
            }
        }
        self.finish_save(ctx);
        if self.confirm_close {
            let mut cancel = false;
            let modal = egui::Modal::new(egui::Id::new("close_scratch")).show(ctx, |ui| {
                ui.heading("Save changes before closing?");
                ui.label(&self.status);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        self.confirm_close = false;
                        self.save.close_after_write = true;
                        self.start_save(false);
                    }
                    if ui.button("Discard").clicked() {
                        self.close(ctx);
                    }
                    cancel = ui.button("Cancel").clicked();
                });
            });
            if cancel || modal.should_close() {
                self.confirm_close = false;
                self.editor.request_focus();
            }
        }
        self.show_save_dialog(ctx);
        if self.save.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(25));
        }
    }
}
