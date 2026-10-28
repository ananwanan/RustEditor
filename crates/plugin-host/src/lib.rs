//! Startup-only loading of explicitly trusted, manifest-described native plugins.
mod callbacks;
mod manifest;

use libloading::Library;
use rusteditor_editor_core::Document;
use rusteditor_plugin_api::*;
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    path::Path,
    rc::Rc,
    thread::ThreadId,
};

pub type SharedDocument = Rc<RefCell<Document>>;
type Registry = Rc<RefCell<BTreeMap<String, Command>>>;
type Logs = Rc<RefCell<Vec<String>>>;

#[derive(Clone, Debug)]
pub struct Command {
    pub id: String,
    pub title: String,
    pub plugin_id: String,
}

#[derive(Debug)]
pub struct PluginError(pub String);
impl fmt::Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for PluginError {}
impl From<std::io::Error> for PluginError {
    fn from(error: std::io::Error) -> Self {
        Self(error.to_string())
    }
}

#[derive(Default, Debug)]
pub struct LoadReport {
    pub loaded: Vec<String>,
    pub errors: Vec<PluginError>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Negotiating,
    Initializing,
    Ready,
    Invoking,
    Disabled,
    Stopping,
}

struct Context {
    owner: ThreadId,
    id: String,
    capabilities: u64,
    document: SharedDocument,
    commands: Registry,
    logs: Logs,
    last_error: RefCell<String>,
    phase: Cell<Phase>,
    registration_failed: Cell<bool>,
    mutating: Cell<bool>,
}

impl Context {
    fn record(&self, message: &str) {
        if let Ok(mut logs) = self.logs.try_borrow_mut() {
            if logs.len() == 256 {
                logs.remove(0);
            }
            logs.push(format!("{}: {message}", self.id));
        }
    }
    fn remove_commands(&self) {
        self.commands
            .borrow_mut()
            .retain(|_, command| command.plugin_id != self.id);
    }
}

struct LoadedPlugin {
    library: Option<Library>,
    host_api: Option<Box<HostApiV1>>,
    context: Option<Box<Context>>,
    api: PluginApiV1,
    entry_called: bool,
    negotiated: bool,
    enabled: bool,
}

impl LoadedPlugin {
    fn context(&self) -> &Context {
        self.context.as_deref().expect("live plugin context")
    }

    fn retain_for_restart(&mut self) {
        self.context().phase.set(Phase::Disabled);
        self.context().remove_commands();
        self.context().record(
            "cleanup not established; native library and callbacks retained until process exit",
        );
        // Unknown native state may retain callbacks. Leak the entire ownership
        // chain, not just the library, so no retained pointer becomes dangling.
        if let Some(context) = self.context.take() {
            std::mem::forget(context);
        }
        if let Some(table) = self.host_api.take() {
            std::mem::forget(table);
        }
        if let Some(library) = self.library.take() {
            std::mem::forget(library);
        }
    }
}

impl Drop for LoadedPlugin {
    fn drop(&mut self) {
        if !self.entry_called {
            return;
        }
        if !self.negotiated {
            self.retain_for_restart();
            return;
        }
        self.context().phase.set(Phase::Stopping);
        self.context().remove_commands();
        // SAFETY: Negotiated callbacks/instance remain valid while library and
        // host context are retained. Dispatch is synchronous and has stopped.
        let status = unsafe { (self.api.shutdown.expect("validated shutdown"))(self.api.instance) };
        if status != OK {
            self.retain_for_restart();
            return;
        }
        // SAFETY: Shutdown completed and no calls remain; creator destroys once.
        let status = unsafe { (self.api.destroy.expect("validated destroy"))(self.api.instance) };
        if status != OK {
            self.retain_for_restart();
        }
        // Only now may Rust drop the library, then the host table and context.
    }
}

/// Not Send/Sync: UI-thread ownership is enforced by Rc and callback checks.
pub struct PluginHost {
    document: SharedDocument,
    plugins: Vec<LoadedPlugin>,
    commands: Registry,
    logs: Logs,
    loaded_once: bool,
}

impl PluginHost {
    pub fn new(document: SharedDocument) -> Self {
        Self {
            document,
            plugins: Vec::new(),
            commands: Rc::default(),
            logs: Rc::default(),
            loaded_once: false,
        }
    }

    pub fn commands(&self) -> Vec<Command> {
        self.commands.borrow().values().cloned().collect()
    }
    pub fn logs(&self) -> Vec<String> {
        self.logs.borrow().clone()
    }
    pub fn loaded_count(&self) -> usize {
        self.plugins.len()
    }

    /// Load once, during startup. The caller explicitly trusts this directory's
    /// native code and dependency libraries to run with host-process privileges.
    ///
    /// # Safety
    /// Every enabled library must honor the native ABI, memory, threading and
    /// lifecycle contracts. A manifest does not make arbitrary code safe.
    pub unsafe fn load_directory(&mut self, root: &Path) -> Result<LoadReport, PluginError> {
        if self.loaded_once {
            return Err(PluginError(
                "plugins load only at startup; restart to update".into(),
            ));
        }
        self.loaded_once = true;
        let root = root.canonicalize()?;
        let mut directories = fs::read_dir(&root)?
            .map(|entry| entry.map(|e| e.path()))
            .collect::<Result<Vec<_>, _>>()?;
        directories.sort();
        let mut report = LoadReport::default();
        let mut candidates = Vec::new();
        for directory in directories {
            if !directory.is_dir() {
                continue;
            }
            match manifest::read(&root, &directory) {
                Ok(candidate) => candidates.push(candidate),
                Err(error) => report.errors.push(error),
            }
        }
        let mut counts = BTreeMap::new();
        for candidate in &candidates {
            *counts.entry(candidate.manifest.id.clone()).or_insert(0) += 1;
        }
        let mut rejected = BTreeSet::new();
        for candidate in candidates {
            if counts[&candidate.manifest.id] != 1 {
                if rejected.insert(candidate.manifest.id.clone()) {
                    report.errors.push(PluginError(format!(
                        "duplicate plugin ID: {}",
                        candidate.manifest.id
                    )));
                }
                continue;
            }
            let id = candidate.manifest.id.clone();
            // SAFETY: Same explicit native-code trust as this public method.
            match unsafe { self.load(candidate) } {
                Ok(plugin) => {
                    self.plugins.push(plugin);
                    report.loaded.push(id);
                }
                Err(error) => report.errors.push(PluginError(format!("{id}: {error}"))),
            }
        }
        Ok(report)
    }

    unsafe fn load(&self, candidate: manifest::Candidate) -> Result<LoadedPlugin, PluginError> {
        // SAFETY: Canonical confined path; caller trusts the native code.
        let library =
            unsafe { open_library(&candidate.path) }.map_err(|e| PluginError(e.to_string()))?;
        // SAFETY: Only the fixed v1 symbol is interpreted using its documented ABI.
        let entry = unsafe {
            *library
                .get::<PluginEntry>(ENTRY_SYMBOL)
                .map_err(|e| PluginError(e.to_string()))?
        };
        let mut context = Box::new(Context {
            owner: std::thread::current().id(),
            id: candidate.manifest.id.clone(),
            capabilities: candidate.capabilities,
            document: self.document.clone(),
            commands: self.commands.clone(),
            logs: self.logs.clone(),
            last_error: RefCell::default(),
            phase: Cell::new(Phase::Negotiating),
            registration_failed: Cell::new(false),
            mutating: Cell::new(false),
        });
        let host_api = Box::new(callbacks::table((&mut *context as *mut Context).cast()));
        let mut plugin = LoadedPlugin {
            library: Some(library),
            host_api: Some(host_api),
            context: Some(context),
            api: PluginApiV1::default(),
            entry_called: true,
            negotiated: false,
            enabled: false,
        };
        // SAFETY: Stable host table and initialized output with writable capacity.
        let status = unsafe {
            entry(
                &**plugin.host_api.as_ref().expect("host table"),
                &mut plugin.api,
            )
        };
        if status != OK {
            return Err(PluginError(format!("entry returned status {status}")));
        }
        let table = &plugin.api;
        table
            .header
            .validate::<PluginApiV1>()
            .map_err(|s| PluginError(format!("invalid plugin table: {s}")))?;
        if table.header.abi_minor > ABI_MINOR
            || table.header.struct_size as usize > std::mem::size_of::<PluginApiV1>()
            || table.required_capabilities & !CAP_ALL != 0
            || table.required_capabilities != candidate.capabilities
        {
            return Err(PluginError(
                "unsupported version, capacity or capabilities".into(),
            ));
        }
        // SAFETY: Trusted native plugin lends metadata until its destruction.
        let id = unsafe { borrowed_utf8(table.id) }
            .map_err(|_| PluginError("invalid plugin ID".into()))?;
        // SAFETY: Same metadata lifetime contract as the plugin ID.
        let version = unsafe { borrowed_utf8(table.version) }
            .map_err(|_| PluginError("invalid plugin version".into()))?;
        if id != candidate.manifest.id
            || version != candidate.manifest.version
            || table.instance.is_null()
            || table.initialize.is_none()
            || table.invoke_command.is_none()
            || table.shutdown.is_none()
            || table.destroy.is_none()
        {
            return Err(PluginError(
                "identity or lifecycle table does not match manifest".into(),
            ));
        }
        plugin.negotiated = true;
        plugin.context().phase.set(Phase::Initializing);
        // SAFETY: Full table negotiated; instance and native library are retained.
        let status =
            unsafe { (plugin.api.initialize.expect("validated initialize"))(plugin.api.instance) };
        if status != OK || plugin.context().registration_failed.get() {
            return Err(PluginError(format!(
                "initialization failed: status {status}"
            )));
        }
        plugin.context().phase.set(Phase::Ready);
        plugin.enabled = true;
        Ok(plugin)
    }

    pub fn invoke(&mut self, id: &str) -> Result<(), PluginError> {
        let command = self
            .commands
            .borrow()
            .get(id)
            .cloned()
            .ok_or_else(|| PluginError(format!("unknown command: {id}")))?;
        let plugin = self
            .plugins
            .iter_mut()
            .find(|p| p.context().id == command.plugin_id && p.enabled)
            .ok_or_else(|| PluginError("plugin is disabled".into()))?;
        if plugin.context().phase.get() != Phase::Ready {
            return Err(PluginError("reentrant dispatch rejected".into()));
        }
        plugin.context().phase.set(Phase::Invoking);
        // SAFETY: UI-thread serialized dispatch with borrowed ID and retained instance.
        let status = unsafe {
            (plugin.api.invoke_command.expect("validated invoke"))(
                plugin.api.instance,
                Utf8Slice::borrowed(id),
            )
        };
        plugin.context().phase.set(Phase::Ready);
        if status == OK {
            Ok(())
        } else {
            Err(PluginError(format!(
                "command {id} returned status {status}"
            )))
        }
    }

    /// Stops dispatch immediately but retains the library until normal exit.
    pub fn disable(&mut self, id: &str) -> bool {
        if let Some(plugin) = self.plugins.iter_mut().find(|p| p.context().id == id) {
            plugin.enabled = false;
            plugin.context().phase.set(Phase::Disabled);
            plugin.context().remove_commands();
            true
        } else {
            false
        }
    }

    /// Idempotent exit cleanup; returns logs after all shutdown/destroy callbacks.
    pub fn shutdown(&mut self) -> Vec<String> {
        self.commands.borrow_mut().clear();
        self.plugins.clear();
        self.logs()
    }
}

impl Drop for PluginHost {
    fn drop(&mut self) {
        self.shutdown();
    }
}

unsafe fn open_library(path: &Path) -> Result<Library, libloading::Error> {
    #[cfg(windows)]
    {
        // Search the DLL's own directory and System32, never CWD or PATH.
        const SEARCH_DLL_LOAD_DIR: u32 = 0x0000_0100;
        const SEARCH_SYSTEM32: u32 = 0x0000_0800;
        // SAFETY: Absolute canonical path and explicit native-code trust at caller.
        unsafe {
            libloading::os::windows::Library::load_with_flags(
                path,
                SEARCH_DLL_LOAD_DIR | SEARCH_SYSTEM32,
            )
            .map(Into::into)
        }
    }
    #[cfg(unix)]
    {
        // Resolve eagerly; plugin symbols do not enter the global symbol namespace.
        // SAFETY: Absolute canonical path and explicit native-code trust at caller.
        unsafe {
            libloading::os::unix::Library::open(
                Some(path),
                libloading::os::unix::RTLD_NOW | libloading::os::unix::RTLD_LOCAL,
            )
            .map(Into::into)
        }
    }
}
