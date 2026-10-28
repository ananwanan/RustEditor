use crate::{Command, Context, Phase, manifest};
use rusteditor_editor_core::{DocumentId, EditError, Revision, Selection, Transaction};
use rusteditor_plugin_api::*;
use std::{
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
};

fn callback(ptr: *mut c_void, call: impl FnOnce(&Context) -> Status) -> Status {
    catch_unwind(AssertUnwindSafe(|| {
        if ptr.is_null() || !(ptr as usize).is_multiple_of(std::mem::align_of::<Context>()) {
            return INVALID_ARGUMENT;
        }
        // SAFETY: Registered host callbacks receive their live, stable context.
        let context = unsafe { &*ptr.cast::<Context>() };
        if context.owner != std::thread::current().id() {
            return WRONG_THREAD;
        }
        let status = call(context);
        if status != OK
            && status != BUFFER_TOO_SMALL
            && let Ok(mut error) = context.last_error.try_borrow_mut()
        {
            *error = format!("{}: host operation returned status {status}", context.id);
        }
        status
    }))
    .unwrap_or(INTERNAL_ERROR)
}

pub(crate) fn table(context: *mut c_void) -> HostApiV1 {
    HostApiV1 {
        header: AbiHeader::new::<HostApiV1>(),
        capabilities: CAP_ALL,
        context,
        log: Some(log),
        register_command: Some(register),
        unregister_command: Some(unregister),
        read_selection: Some(snapshot),
        replace_text: Some(replace),
        error_details: Some(error_details),
    }
}

unsafe extern "C" fn log(ptr: *mut c_void, message: Utf8Slice) -> Status {
    callback(ptr, |context| {
        // SAFETY: Native caller lends valid bytes for this callback only.
        let Ok(message) = (unsafe { borrowed_utf8(message) }) else {
            return INVALID_ARGUMENT;
        };
        if message.len() > 4096 {
            return INVALID_ARGUMENT;
        }
        context.record(message);
        OK
    })
}

unsafe extern "C" fn register(ptr: *mut c_void, id: Utf8Slice, title: Utf8Slice) -> Status {
    callback(ptr, |context| {
        if context.phase.get() != Phase::Initializing || context.capabilities & CAP_COMMANDS == 0 {
            return DISABLED;
        }
        // SAFETY: The caller supplies call-scoped UTF-8 slices.
        let (Ok(id), Ok(title)) = (unsafe { borrowed_utf8(id) }, unsafe {
            borrowed_utf8(title)
        }) else {
            return INVALID_ARGUMENT;
        };
        if !manifest::identifier(id)
            || !id.starts_with(&format!("{}.", context.id))
            || title.trim().is_empty()
            || title.len() > 256
            || title.chars().any(char::is_control)
        {
            return INVALID_ARGUMENT;
        }
        let Ok(mut commands) = context.commands.try_borrow_mut() else {
            return CONFLICT;
        };
        if commands.contains_key(id) {
            context.registration_failed.set(true);
            return CONFLICT;
        }
        if commands.len() >= 1024
            || commands
                .values()
                .filter(|c| c.plugin_id == context.id)
                .count()
                >= 64
        {
            return INVALID_ARGUMENT;
        }
        commands.insert(
            id.into(),
            Command {
                id: id.into(),
                title: title.into(),
                plugin_id: context.id.clone(),
            },
        );
        OK
    })
}

unsafe extern "C" fn unregister(ptr: *mut c_void, id: Utf8Slice) -> Status {
    callback(ptr, |context| {
        if !matches!(context.phase.get(), Phase::Initializing | Phase::Stopping) {
            return DISABLED;
        }
        // SAFETY: The command ID is borrowed for this callback only.
        let Ok(id) = (unsafe { borrowed_utf8(id) }) else {
            return INVALID_ARGUMENT;
        };
        if !id.starts_with(&format!("{}.", context.id)) {
            return INVALID_ARGUMENT;
        }
        let Ok(mut commands) = context.commands.try_borrow_mut() else {
            return CONFLICT;
        };
        commands.remove(id);
        OK
    })
}

fn output_header<T>(ptr: *mut T) -> Result<(), Status> {
    if ptr.is_null() || !(ptr as usize).is_multiple_of(std::mem::align_of::<T>()) {
        return Err(INVALID_ARGUMENT);
    }
    // SAFETY: Callback callers supply an initialized capacity header at ptr.
    let header = unsafe { ptr.cast::<AbiHeader>().read() };
    header.validate::<T>()
}

unsafe extern "C" fn snapshot(
    ptr: *mut c_void,
    expected: u64,
    output: OutputBuffer,
    metadata: *mut SelectionSnapshot,
) -> Status {
    callback(ptr, |context| {
        if !matches!(context.phase.get(), Phase::Initializing | Phase::Invoking)
            || context.capabilities & CAP_READ_SELECTION == 0
        {
            return DISABLED;
        }
        if let Err(status) = output_header(metadata) {
            return status;
        }
        let Ok(document) = context.document.try_borrow() else {
            return CONFLICT;
        };
        if expected != 0 && expected != document.revision().0 {
            return STALE_REVISION;
        }
        let range = document.selection().range();
        let text = document.selected_text();
        let value = SelectionSnapshot {
            header: AbiHeader::new::<SelectionSnapshot>(),
            document_id: document.id().0,
            revision: document.revision().0,
            start: range.start as u64,
            end: range.end as u64,
            required_length: text.len() as u64,
        };
        // SAFETY: Caller supplies writable output metadata; header capacity checked.
        unsafe { metadata.write(value) };
        // SAFETY: Caller owns the destination buffer; source stays borrowed here.
        unsafe { copy_output(text.as_bytes(), output) }
    })
}

struct Mutation<'a>(&'a std::cell::Cell<bool>);
impl Drop for Mutation<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

unsafe extern "C" fn replace(ptr: *mut c_void, request: *const ReplaceRequest) -> Status {
    callback(ptr, |context| {
        if context.phase.get() != Phase::Invoking || context.capabilities & CAP_REPLACE_TEXT == 0 {
            return DISABLED;
        }
        if context.mutating.replace(true) {
            return CONFLICT;
        }
        let _mutation = Mutation(&context.mutating);
        // SAFETY: Caller provides a readable, header-prefixed request for this call.
        let Ok(request) = (unsafe { read_table(request) }) else {
            return INVALID_ARGUMENT;
        };
        // SAFETY: The replacement is borrowed only until this callback returns.
        let Ok(text) = (unsafe { borrowed_utf8(request.replacement) }) else {
            return INVALID_ARGUMENT;
        };
        let (Ok(start), Ok(end)) = (usize::try_from(request.start), usize::try_from(request.end))
        else {
            return INVALID_ARGUMENT;
        };
        let Some(head) = start.checked_add(text.len()) else {
            return INVALID_ARGUMENT;
        };
        let Ok(mut document) = context.document.try_borrow_mut() else {
            return CONFLICT;
        };
        match document.apply(Transaction {
            document_id: DocumentId(request.document_id),
            expected_revision: Revision(request.expected_revision),
            range: start..end,
            replacement: text.to_owned(),
            selection_after: Selection {
                anchor: start,
                head,
            },
        }) {
            Ok(_) => OK,
            Err(EditError::StaleRevision) => STALE_REVISION,
            Err(_) => INVALID_ARGUMENT,
        }
    })
}

unsafe extern "C" fn error_details(
    ptr: *mut c_void,
    output: OutputBuffer,
    required: *mut u64,
) -> Status {
    callback(ptr, |context| {
        if required.is_null() || !(required as usize).is_multiple_of(std::mem::align_of::<u64>()) {
            return INVALID_ARGUMENT;
        }
        let Ok(message) = context.last_error.try_borrow() else {
            return CONFLICT;
        };
        // SAFETY: Caller provides writable, aligned required-length storage.
        unsafe { required.write(message.len() as u64) };
        // SAFETY: Caller owns the output buffer, disjoint from host error storage.
        unsafe { copy_output(message.as_bytes(), output) }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusteditor_editor_core::{Document, DocumentId};
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    fn context() -> Box<Context> {
        let document = Rc::new(RefCell::new(Document::new(DocumentId(1), "中🙂").unwrap()));
        document
            .borrow_mut()
            .set_selection(Selection { anchor: 0, head: 7 })
            .unwrap();
        Box::new(Context {
            owner: std::thread::current().id(),
            id: "org.rusteditor.probe".into(),
            capabilities: CAP_ALL,
            document,
            commands: Rc::default(),
            logs: Rc::default(),
            last_error: RefCell::default(),
            phase: Cell::new(Phase::Invoking),
            registration_failed: Cell::new(false),
            mutating: Cell::new(false),
        })
    }

    #[test]
    fn snapshot_capacity_stale_revision_and_short_output_header() {
        let mut context = context();
        let ptr = (&mut *context as *mut Context).cast();
        let mut metadata = SelectionSnapshot::default();
        let mut small = [0xa5; 2];
        // SAFETY: All inputs are valid initialized fixture allocations, with
        // deliberately short capacities that must be rejected before copying.
        unsafe {
            assert_eq!(
                snapshot(ptr, 0, OutputBuffer::query(), &mut metadata),
                BUFFER_TOO_SMALL
            );
            assert_eq!(metadata.required_length, 7);
            assert_eq!(
                snapshot(
                    ptr,
                    1,
                    OutputBuffer {
                        ptr: small.as_mut_ptr(),
                        capacity: 2
                    },
                    &mut metadata
                ),
                BUFFER_TOO_SMALL
            );
            assert_eq!(small, [0xa5; 2]);
            assert_eq!(
                snapshot(ptr, 9, OutputBuffer::query(), &mut metadata),
                STALE_REVISION
            );
            let mut header = AbiHeader::new::<AbiHeader>();
            assert_eq!(
                snapshot(
                    ptr,
                    0,
                    OutputBuffer::query(),
                    (&mut header as *mut AbiHeader).cast()
                ),
                INVALID_ARGUMENT
            );
            assert_eq!(
                snapshot(
                    ptr,
                    0,
                    OutputBuffer {
                        ptr: std::ptr::null_mut(),
                        capacity: 10
                    },
                    &mut metadata
                ),
                INVALID_ARGUMENT
            );
        }
    }

    #[test]
    fn replacement_validates_identity_revision_utf8_boundaries_and_reentrancy() {
        let mut context = context();
        let ptr = (&mut *context as *mut Context).cast();
        let mut request = ReplaceRequest {
            header: AbiHeader::new::<ReplaceRequest>(),
            document_id: 1,
            expected_revision: 1,
            start: 1,
            end: 3,
            replacement: Utf8Slice::borrowed("x"),
        };
        // SAFETY: Request and all borrowed replacement bytes remain valid.
        unsafe {
            assert_eq!(replace(ptr, &request), INVALID_ARGUMENT);
            request.start = 0;
            request.document_id = 2;
            assert_eq!(replace(ptr, &request), INVALID_ARGUMENT);
            request.document_id = 1;
            request.expected_revision = 99;
            assert_eq!(replace(ptr, &request), STALE_REVISION);
            request.expected_revision = 1;
            let invalid = [0xff];
            request.replacement = Utf8Slice {
                ptr: invalid.as_ptr(),
                len: 1,
            };
            assert_eq!(replace(ptr, &request), INVALID_ARGUMENT);
            request.replacement = Utf8Slice::borrowed("好");
            context.mutating.set(true);
            assert_eq!(replace(ptr, &request), CONFLICT);
            context.mutating.set(false);
            assert_eq!(context.document.borrow().text(), "中🙂");
            assert_eq!(replace(ptr, &request), OK);
            assert_eq!(context.document.borrow().text(), "好🙂");
            assert_eq!(replace(ptr, &request), STALE_REVISION);
        }
    }

    #[test]
    fn callbacks_convert_unwindable_host_panics() {
        let mut context = context();
        let ptr = (&mut *context as *mut Context).cast();
        assert_eq!(
            callback(ptr, |_| panic!("host callback probe")),
            INTERNAL_ERROR
        );
    }
}
