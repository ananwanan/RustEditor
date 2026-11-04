use super::*;
use std::{
    fs,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
};

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "rusteditor-tabs-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn workspace() -> Workspace {
    let document = Rc::new(RefCell::new(Document::new(DocumentId(1), "").unwrap()));
    Workspace::new(
        vec![Tab::new(document.clone(), None)],
        PluginHost::new(document),
    )
}

fn edit(workspace: &Workspace, id: DocumentId, text: &str) {
    let mut doc = workspace.tab(id).unwrap().document.borrow_mut();
    let len = doc.text().len();
    doc.replace(0..len, text).unwrap();
}

fn paused_save(workspace: &mut Workspace, id: DocumentId, path: PathBuf) -> mpsc::Sender<()> {
    let tab = workspace.tab_mut(id).unwrap();
    tab.start_write(path, Arc::new(SaveTarget::New));
    let (resume, wait) = mpsc::channel();
    tab.save.pause(wait);
    resume
}

fn finish(workspace: &mut Workspace, id: DocumentId, resume: mpsc::Sender<()>) {
    resume.send(()).unwrap();
    let completion = workspace.tab_mut(id).unwrap().save.wait();
    assert_eq!(completion.document_id, id);
    workspace.complete_save(completion);
}

#[test]
fn open_during_save_then_switch_and_close_preserves_each_owner_and_disk_baseline() {
    let files = Files::new();
    let opened_path = files.path("opened 中文.txt");
    fs::write(&opened_path, "beta 世界🙂 e\u{301}\r\n").unwrap();
    let mut w = workspace();
    let a = DocumentId(1);
    edit(&w, a, "snapshot A");
    let resume_save = paused_save(&mut w, a, files.path("a.txt"));
    w.start_open(opened_path.clone()).unwrap();
    let (resume_open, wait) = mpsc::channel();
    w.open.pause(wait);
    assert!(w.start_open(opened_path.clone()).is_err());
    let scratch = w.new_tab().unwrap();
    edit(&w, scratch, "independent scratch");
    w.activate(a);
    edit(&w, a, "newer A");
    w.request_close(a);
    resume_open.send(()).unwrap();
    w.open.wait_until_ready();
    let ctx = egui::Context::default();
    w.poll_open(&ctx, true); // Ready while composing: do not steal focus.
    assert_eq!(w.active_id(), Some(a));
    w.poll_open(&ctx, false);
    let b = w.active_id().unwrap();
    assert_ne!(b, a);
    assert_eq!(
        w.active().unwrap().document.borrow().text(),
        "beta 世界🙂 e\u{301}\r\n"
    );
    assert!(!w.active().unwrap().document.borrow().is_modified());
    assert!(
        w.active()
            .unwrap()
            .save
            .association
            .as_ref()
            .unwrap()
            .target
            .is_some()
    );
    finish(&mut w, a, resume_save);
    assert_eq!(w.active_id(), Some(b));
    assert!(w.tab(a).unwrap().confirm_close);
    assert_eq!(fs::read(files.path("a.txt")).unwrap(), b"snapshot A");
    assert_eq!(
        w.tab(scratch).unwrap().document.borrow().text(),
        "independent scratch"
    );
    w.cancel_close(a);
    edit(&w, b, "saved opened B");
    w.active_mut().unwrap().start_save(false);
    let completion = w.active_mut().unwrap().save.wait();
    w.complete_save(completion);
    assert_eq!(fs::read(opened_path).unwrap(), b"saved opened B");
    w.request_close(b);
    assert!(w.tab(b).is_none());
    assert!(w.tab(a).unwrap().document.borrow().is_modified());
}

#[test]
fn failed_canceled_and_modal_deferred_opens_never_replace_existing_tabs() {
    let files = Files::new();
    let path = files.path("read.txt");
    fs::write(&path, "incoming\n").unwrap();
    let mut w = workspace();
    edit(&w, DocumentId(1), "keep 你好");
    let selection = w.active().unwrap().document.borrow().selection();
    let ctx = egui::Context::default();
    for (name, bytes) in [
        ("bad.txt", vec![0xff]),
        ("bom.txt", b"\xef\xbb\xbftext".to_vec()),
        (
            "large.txt",
            vec![b'x'; rusteditor_editor_core::MAX_DOCUMENT_BYTES + 1],
        ),
    ] {
        let bad = files.path(name);
        fs::write(&bad, bytes).unwrap();
        w.start_open(bad).unwrap();
        w.open.wait_until_ready();
        w.poll_open(&ctx, false);
        assert!(w.open.status.starts_with("Open failed:"));
        assert!(w.open.status.contains(name));
        assert_eq!(w.tabs.len(), 1);
        assert_eq!(w.active().unwrap().document.borrow().selection(), selection);
        assert_eq!(w.active().unwrap().document.borrow().text(), "keep 你好");
        assert!(w.active().unwrap().document.borrow().can_undo());
    }
    w.start_open(path.clone()).unwrap();
    w.open.wait_until_ready();
    w.active_mut().unwrap().confirm_close = true;
    w.poll_open(&ctx, false);
    assert_eq!(w.tabs.len(), 1);
    w.cancel_close(DocumentId(1));
    w.open.cancel();
    w.poll_open(&ctx, false);
    assert_eq!(w.tabs.len(), 1);
    assert!(!w.open.busy());
    w.start_open(path).unwrap();
    w.open.wait_until_ready();
    w.poll_open(&ctx, false);
    assert_eq!(w.tabs.len(), 2);
}

#[test]
fn pending_open_reserves_the_last_tab_slot_and_can_finish_after_all_tabs_close() {
    let files = Files::new();
    let path = files.path("empty.txt");
    fs::write(&path, "").unwrap();
    let mut w = workspace();
    w.start_open(path.clone()).unwrap();
    for _ in 1..MAX_TABS - 1 {
        w.new_tab().unwrap();
    }
    assert!(!w.can_add_tab());
    assert!(w.new_tab().is_err());
    w.open.wait_until_ready();
    w.poll_open(&egui::Context::default(), false);
    assert_eq!(w.tabs.len(), MAX_TABS);
    let ids: Vec<_> = w.tabs.iter().map(Tab::id).collect();
    for id in ids {
        w.request_close(id);
    }
    assert!(w.tabs.is_empty());
    w.start_open(path).unwrap();
    w.open.wait_until_ready();
    w.poll_open(&egui::Context::default(), false);
    assert_eq!(w.tabs.len(), 1);
    assert!(w.active_id().unwrap().0 > MAX_TABS as u64);
}

#[test]
fn canceled_inflight_read_holds_its_slot_and_cannot_reopen_the_workspace() {
    let files = Files::new();
    let path = files.path("canceled.txt");
    fs::write(&path, "never apply").unwrap();
    let mut w = workspace();
    w.start_open(path.clone()).unwrap();
    let (resume, wait) = mpsc::channel();
    w.open.pause(wait);
    w.open.cancel();
    assert!(w.open.busy());
    assert!(w.start_open(path).is_err());
    w.request_close(DocumentId(1));
    assert!(w.tabs.is_empty());
    resume.send(()).unwrap();
    w.open.wait_until_ready();
    w.poll_open(&egui::Context::default(), false);
    assert!(!w.open.busy());
    assert!(w.tabs.is_empty());
    assert!(w.active_id().is_none());
}

#[test]
fn simultaneous_saves_complete_in_reverse_order_after_switch_and_keep_newer_edits() {
    let files = Files::new();
    let mut w = workspace();
    let a = DocumentId(1);
    let b = w.new_tab().unwrap();
    edit(&w, a, "first 你好🙂 e\u{301}\r\n");
    edit(&w, b, "second\n");
    let resume_a = paused_save(&mut w, a, files.path("a 中文.txt"));
    let resume_b = paused_save(&mut w, b, files.path("b.txt"));
    w.activate(a);
    edit(&w, a, "newer");
    finish(&mut w, b, resume_b);
    assert_eq!(w.active_id(), Some(a));
    assert!(!w.tab(b).unwrap().document.borrow().is_modified());
    assert!(w.tab(a).unwrap().save.association.is_none());
    finish(&mut w, a, resume_a);
    assert_eq!(
        fs::read(files.path("a 中文.txt")).unwrap(),
        "first 你好🙂 e\u{301}\r\n".as_bytes()
    );
    assert_eq!(fs::read(files.path("b.txt")).unwrap(), b"second\n");
    assert!(w.tab(a).unwrap().document.borrow().is_modified());
    w.tab(a).unwrap().document.borrow_mut().undo().unwrap();
    assert!(!w.tab(a).unwrap().document.borrow().is_modified());
    assert_eq!(w.tab(b).unwrap().document.borrow().text(), "second\n");
    // Each second save uses its own committed disk baseline.
    edit(&w, a, "saved again A");
    edit(&w, b, "saved again B");
    for id in [a, b] {
        w.tab_mut(id).unwrap().start_save(false);
        let completion = w.tab_mut(id).unwrap().save.wait();
        w.complete_save(completion);
        assert!(!w.tab(id).unwrap().document.borrow().is_modified());
    }
    assert_eq!(
        fs::read(files.path("a 中文.txt")).unwrap(),
        b"saved again A"
    );
    assert_eq!(fs::read(files.path("b.txt")).unwrap(), b"saved again B");
}

#[test]
fn closing_an_inactive_saving_tab_waits_without_closing_or_retargeting_the_active_tab() {
    let files = Files::new();
    let mut w = workspace();
    let a = DocumentId(1);
    edit(&w, a, "save then close");
    let resume = paused_save(&mut w, a, files.path("a.txt"));
    let b = w.new_tab().unwrap();
    edit(&w, b, "keep editing");
    w.request_close(a);
    w.remove(a); // Even explicit removal cannot detach a running write.
    assert!(w.tab(a).is_some());
    assert!(w.tab(a).unwrap().save.close_after_write);
    finish(&mut w, a, resume);
    assert!(w.tab(a).is_none());
    assert_eq!(w.active_id(), Some(b));
    assert_eq!(w.active().unwrap().document.borrow().text(), "keep editing");
    assert!(w.active().unwrap().document.borrow().is_modified());
    assert_eq!(fs::read(files.path("a.txt")).unwrap(), b"save then close");
    w.cancel_close(b);
    w.remove(b);
    assert!(w.active_id().is_none());
    assert!(w.host.invoke(crate::UPPERCASE).is_err());
    assert!(w.new_tab().unwrap().0 > b.0);
}

#[test]
fn close_during_save_rechecks_newer_edits_and_cancel_only_affects_its_owner() {
    let files = Files::new();
    let mut w = workspace();
    let a = DocumentId(1);
    edit(&w, a, "snapshot");
    let resume = paused_save(&mut w, a, files.path("a.txt"));
    w.request_close(a);
    edit(&w, a, "intervening edit");
    let b = w.new_tab().unwrap();
    finish(&mut w, a, resume);
    assert_eq!(w.modal_owner(), Some(a));
    assert_eq!(w.active_id(), Some(b));
    w.cancel_close(a);
    assert!(w.modal_owner().is_none());
    assert!(w.tab(a).unwrap().document.borrow().is_modified());
    assert!(!w.tab(b).unwrap().document.borrow().is_modified());
    assert_eq!(fs::read(files.path("a.txt")).unwrap(), b"snapshot");
}

#[test]
fn failed_save_during_window_close_keeps_tabs_and_aborts_the_close_sequence() {
    let files = Files::new();
    let path = files.path("exists.txt");
    fs::write(&path, "original").unwrap();
    let mut w = workspace();
    let a = DocumentId(1);
    edit(&w, a, "unsaved");
    let resume = paused_save(&mut w, a, path.clone()); // No-clobber must fail.
    let b = w.new_tab().unwrap();
    w.closing_all = true;
    w.advance_close_all();
    finish(&mut w, a, resume);
    assert!(!w.closing_all);
    assert!(w.tab(a).unwrap().status.starts_with("Save failed:"));
    assert!(w.tab(a).unwrap().document.borrow().is_modified());
    assert!(w.tab(a).unwrap().save.association.is_none());
    assert!(w.tab(b).is_some());
    assert_eq!(fs::read(path).unwrap(), b"original");
}

#[test]
fn save_as_inspection_and_completion_follow_the_owner_after_switching() {
    let files = Files::new();
    let mut w = workspace();
    let a = DocumentId(1);
    edit(&w, a, "owner A");
    w.tab_mut(a)
        .unwrap()
        .save
        .inspect(files.path("new.txt"))
        .unwrap();
    let b = w.new_tab().unwrap();
    let inspected = w.tab_mut(a).unwrap().save.wait();
    w.complete_save(inspected);
    let written = w.tab_mut(a).unwrap().save.wait();
    w.complete_save(written);
    assert_eq!(w.active_id(), Some(b));
    assert_eq!(
        w.tab(a).unwrap().save.association.as_ref().unwrap().path,
        files.path("new.txt")
    );
    assert!(w.tab(b).unwrap().save.association.is_none());
    assert_eq!(fs::read(files.path("new.txt")).unwrap(), b"owner A");
}

#[test]
fn window_close_cancel_preserves_other_tabs_and_independent_histories() {
    let mut w = workspace();
    let a = DocumentId(1);
    let b = w.new_tab().unwrap();
    edit(&w, a, "你好");
    edit(&w, b, "🙂");
    w.closing_all = true;
    w.advance_close_all();
    assert_eq!(w.modal_owner(), Some(a));
    w.cancel_close(a);
    w.advance_close_all();
    assert_eq!(w.tabs.len(), 2);
    w.tab(a).unwrap().document.borrow_mut().undo().unwrap();
    assert_eq!(w.tab(a).unwrap().document.borrow().text(), "");
    assert_eq!(w.tab(b).unwrap().document.borrow().text(), "🙂");
    assert!(w.tab(b).unwrap().document.borrow().is_modified());
    w.tab(a).unwrap().document.borrow_mut().redo().unwrap();
    assert_eq!(w.tab(a).unwrap().document.borrow().text(), "你好");
}

#[test]
fn external_change_failure_keeps_the_owning_disk_and_saved_baselines() {
    let files = Files::new();
    let mut w = workspace();
    let a = DocumentId(1);
    let b = w.new_tab().unwrap();
    for (id, name) in [(a, "a.txt"), (b, "b.txt")] {
        edit(&w, id, name);
        let resume = paused_save(&mut w, id, files.path(name));
        finish(&mut w, id, resume);
    }
    let target = w
        .tab(a)
        .unwrap()
        .save
        .association
        .as_ref()
        .unwrap()
        .target
        .as_ref()
        .unwrap()
        .clone();
    edit(&w, a, "new A");
    fs::write(files.path("a.txt"), "external A").unwrap();
    w.tab_mut(a).unwrap().start_save(false);
    w.activate(b);
    let completion = w.tab_mut(a).unwrap().save.wait();
    w.complete_save(completion);
    let tab = w.tab(a).unwrap();
    assert!(tab.status.starts_with("Save failed:"));
    assert!(Arc::ptr_eq(
        &target,
        tab.save
            .association
            .as_ref()
            .unwrap()
            .target
            .as_ref()
            .unwrap()
    ));
    assert!(tab.document.borrow().is_modified());
    tab.document.borrow_mut().undo().unwrap();
    assert!(!tab.document.borrow().is_modified());
    assert_eq!(w.active_id(), Some(b));
    assert!(!w.tab(b).unwrap().document.borrow().is_modified());
    assert_eq!(fs::read(files.path("a.txt")).unwrap(), b"external A");
    assert_eq!(fs::read(files.path("b.txt")).unwrap(), b"b.txt");
}

#[test]
fn foreign_snapshots_are_rejected_and_tab_ids_are_never_reused() {
    let files = Files::new();
    let mut w = workspace();
    let a = DocumentId(1);
    let b = w.new_tab().unwrap();
    let snapshot = w.tab(a).unwrap().document.borrow().capture_save_snapshot();
    assert!(
        w.tab_mut(b)
            .unwrap()
            .save
            .write(files.path("wrong.txt"), Arc::new(SaveTarget::New), snapshot)
            .is_err()
    );
    w.remove(a);
    w.complete_save(SaveCompletion {
        document_id: a,
        result: Err("late closed-tab result".into()),
    });
    assert_eq!(w.tab(b).unwrap().status, "Ready");
    let c = w.new_tab().unwrap();
    assert!(c.0 > b.0);
    assert!(w.tab(a).is_none());
    for _ in w.tabs.len()..MAX_TABS {
        w.new_tab().unwrap();
    }
    assert!(w.new_tab().is_err());
}
