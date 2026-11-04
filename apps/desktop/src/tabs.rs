//! UI-thread document ownership and routing. Workers never access the active tab.
use crate::save::{Completion, FileAssociation, SaveCompletion, SaveDialog, SaveSession};
use eframe::egui;
use rusteditor_editor_core::{Document, DocumentId};
use rusteditor_editor_view::EditorView;
use rusteditor_file_io::SaveTarget;
use rusteditor_plugin_host::{PluginHost, SharedDocument};
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc};

pub const MAX_TABS: usize = 32;

pub struct Tab {
    pub document: SharedDocument,
    pub editor: EditorView,
    pub save: SaveSession,
    pub status: String,
    pub confirm_close: bool,
}

impl Tab {
    pub fn new(document: SharedDocument, association: Option<FileAssociation>) -> Self {
        let id = document.borrow().id();
        Self {
            document,
            editor: EditorView::default(),
            save: SaveSession::new(id, association),
            status: "Ready".into(),
            confirm_close: false,
        }
    }

    pub fn id(&self) -> DocumentId {
        self.document.borrow().id()
    }

    pub fn title(&self) -> String {
        let name = self.save.association.as_ref().map_or_else(
            || format!("Untitled {}", self.id().0),
            |file| {
                file.path
                    .file_name()
                    .unwrap_or(file.path.as_os_str())
                    .to_string_lossy()
                    .into_owned()
            },
        );
        format!(
            "{name}{}",
            if self.document.borrow().is_modified() {
                " *"
            } else {
                ""
            }
        )
    }

    pub fn save_failed(&mut self, error: String) {
        self.save.close_after_write = false;
        self.status = format!("Save failed: {error}");
        if self.save.dialog.is_none() {
            self.editor.request_focus();
        }
    }

    pub fn start_write(&mut self, path: PathBuf, target: Arc<SaveTarget>) {
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

    pub fn start_save(&mut self, save_as: bool) {
        if self.save.busy() {
            return;
        }
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

    /// Returns true only when an explicitly requested close can safely finish.
    fn finish_save(&mut self, result: Result<Completion, String>) -> bool {
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
                // Validate identity before changing either the file or text baseline.
                if snapshot.document_id() != self.id() {
                    self.save_failed("snapshot belongs to another document".into());
                    return false;
                }
                let revision = snapshot.revision().0;
                if outcome.baseline.is_some() {
                    let result = self.document.borrow_mut().mark_snapshot_saved(snapshot);
                    if let Err(error) = result {
                        self.save_failed(error.to_string());
                        return false;
                    }
                }
                let verified = outcome.baseline.is_some();
                self.save.association = Some(FileAssociation::new(path.clone(), outcome.baseline));
                let dirty = self.document.borrow().is_modified();
                self.status = if let Some(warning) = &outcome.warning {
                    format!("Wrote {}. {warning}", path.display())
                } else if dirty {
                    format!("Saved revision {revision}. Newer edits are unsaved")
                } else {
                    format!("Saved {}", path.display())
                };
                if std::mem::take(&mut self.save.close_after_write)
                    && verified
                    && outcome.warning.is_none()
                {
                    if dirty || self.editor.is_composing() {
                        self.confirm_close = true;
                    } else {
                        return true;
                    }
                }
            }
            Err(error) => self.save_failed(error),
        }
        false
    }
}

pub struct Workspace {
    pub tabs: Vec<Tab>,
    pub host: PluginHost,
    active: Option<DocumentId>,
    next_id: u64,
    pub closing_all: bool,
    pub open: crate::open::OpenSession,
}

impl Workspace {
    pub fn new(tabs: Vec<Tab>, host: PluginHost) -> Self {
        let active = tabs.first().map(Tab::id);
        let next_id = tabs.iter().map(|tab| tab.id().0).max().unwrap_or(0) + 1;
        Self {
            tabs,
            host,
            active,
            next_id,
            closing_all: false,
            open: crate::open::OpenSession::default(),
        }
    }

    pub fn active_id(&self) -> Option<DocumentId> {
        self.active
    }
    pub fn tab(&self, id: DocumentId) -> Option<&Tab> {
        self.tabs.iter().find(|tab| tab.id() == id)
    }
    pub fn tab_mut(&mut self, id: DocumentId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|tab| tab.id() == id)
    }
    pub fn active(&self) -> Option<&Tab> {
        self.tab(self.active?)
    }
    pub fn active_mut(&mut self) -> Option<&mut Tab> {
        self.tab_mut(self.active?)
    }

    pub fn activate(&mut self, id: DocumentId) {
        if self.active == Some(id) {
            return;
        }
        let Some(document) = self.tab(id).map(|tab| tab.document.clone()) else {
            return;
        };
        if let Err(error) = self.host.set_active_document(Some(document)) {
            if let Some(tab) = self.active_mut() {
                tab.status = error.to_string();
            }
            return;
        }
        if let Some(tab) = self.active_mut() {
            tab.editor.reset_ime();
        }
        self.active = Some(id);
        let tab = self.active_mut().expect("activated tab");
        tab.editor.reset_ime();
        tab.editor.request_focus();
    }

    pub fn new_tab(&mut self) -> Result<DocumentId, String> {
        if !self.can_add_tab() {
            return Err(format!("At most {MAX_TABS} tabs can be open"));
        }
        self.add_document(String::new(), None)
    }

    pub fn can_add_tab(&self) -> bool {
        self.tabs.len() + usize::from(self.open.busy()) < MAX_TABS
    }

    fn add_document(
        &mut self,
        text: String,
        association: Option<FileAssociation>,
    ) -> Result<DocumentId, String> {
        if self.tabs.len() >= MAX_TABS {
            return Err(format!("At most {MAX_TABS} tabs can be open"));
        }
        let id = DocumentId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or("document IDs exhausted")?;
        let document = Document::new(id, text).map_err(|error| error.to_string())?;
        self.tabs
            .push(Tab::new(Rc::new(RefCell::new(document)), association));
        self.activate(id);
        Ok(id)
    }

    pub fn start_open(&mut self, path: PathBuf) -> Result<(), String> {
        if !self.can_add_tab() {
            return Err(format!(
                "At most {MAX_TABS} tabs can be open (including the pending open)"
            ));
        }
        self.open.start(path)
    }

    /// Run after frame input and save completion handling, never ahead of edits.
    pub fn poll_open(&mut self, ctx: &egui::Context, composing: bool) {
        let defer =
            composing || self.modal_owner().is_some() || self.closing_all || self.open.dialog;
        let Some(result) = self.open.poll(ctx, defer) else {
            return;
        };
        match result.and_then(|file| {
            let path = file.path.clone();
            self.add_document(
                file.text,
                Some(FileAssociation::new(file.path, Some(file.baseline))),
            )?;
            Ok(path)
        }) {
            Ok(path) => {
                self.open.status = format!("Opened {}", path.display());
                crate::view::cancel_native_ime(ctx);
            }
            Err(error) => self.open.status = format!("Open failed: {error}"),
        }
        ctx.request_repaint();
    }

    pub fn request_close(&mut self, id: DocumentId) {
        let Some(tab) = self.tab_mut(id) else {
            return;
        };
        if tab.save.busy() {
            tab.save.close_after_write = true;
            tab.status = "Waiting for save before closing…".into();
        } else if tab.save.dialog.is_none() {
            if tab.document.borrow().is_modified() {
                tab.confirm_close = true;
            } else {
                self.remove(id);
            }
        }
    }

    /// Call only for a clean document or an explicit discard, with no job running.
    pub fn remove(&mut self, id: DocumentId) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id() == id) else {
            return;
        };
        if self.tabs[index].save.busy() {
            return;
        }
        if self.active == Some(id) {
            let next = self
                .tabs
                .get(index + 1)
                .or_else(|| index.checked_sub(1).map(|i| &self.tabs[i]));
            let next_id = next.map(Tab::id);
            let document = next.map(|tab| tab.document.clone());
            if self.host.set_active_document(document).is_err() {
                return;
            }
            self.active = next_id;
            if let Some(tab) = self.active_mut() {
                tab.editor.reset_ime();
                tab.editor.request_focus();
            }
        }
        self.tabs.remove(index);
    }

    pub fn poll_saves(&mut self, ctx: &egui::Context) {
        let completed: Vec<_> = self
            .tabs
            .iter_mut()
            .filter_map(|tab| tab.save.poll(ctx))
            .collect();
        for completion in completed {
            self.complete_save(completion);
        }
    }

    pub fn complete_save(&mut self, completion: SaveCompletion) {
        let Some(tab) = self.tab_mut(completion.document_id) else {
            return;
        };
        let closing = tab.save.close_after_write;
        if tab.finish_save(completion.result) {
            self.remove(completion.document_id);
        } else if closing && !tab.save.busy() && !tab.confirm_close {
            self.closing_all = false;
        }
    }

    pub fn cancel_close(&mut self, id: DocumentId) {
        if let Some(tab) = self.tab_mut(id) {
            tab.confirm_close = false;
            tab.save.close_after_write = false;
            tab.editor.request_focus();
        }
        self.closing_all = false;
    }

    pub fn modal_owner(&self) -> Option<DocumentId> {
        self.tabs
            .iter()
            .find(|tab| tab.confirm_close || tab.save.dialog.is_some())
            .map(Tab::id)
    }

    pub fn advance_close_all(&mut self) {
        if !self.closing_all || self.modal_owner().is_some() {
            return;
        }
        while let Some(tab) = self.tabs.first() {
            let id = tab.id();
            self.request_close(id);
            if self.tab(id).is_some() {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests;
