//! One bounded, read-only job. Cancellation discards its result, never a tab.
use eframe::egui;
use rusteditor_file_io::{DiskBaseline, open_utf8_for_editing};
use std::{
    path::PathBuf,
    thread::{self, JoinHandle},
    time::Duration,
};

pub struct OpenedFile {
    pub path: PathBuf,
    pub text: String,
    pub baseline: DiskBaseline,
}

#[derive(Default)]
pub struct OpenSession {
    pub dialog: bool,
    pub path_input: String,
    pub focus_path: bool,
    pub status: String,
    worker: Option<JoinHandle<Result<OpenedFile, String>>>,
    canceled: bool,
}

impl OpenSession {
    pub fn busy(&self) -> bool {
        self.worker.is_some()
    }

    pub fn choose_path(&mut self) {
        self.dialog = true;
        self.focus_path = true;
    }

    pub fn start(&mut self, path: PathBuf) -> Result<(), String> {
        if self.busy() {
            return Err("another file is still being read".into());
        }
        let display = path.display().to_string();
        self.worker = Some(
            thread::Builder::new()
                .name("rusteditor-open".into())
                .spawn(move || {
                    let path = std::path::absolute(path).map_err(|e| e.to_string())?;
                    let (text, baseline) = open_utf8_for_editing(&path)
                        .map_err(|e| format!("{}: {e}", path.display()))?;
                    Ok(OpenedFile {
                        path,
                        text,
                        baseline,
                    })
                })
                .map_err(|e| e.to_string())?,
        );
        self.canceled = false;
        self.dialog = false;
        self.status = format!("Opening {display}…");
        Ok(())
    }

    pub fn cancel(&mut self) {
        self.dialog = false;
        self.canceled = true;
        self.status = "Open canceled".into();
    }

    /// A ready result may wait for IME/modal input to finish before changing tabs.
    pub fn poll(&mut self, ctx: &egui::Context, defer: bool) -> Option<Result<OpenedFile, String>> {
        let worker = self.worker.as_ref()?;
        if !worker.is_finished() || (defer && !self.canceled) {
            ctx.request_repaint_after(Duration::from_millis(25));
            return None;
        }
        let result = self
            .worker
            .take()
            .expect("finished reader")
            .join()
            .unwrap_or_else(|_| Err("open worker exited unexpectedly".into()));
        if self.canceled { None } else { Some(result) }
    }

    // Dropping a JoinHandle detaches this read-only worker at application exit.
    // It owns no document, UI or plugin reference and cannot publish an edit.
    // An OS read cannot be interrupted; the slot stays occupied until it ends.

    #[cfg(test)]
    pub(crate) fn pause(&mut self, resume: std::sync::mpsc::Receiver<()>) {
        let worker = self.worker.take().expect("started reader");
        self.worker = Some(thread::spawn(move || {
            resume.recv().unwrap();
            worker.join().unwrap()
        }));
    }

    #[cfg(test)]
    pub(crate) fn wait_until_ready(&self) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !self.worker.as_ref().expect("started reader").is_finished() {
            assert!(std::time::Instant::now() < deadline, "read did not finish");
            thread::sleep(Duration::from_millis(1));
        }
    }
}
