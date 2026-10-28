#![forbid(unsafe_code)]
use eframe::egui::{self, Color32, Key, Modifiers};
use rusteditor_editor_view::EditorView;
use rusteditor_plugin_host::{PluginHost, SharedDocument};

pub struct EditorApp {
    document: SharedDocument,
    host: PluginHost,
    diagnostics: Vec<String>,
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
            document,
            host,
            diagnostics,
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
}

impl eframe::App for EditorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.composing = self.editor.blocks_shortcuts(ctx);
        let mut action = None;
        let mut undo = false;
        let mut redo = false;
        if !self.composing {
            ctx.input_mut(|input| {
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
                    egui::RichText::new("M0 / TECHNICAL PREVIEW")
                        .small()
                        .color(Color32::GRAY),
                );
            });
            ui.separator();
            ui.horizontal(|ui| {
                let doc = self.document.borrow();
                undo |= ui
                    .add_enabled(!self.composing && doc.can_undo(), egui::Button::new("Undo"))
                    .clicked();
                redo |= ui
                    .add_enabled(!self.composing && doc.can_redo(), egui::Button::new("Redo"))
                    .clicked();
                drop(doc);
                ui.menu_button("Plugins", |ui| {
                    for command in self.host.commands() {
                        if ui
                            .add_enabled(!self.composing, egui::Button::new(&command.title))
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
                ui.label(
                    egui::RichText::new("Scratch buffer · not saved to disk")
                        .color(Color32::from_rgb(236, 186, 114)),
                );
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
                if ui.add_enabled(!self.composing, egui::Button::new("Run command")).clicked() { action = Some(command.id); }
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
            ui.horizontal(|ui| {
                ui.label(&self.status);
                ui.separator();
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
            ui.add_enabled_ui(!self.confirm_close, |ui| {
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
        if ctx.input(|input| input.viewport().close_requested())
            && !self.allow_close
            && self.document.borrow().revision().0 > 1
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_close = true;
        }
        if self.confirm_close {
            egui::Modal::new(egui::Id::new("close_scratch")).show(ctx, |ui| {
                ui.heading("Discard scratch text?");
                ui.label("This preview does not save files. Closing discards the scratch buffer.");
                ui.horizontal(|ui| {
                    if ui.button("Keep editing").clicked() {
                        self.confirm_close = false;
                    }
                    if ui.button("Discard and close").clicked() {
                        self.allow_close = true;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
        }
    }
}
