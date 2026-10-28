//! Native text-command ABI v1.0. All pointers are call-scoped unless documented otherwise.
use std::{ffi::c_void, mem::size_of};

pub const MAGIC: u32 = 0x5253_4544;
pub const ABI_MAJOR: u16 = 1;
pub const ABI_MINOR: u16 = 0;
pub const TARGET: &str = env!("RUSTEDITOR_TARGET");
pub const ENTRY_SYMBOL: &[u8] = b"rusteditor_plugin_entry_v1\0";
pub const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;
pub const CAP_READ_SELECTION: u64 = 1;
pub const CAP_REPLACE_TEXT: u64 = 2;
pub const CAP_COMMANDS: u64 = 4;
pub const CAP_ALL: u64 = CAP_READ_SELECTION | CAP_REPLACE_TEXT | CAP_COMMANDS;

pub type Status = i32;
pub const OK: Status = 0;
pub const INVALID_ARGUMENT: Status = 1;
pub const UNSUPPORTED: Status = 2;
pub const STALE_REVISION: Status = 3;
pub const BUFFER_TOO_SMALL: Status = 4;
pub const INTERNAL_ERROR: Status = 5;
pub const CONFLICT: Status = 6;
pub const WRONG_THREAD: Status = 7;
pub const DISABLED: Status = 8;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct AbiHeader {
    pub magic: u32,
    pub abi_major: u16,
    pub abi_minor: u16,
    /// Available bytes, including this header. New minor versions only append fields.
    pub struct_size: u32,
    pub reserved: u32,
}

impl AbiHeader {
    pub const fn new<T>() -> Self {
        Self {
            magic: MAGIC,
            abi_major: ABI_MAJOR,
            abi_minor: ABI_MINOR,
            struct_size: size_of::<T>() as u32,
            reserved: 0,
        }
    }

    pub fn validate<T>(&self) -> Result<(), Status> {
        if self.magic != MAGIC || self.abi_major != ABI_MAJOR {
            return Err(UNSUPPORTED);
        }
        if self.reserved != 0 || (self.struct_size as usize) < size_of::<T>() {
            return Err(INVALID_ARGUMENT);
        }
        Ok(())
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Utf8Slice {
    pub ptr: *const u8,
    pub len: u64,
}

impl Utf8Slice {
    pub fn borrowed(value: &str) -> Self {
        Self {
            ptr: value.as_ptr(),
            len: value.len() as u64,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OutputBuffer {
    pub ptr: *mut u8,
    pub capacity: u64,
}

impl OutputBuffer {
    pub const fn query() -> Self {
        Self {
            ptr: std::ptr::null_mut(),
            capacity: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SelectionSnapshot {
    pub header: AbiHeader,
    pub document_id: u64,
    pub revision: u64,
    pub start: u64,
    pub end: u64,
    pub required_length: u64,
}

impl Default for SelectionSnapshot {
    fn default() -> Self {
        Self {
            header: AbiHeader::new::<Self>(),
            document_id: 0,
            revision: 0,
            start: 0,
            end: 0,
            required_length: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ReplaceRequest {
    pub header: AbiHeader,
    pub document_id: u64,
    pub expected_revision: u64,
    pub start: u64,
    pub end: u64,
    pub replacement: Utf8Slice,
}

pub type LogFn = unsafe extern "C" fn(*mut c_void, Utf8Slice) -> Status;
pub type RegisterFn = unsafe extern "C" fn(*mut c_void, Utf8Slice, Utf8Slice) -> Status;
pub type UnregisterFn = unsafe extern "C" fn(*mut c_void, Utf8Slice) -> Status;
pub type SnapshotFn =
    unsafe extern "C" fn(*mut c_void, u64, OutputBuffer, *mut SelectionSnapshot) -> Status;
pub type ReplaceFn = unsafe extern "C" fn(*mut c_void, *const ReplaceRequest) -> Status;
pub type ErrorFn = unsafe extern "C" fn(*mut c_void, OutputBuffer, *mut u64) -> Status;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct HostApiV1 {
    pub header: AbiHeader,
    pub capabilities: u64,
    pub context: *mut c_void,
    pub log: Option<LogFn>,
    pub register_command: Option<RegisterFn>,
    pub unregister_command: Option<UnregisterFn>,
    pub read_selection: Option<SnapshotFn>,
    pub replace_text: Option<ReplaceFn>,
    pub error_details: Option<ErrorFn>,
}

pub type InitializeFn = unsafe extern "C" fn(*mut c_void) -> Status;
pub type InvokeFn = unsafe extern "C" fn(*mut c_void, Utf8Slice) -> Status;
pub type ShutdownFn = unsafe extern "C" fn(*mut c_void) -> Status;
pub type DestroyFn = unsafe extern "C" fn(*mut c_void) -> Status;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PluginApiV1 {
    pub header: AbiHeader,
    pub required_capabilities: u64,
    /// Plugin-owned UTF-8 metadata, valid until destroy returns.
    pub id: Utf8Slice,
    pub version: Utf8Slice,
    pub instance: *mut c_void,
    pub initialize: Option<InitializeFn>,
    pub invoke_command: Option<InvokeFn>,
    pub shutdown: Option<ShutdownFn>,
    pub destroy: Option<DestroyFn>,
}

impl Default for PluginApiV1 {
    fn default() -> Self {
        Self {
            header: AbiHeader::new::<Self>(),
            required_capabilities: 0,
            id: Utf8Slice::borrowed(""),
            version: Utf8Slice::borrowed(""),
            instance: std::ptr::null_mut(),
            initialize: None,
            invoke_command: None,
            shutdown: None,
            destroy: None,
        }
    }
}

pub type PluginEntry = unsafe extern "C" fn(*const HostApiV1, *mut PluginApiV1) -> Status;

/// Read a header before reading a complete ABI table.
///
/// # Safety
/// `ptr` must point to an aligned, readable header, followed by at least the
/// advertised number of readable bytes. `T` must be a C-layout table with that
/// header as its first field. Pointers must remain valid for this call.
pub unsafe fn read_table<T: Copy>(ptr: *const T) -> Result<T, Status> {
    if ptr.is_null() || !(ptr as usize).is_multiple_of(std::mem::align_of::<T>()) {
        return Err(INVALID_ARGUMENT);
    }
    // SAFETY: The caller guarantees a readable header at the start of the table.
    let header = unsafe { ptr.cast::<AbiHeader>().read() };
    header.validate::<T>()?;
    // SAFETY: The validated size and caller's allocation contract cover T.
    Ok(unsafe { ptr.read() })
}

/// # Safety
/// A nonempty slice must identify initialized, readable bytes valid for `'a`.
/// A native plugin can violate this contract; validation is not memory isolation.
pub unsafe fn borrowed_utf8<'a>(value: Utf8Slice) -> Result<&'a str, Status> {
    let len = usize::try_from(value.len).map_err(|_| INVALID_ARGUMENT)?;
    if len > MAX_TEXT_BYTES {
        return Err(INVALID_ARGUMENT);
    }
    if len == 0 {
        return Ok("");
    }
    if value.ptr.is_null() {
        return Err(INVALID_ARGUMENT);
    }
    // SAFETY: Valid allocation and lifetime are the caller's FFI precondition.
    let bytes = unsafe { std::slice::from_raw_parts(value.ptr, len) };
    std::str::from_utf8(bytes).map_err(|_| INVALID_ARGUMENT)
}

/// Copy into a caller-owned buffer without sharing allocator ownership.
///
/// # Safety
/// A nonempty buffer must be writable for its capacity and must not overlap
/// `bytes`. All memory must remain valid for the duration of this call.
pub unsafe fn copy_output(bytes: &[u8], output: OutputBuffer) -> Status {
    let Ok(capacity) = usize::try_from(output.capacity) else {
        return INVALID_ARGUMENT;
    };
    if capacity > MAX_TEXT_BYTES || (capacity != 0 && output.ptr.is_null()) {
        return INVALID_ARGUMENT;
    }
    if capacity < bytes.len() {
        return BUFFER_TOO_SMALL;
    }
    if !bytes.is_empty() {
        // SAFETY: Caller guarantees valid disjoint storage; capacity was checked.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), output.ptr, bytes.len()) };
    }
    OK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_prefix_and_append_only_negotiation() {
        assert_eq!(size_of::<AbiHeader>(), 16);
        assert_eq!(std::mem::offset_of!(HostApiV1, header), 0);
        assert_eq!(std::mem::offset_of!(PluginApiV1, header), 0);
        let mut header = AbiHeader::new::<HostApiV1>();
        header.abi_minor += 1;
        header.struct_size += 16;
        assert_eq!(header.validate::<HostApiV1>(), Ok(()));
        header.struct_size = 16;
        assert_eq!(header.validate::<HostApiV1>(), Err(INVALID_ARGUMENT));
        header.abi_major += 1;
        assert_eq!(header.validate::<HostApiV1>(), Err(UNSUPPORTED));
    }
}
