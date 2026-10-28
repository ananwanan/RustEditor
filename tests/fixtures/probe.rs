//! Test-only cdylib compiled separately for each fault mode by the integration harness.
#![allow(dead_code)]
use rusteditor_plugin_sdk::{Host, Plugin, api::*};

struct Probe {
    host: Host,
}

impl Plugin for Probe {
    const ID: &'static str = "org.rusteditor.probe";
    const VERSION: &'static str = "0.1.0";
    fn create(host: Host) -> Result<Self, Status> {
        Ok(Self { host })
    }
    fn initialize(&mut self) -> Result<(), Status> {
        self.host.log("initialize")?;
        self.host
            .register_command("org.rusteditor.probe.run", "Probe")?;
        if cfg!(fixture = "duplicate-command") {
            self.host
                .register_command("org.rusteditor.probe.run", "Duplicate")?;
        }
        if cfg!(fixture = "init-failure") {
            return Err(INTERNAL_ERROR);
        }
        Ok(())
    }
    fn invoke(&mut self, _command: &str) -> Result<(), Status> {
        if cfg!(fixture = "panic") {
            panic!("catchable plugin panic");
        }
        if cfg!(fixture = "abort") {
            std::process::abort();
        }
        let snapshot = self.host.selection()?;
        if cfg!(fixture = "stale") {
            self.host.replace_selection(&snapshot, "changed")?;
            return self.host.replace_selection(&snapshot, "must not apply");
        }
        if cfg!(fixture = "wrong-thread") {
            let address = HOST.load(std::sync::atomic::Ordering::SeqCst) as usize;
            let status = std::thread::spawn(move || {
                // SAFETY: The test host retains its immutable API table until
                // join completes. The callback must reject this thread before
                // accessing any UI-owned document/registry fields.
                unsafe {
                    let table = &*(address as *const HostApiV1);
                    let mut snapshot = SelectionSnapshot::default();
                    (table.read_selection.unwrap())(
                        table.context,
                        0,
                        OutputBuffer::query(),
                        &mut snapshot,
                    )
                }
            })
            .join()
            .map_err(|_| INTERNAL_ERROR)?;
            return Err(status);
        }
        self.host.replace_selection(&snapshot, "probe")
    }
    fn shutdown(&mut self) -> Result<(), Status> {
        self.host.log("shutdown")?;
        if cfg!(fixture = "shutdown-failure") {
            return Err(INTERNAL_ERROR);
        }
        self.host.unregister_command("org.rusteditor.probe.run")
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        let _ = self.host.log("destroy");
    }
}

static HOST: std::sync::atomic::AtomicPtr<HostApiV1> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

#[cfg(not(any(
    fixture = "missing-symbol",
    fixture = "bad-major",
    fixture = "short-table",
    fixture = "missing-capabilities"
)))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rusteditor_plugin_entry_v1(
    host: *const HostApiV1,
    output: *mut PluginApiV1,
) -> Status {
    HOST.store(host.cast_mut(), std::sync::atomic::Ordering::SeqCst);
    // SAFETY: The integration host supplies valid, stable ABI tables.
    unsafe { rusteditor_plugin_sdk::entry::<Probe>(host, output) }
}

#[cfg(any(
    fixture = "bad-major",
    fixture = "short-table",
    fixture = "missing-capabilities"
))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rusteditor_plugin_entry_v1(
    _host: *const HostApiV1,
    output: *mut PluginApiV1,
) -> Status {
    let mut table = PluginApiV1::default();
    if cfg!(fixture = "bad-major") {
        table.header.abi_major = 77;
    }
    if cfg!(fixture = "short-table") {
        table.header.struct_size = std::mem::size_of::<AbiHeader>() as u32;
    }
    if cfg!(fixture = "missing-capabilities") {
        table.required_capabilities = 1 << 63;
    }
    // SAFETY: The test host passes a full, initialized PluginApiV1 output buffer.
    unsafe { output.write(table) };
    OK
}

#[cfg(fixture = "missing-symbol")]
#[unsafe(no_mangle)]
pub extern "C" fn unrelated_symbol() {}
