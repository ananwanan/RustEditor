//! Rust helpers for the C ABI. Each exported boundary catches unwindable panics.
use api::*;
pub use rusteditor_plugin_api as api;
use std::{
    ffi::c_void,
    marker::PhantomData,
    panic::{AssertUnwindSafe, catch_unwind},
    rc::Rc,
};

pub fn boundary(call: impl FnOnce() -> Result<(), Status>) -> Status {
    match catch_unwind(AssertUnwindSafe(call)) {
        Ok(Ok(())) => OK,
        Ok(Err(status)) => status,
        Err(_) => INTERNAL_ERROR,
    }
}

fn check(status: Status) -> Result<(), Status> {
    if status == OK { Ok(()) } else { Err(status) }
}

/// A host table borrowed until plugin destruction, restricted to the UI thread.
#[derive(Clone, Copy)]
pub struct Host {
    table: HostApiV1,
    _ui_thread: PhantomData<Rc<()>>,
}

pub struct Snapshot {
    pub metadata: SelectionSnapshot,
    pub text: String,
}

impl Host {
    /// # Safety
    /// The host table, its context and callbacks must stay valid until destroy
    /// completes. Construction and all subsequent calls occur on the UI thread.
    pub unsafe fn from_raw(table: *const HostApiV1, required: u64) -> Result<Self, Status> {
        // SAFETY: The entry caller supplies an ABI header followed by its table.
        let table = unsafe { read_table(table) }?;
        if required & !table.capabilities != 0 {
            return Err(UNSUPPORTED);
        }
        if table.context.is_null()
            || table.log.is_none()
            || table.register_command.is_none()
            || table.unregister_command.is_none()
            || table.read_selection.is_none()
            || table.replace_text.is_none()
            || table.error_details.is_none()
        {
            return Err(INVALID_ARGUMENT);
        }
        Ok(Self {
            table,
            _ui_thread: PhantomData,
        })
    }

    pub fn log(&self, message: &str) -> Result<(), Status> {
        // SAFETY: Validated host table and call-scoped input; Host is not Send/Sync.
        check(unsafe {
            (self.table.log.ok_or(UNSUPPORTED)?)(self.table.context, Utf8Slice::borrowed(message))
        })
    }

    pub fn register_command(&self, id: &str, title: &str) -> Result<(), Status> {
        // SAFETY: Valid table; strings are borrowed only until this call returns.
        check(unsafe {
            (self.table.register_command.ok_or(UNSUPPORTED)?)(
                self.table.context,
                Utf8Slice::borrowed(id),
                Utf8Slice::borrowed(title),
            )
        })
    }

    pub fn unregister_command(&self, id: &str) -> Result<(), Status> {
        // SAFETY: Valid table and borrowed command ID, dispatched on the UI thread.
        check(unsafe {
            (self.table.unregister_command.ok_or(UNSUPPORTED)?)(
                self.table.context,
                Utf8Slice::borrowed(id),
            )
        })
    }

    pub fn selection(&self) -> Result<Snapshot, Status> {
        let read = self.table.read_selection.ok_or(UNSUPPORTED)?;
        let mut metadata = SelectionSnapshot::default();
        // SAFETY: Writable output table and a null, zero-capacity query buffer.
        let status = unsafe { read(self.table.context, 0, OutputBuffer::query(), &mut metadata) };
        if status != BUFFER_TOO_SMALL && status != OK {
            return Err(status);
        }
        metadata.header.validate::<SelectionSnapshot>()?;
        if metadata.document_id == 0
            || metadata.revision == 0
            || metadata.start > metadata.end
            || metadata.end > MAX_TEXT_BYTES as u64
            || metadata.required_length != metadata.end - metadata.start
        {
            return Err(INVALID_ARGUMENT);
        }
        let len = usize::try_from(metadata.required_length).map_err(|_| INVALID_ARGUMENT)?;
        let mut bytes = vec![0; len];
        let mut second = SelectionSnapshot::default();
        // SAFETY: Caller-owned output allocation covers its advertised capacity.
        check(unsafe {
            read(
                self.table.context,
                metadata.revision,
                OutputBuffer {
                    ptr: bytes.as_mut_ptr(),
                    capacity: len as u64,
                },
                &mut second,
            )
        })?;
        second.header.validate::<SelectionSnapshot>()?;
        if second.document_id != metadata.document_id
            || second.revision != metadata.revision
            || second.start != metadata.start
            || second.end != metadata.end
            || second.required_length != metadata.required_length
        {
            return Err(STALE_REVISION);
        }
        Ok(Snapshot {
            metadata: second,
            text: String::from_utf8(bytes).map_err(|_| INVALID_ARGUMENT)?,
        })
    }

    pub fn replace_selection(&self, snapshot: &Snapshot, text: &str) -> Result<(), Status> {
        if text.len() > MAX_TEXT_BYTES {
            return Err(INVALID_ARGUMENT);
        }
        let request = ReplaceRequest {
            header: AbiHeader::new::<ReplaceRequest>(),
            document_id: snapshot.metadata.document_id,
            expected_revision: snapshot.metadata.revision,
            start: snapshot.metadata.start,
            end: snapshot.metadata.end,
            replacement: Utf8Slice::borrowed(text),
        };
        // SAFETY: Request and replacement remain readable for this synchronous call.
        check(unsafe {
            (self.table.replace_text.ok_or(UNSUPPORTED)?)(self.table.context, &request)
        })
    }
}

pub trait Plugin: Sized + 'static {
    const ID: &'static str;
    const VERSION: &'static str;
    const CAPABILITIES: u64 = CAP_ALL;
    fn create(host: Host) -> Result<Self, Status>;
    fn initialize(&mut self) -> Result<(), Status>;
    fn invoke(&mut self, command: &str) -> Result<(), Status>;
    fn shutdown(&mut self) -> Result<(), Status>;
}

/// # Safety
/// The input tables must obey their pointer/header contracts. The host must
/// serialize all lifecycle calls and retain its table through destruction.
pub unsafe fn entry<P: Plugin>(host: *const HostApiV1, output: *mut PluginApiV1) -> Status {
    boundary(|| {
        if output.is_null()
            || !(output as usize).is_multiple_of(std::mem::align_of::<PluginApiV1>())
        {
            return Err(INVALID_ARGUMENT);
        }
        // SAFETY: The caller guarantees a readable output-capacity header.
        let capacity = unsafe { output.cast::<AbiHeader>().read() };
        capacity.validate::<PluginApiV1>()?;
        // SAFETY: Entry's caller retains the host table until plugin destruction.
        let host = unsafe { Host::from_raw(host, P::CAPABILITIES) }?;
        let plugin = Box::new(P::create(host)?);
        let table = PluginApiV1 {
            header: AbiHeader::new::<PluginApiV1>(),
            required_capabilities: P::CAPABILITIES,
            id: Utf8Slice::borrowed(P::ID),
            version: Utf8Slice::borrowed(P::VERSION),
            instance: Box::into_raw(plugin).cast(),
            initialize: Some(initialize::<P>),
            invoke_command: Some(invoke::<P>),
            shutdown: Some(shutdown::<P>),
            destroy: Some(destroy::<P>),
        };
        // SAFETY: Capacity was checked before allocation; only the v1 prefix is written.
        unsafe { output.write(table) };
        Ok(())
    })
}

fn with_instance<P: Plugin>(
    ptr: *mut c_void,
    call: impl FnOnce(&mut P) -> Result<(), Status>,
) -> Status {
    boundary(|| {
        if ptr.is_null() || !(ptr as usize).is_multiple_of(std::mem::align_of::<P>()) {
            return Err(INVALID_ARGUMENT);
        }
        // SAFETY: The host serializes calls using the instance allocated by entry.
        call(unsafe { &mut *ptr.cast::<P>() })
    })
}

unsafe extern "C" fn initialize<P: Plugin>(ptr: *mut c_void) -> Status {
    with_instance::<P>(ptr, P::initialize)
}
unsafe extern "C" fn invoke<P: Plugin>(ptr: *mut c_void, command: Utf8Slice) -> Status {
    with_instance::<P>(ptr, |plugin| {
        // SAFETY: The host lends the command's bytes until this callback returns.
        plugin.invoke(unsafe { borrowed_utf8(command) }?)
    })
}
unsafe extern "C" fn shutdown<P: Plugin>(ptr: *mut c_void) -> Status {
    with_instance::<P>(ptr, P::shutdown)
}
unsafe extern "C" fn destroy<P: Plugin>(ptr: *mut c_void) -> Status {
    boundary(|| {
        if ptr.is_null() || !(ptr as usize).is_multiple_of(std::mem::align_of::<P>()) {
            return Err(INVALID_ARGUMENT);
        }
        // SAFETY: The host destroys exactly once, after all other callbacks finish.
        unsafe { drop(Box::from_raw(ptr.cast::<P>())) };
        Ok(())
    })
}

#[macro_export]
macro_rules! export_plugin {
    ($plugin:ty) => {
        /// # Safety
        /// Called by a v1 host providing valid, aligned ABI tables.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn rusteditor_plugin_entry_v1(
            host: *const $crate::api::HostApiV1,
            output: *mut $crate::api::PluginApiV1,
        ) -> $crate::api::Status {
            // SAFETY: The host is responsible for the entry pointer contracts.
            unsafe { $crate::entry::<$plugin>(host, output) }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catches_panics_and_validates_foreign_buffers() {
        assert_eq!(boundary(|| panic!("boundary probe")), INTERNAL_ERROR);
        assert_eq!(boundary(|| Err(STALE_REVISION)), STALE_REVISION);
        let invalid = [0xff];
        // SAFETY: Test allocations are valid for these checked call-scoped slices.
        unsafe {
            assert_eq!(
                borrowed_utf8(Utf8Slice {
                    ptr: invalid.as_ptr(),
                    len: 1
                }),
                Err(INVALID_ARGUMENT)
            );
            assert_eq!(
                borrowed_utf8(Utf8Slice {
                    ptr: std::ptr::null(),
                    len: 1
                }),
                Err(INVALID_ARGUMENT)
            );
            assert_eq!(
                borrowed_utf8(Utf8Slice {
                    ptr: std::ptr::null(),
                    len: u64::MAX
                }),
                Err(INVALID_ARGUMENT)
            );
            assert_eq!(copy_output(b"abc", OutputBuffer::query()), BUFFER_TOO_SMALL);
        }
    }

    #[test]
    fn short_table_is_rejected_before_reading_its_tail() {
        let header = AbiHeader::new::<AbiHeader>();
        // SAFETY: A real header is readable. Its short size forbids accessing the tail.
        let status = unsafe { Host::from_raw((&header as *const AbiHeader).cast(), CAP_ALL) }.err();
        assert_eq!(status, Some(INVALID_ARGUMENT));
    }
}
