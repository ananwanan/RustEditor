use eframe::egui;
use rusteditor_editor_core::{DocumentId, SaveSnapshot};
use rusteditor_file_io::{DiskBaseline, SaveOutcome, SaveTarget, inspect_save_target, save_utf8};
use std::{
    path::PathBuf,
    sync::Arc,
    thread::{self, JoinHandle},
    time::Duration,
};

pub struct FileAssociation {
    pub path: PathBuf,
    pub target: Option<Arc<SaveTarget>>,
}

impl FileAssociation {
    pub fn new(path: PathBuf, baseline: Option<DiskBaseline>) -> Self {
        Self {
            path,
            target: baseline.map(|baseline| Arc::new(SaveTarget::Existing(baseline))),
        }
    }
}

pub enum SaveDialog {
    Path,
    Overwrite { path: PathBuf, target: SaveTarget },
}

pub enum Completion {
    Inspected {
        path: PathBuf,
        target: SaveTarget,
    },
    Written {
        path: PathBuf,
        snapshot: SaveSnapshot,
        outcome: SaveOutcome,
    },
}

pub struct SaveCompletion {
    pub document_id: DocumentId,
    pub result: Result<Completion, String>,
}

pub struct SaveSession {
    pub association: Option<FileAssociation>,
    pub dialog: Option<SaveDialog>,
    pub path_input: String,
    pub focus_path: bool,
    pub close_after_write: bool,
    document_id: DocumentId,
    worker: Option<JoinHandle<Result<Completion, String>>>,
}

impl SaveSession {
    pub fn new(document_id: DocumentId, association: Option<FileAssociation>) -> Self {
        Self {
            association,
            dialog: None,
            path_input: String::new(),
            focus_path: false,
            close_after_write: false,
            document_id,
            worker: None,
        }
    }

    pub fn busy(&self) -> bool {
        self.worker.is_some()
    }

    pub fn choose_path(&mut self) {
        self.path_input = self
            .association
            .as_ref()
            .map_or_else(String::new, |file| file.path.to_string_lossy().into_owned());
        self.dialog = Some(SaveDialog::Path);
        self.focus_path = true;
    }

    pub fn inspect(&mut self, path: PathBuf) -> Result<(), String> {
        self.start(move || {
            let path = std::path::absolute(path).map_err(|e| e.to_string())?;
            let target =
                inspect_save_target(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(Completion::Inspected { path, target })
        })
    }

    pub fn write(
        &mut self,
        path: PathBuf,
        target: Arc<SaveTarget>,
        snapshot: SaveSnapshot,
    ) -> Result<(), String> {
        if snapshot.document_id() != self.document_id {
            return Err("save snapshot belongs to another document".into());
        }
        self.start(move || {
            let outcome = save_utf8(&path, snapshot.text(), &target)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(Completion::Written {
                path,
                snapshot,
                outcome,
            })
        })
    }

    fn start(
        &mut self,
        job: impl FnOnce() -> Result<Completion, String> + Send + 'static,
    ) -> Result<(), String> {
        if self.busy() {
            return Err("another save operation is in progress".into());
        }
        self.worker = Some(
            thread::Builder::new()
                .name("rusteditor-save".into())
                .spawn(job)
                .map_err(|e| e.to_string())?,
        );
        Ok(())
    }

    pub fn poll(&mut self, ctx: &egui::Context) -> Option<SaveCompletion> {
        if !self.worker.as_ref()?.is_finished() {
            ctx.request_repaint_after(Duration::from_millis(25));
            return None;
        }
        Some(SaveCompletion {
            document_id: self.document_id,
            result: self
                .worker
                .take()
                .expect("finished worker")
                .join()
                .unwrap_or_else(|_| Err("save worker exited unexpectedly".into())),
        })
    }

    #[cfg(test)]
    pub(crate) fn wait(&mut self) -> SaveCompletion {
        SaveCompletion {
            document_id: self.document_id,
            result: self.worker.take().unwrap().join().unwrap(),
        }
    }

    #[cfg(test)]
    pub(crate) fn pause(&mut self, resume: std::sync::mpsc::Receiver<()>) {
        let worker = self.worker.take().expect("started worker");
        self.worker = Some(thread::spawn(move || {
            resume.recv().unwrap();
            worker.join().unwrap()
        }));
    }

    #[cfg(test)]
    pub(crate) fn wait_until_ready(&self) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !self.worker.as_ref().expect("started worker").is_finished() {
            assert!(std::time::Instant::now() < deadline, "save did not finish");
            thread::sleep(Duration::from_millis(1));
        }
    }
}

impl Drop for SaveSession {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusteditor_editor_core::{Document, DocumentId};
    use std::{fs, time::SystemTime};

    #[test]
    fn worker_saves_only_the_captured_revision_and_failure_leaves_baseline() {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rusteditor-snapshot-{}-{nonce}.txt",
            std::process::id()
        ));
        let mut doc = Document::new(DocumentId(1), "").unwrap();
        let mut session = SaveSession::new(doc.id(), None);
        doc.replace(0..0, "first").unwrap();
        let snapshot = doc.capture_save_snapshot();
        let revision = snapshot.revision();
        session
            .write(path.clone(), Arc::new(SaveTarget::New), snapshot)
            .unwrap();
        assert!(
            session
                .write(
                    path.clone(),
                    Arc::new(SaveTarget::New),
                    doc.capture_save_snapshot()
                )
                .is_err()
        );
        doc.replace(0..5, "second").unwrap();

        let Completion::Written {
            snapshot, outcome, ..
        } = session.worker.take().unwrap().join().unwrap().unwrap()
        else {
            panic!("expected a write")
        };
        assert_eq!(snapshot.document_id(), doc.id());
        assert_eq!(snapshot.revision(), revision);
        assert_eq!(fs::read(&path).unwrap(), b"first");
        assert!(outcome.baseline.is_some());
        doc.mark_snapshot_saved(snapshot).unwrap();
        assert!(doc.is_modified());
        doc.undo().unwrap();
        assert!(!doc.is_modified());
        doc.redo().unwrap();
        assert!(doc.is_modified());

        session
            .write(
                path.clone(),
                Arc::new(SaveTarget::New),
                doc.capture_save_snapshot(),
            )
            .unwrap();
        assert!(session.worker.take().unwrap().join().unwrap().is_err());
        assert_eq!(fs::read(&path).unwrap(), b"first");
        assert!(doc.is_modified());
        doc.undo().unwrap();
        assert!(!doc.is_modified());
        fs::remove_file(path).unwrap();
    }
}
