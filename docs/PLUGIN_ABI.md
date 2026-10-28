# Dynamic Library Plugin Protocol Draft v1

Status: **implemented experimental v1.0**, pending the remaining [M0 acceptance gates](acceptance/M0.md). The canonical Rust C-layout types and status values are in [plugin-api](../crates/plugin-api/src/lib.rs); the implemented Rust wrappers are in [plugin-sdk](../crates/plugin-sdk/src/lib.rs). Linux and Windows CI real-library checks pass; macOS and older released-SDK compatibility are unverified. Rust plugins use `crate-type = ["cdylib"]`, distinct from Rust's `dylib` crate type.

## Boundaries

Rust's default ABI does not provide a stable plugin protocol. Use `extern "C"` functions and `#[repr(C)]` structures. Do not pass String, Vec, Result, trait objects, Rust references, or GUI objects across the boundary. Even when both sides are written in Rust, internal layouts must not be assumed identical across compiler versions.

Native plugins run inside the host process with native privileges. They cannot isolate out-of-bounds writes, deadlocks, or process crashes. Capability declarations support API routing and user understanding; they are not a security sandbox. Untrusted extensions should use subprocess IPC or WASM in a future design. Catching panics cannot recover access violations or aborts.

## Entry Point and Negotiation

The sole exported symbol is `rusteditor_plugin_entry_v1`, with a fixed signature:

```rust
// Implemented in plugin-api; this signature never depends on Rust's native ABI.
unsafe extern "C" fn(
    host: *const HostApiV1,
    out_plugin: *mut PluginApiV1,
) -> i32;
```

Rust 2024 implementations use `#[unsafe(no_mangle)]` for the export. `rusteditor_plugin_sdk::export_plugin!(PluginType)` emits this entry and uses the SDK's panic-converting lifecycle wrappers. The example [text-tools](../plugins/text-tools/src/lib.rs) implements the SDK Plugin trait.

The host initializes the entire output buffer and sets its writable capacity. Both input and output table headers contain magic, abi_major, abi_minor, and struct_size. The entry signature and header prefix remain fixed. Only files and symbols can be checked before invoking the entry point; returned tables are checked afterward. The plugin must check the host header and output buffer capacity before writing within the mutually supported size. Reject major-version mismatches. Minor versions may only append fields, which require size and capability checks before use. Reject missing required capabilities. Invalid pointers returned by a library cannot be made safe through validation alone; trusted native code is a prerequisite.

HostApiV1 includes at least logging, command registration/removal, current document ID/Revision, selection snapshot reads, and replacement requests with an expected revision. PluginApiV1 includes at least plugin ID, version, an opaque instance, initialize, invoke_command, shutdown, and destroy. Initialization failure must clean up resources already created. Namespace command IDs by plugin ID and reject conflicts.

### Implemented v1.0 Layout and Calls

The 16-byte `AbiHeader` prefix is `magic: u32` (`0x52534544`), `abi_major: u16` (1), `abi_minor: u16` (0), `struct_size: u32`, and `reserved: u32` (0). The SDK checks a readable prefix and capacity before accessing the full table or writing the output prefix. The host initializes the output table, rejects larger-than-capacity returns, and rejects plugins requiring a later minor version. A newer host table can append fields; the v1.0 SDK reads only its known prefix. These negotiation checks are not a substitute for a separately built older-SDK compatibility test.

| Table | Fields after header |
|---|---|
| HostApiV1 | capabilities (`u64`), opaque context, log, register_command, unregister_command, read_selection, replace_text, error_details |
| PluginApiV1 | required_capabilities (`u64`), id/version UTF-8 slices, opaque instance, initialize, invoke_command, shutdown, destroy |
| SelectionSnapshot | document_id, revision, start, end, required_length (all `u64`) |
| ReplaceRequest | document_id, expected_revision, start, end (all `u64`), replacement UTF-8 slice |

Functions are nullable C function pointers in the wire tables; every required function is checked before use. v1.0 requires all listed callbacks and a non-null instance. Plugins allocate and destroy their own instances. ID/version bytes are plugin-owned until destroy returns. All other input slices are borrowed only for one call. A `Utf8Slice` contains `*const u8` + `u64 len`; an `OutputBuffer` contains `*mut u8` + `u64 capacity`. Lengths/capacities are bounded to 4 MiB before constructing slices or copying. There are no NUL terminators or allocator transfers.

`read_selection(context, expected_revision, buffer, out_snapshot)` accepts expected_revision=0 for a current query. Initialize the output header/capacity even for a query. For a nonempty selection, a zero-capacity query returns buffer_too_small and writes the metadata/required length; an empty selection returns ok. Allocate caller-owned storage and read again with the returned revision. A revision change returns stale_revision. The SDK also compares document ID, range, revision and length between reads. Offsets are UTF-8 bytes, not Unicode scalar indices or screen columns.

`replace_text(context, request)` is allowed only during command invocation, with the declared replace-text capability and no reentrant mutation. It validates document identity, revision, range order, UTF-8 boundaries, replacement UTF-8 and final document size, then submits a core transaction. The replacement remains selected, and a Unicode expansion is one undo step. Initialization/shutdown cannot mutate documents.

Registration is allowed during initialize only, with at most 64 commands per plugin / 1024 total. IDs must have the manifest plugin ID followed by a dot, use the ASCII ID alphabet, and be unique. A duplicate registration also invalidates initialization if its status was ignored. Unregistration during initialize/shutdown is idempotent; the host removes registrations on initialization rollback, disable, or exit.

Capability bits are read-selection=1, replace-text=2, commands=4. Manifest declarations and the returned required-capabilities value must agree. Callbacks check the manifest grants. `error_details(context, buffer, out_required_length)` copies the host's most recent generic status diagnostic into caller storage; it never lends a temporary error pointer.

| Status | Value |
|---|---|
| ok | 0 |
| invalid_argument | 1 |
| unsupported | 2 |
| stale_revision | 3 |
| buffer_too_small | 4 |
| internal_error | 5 |
| conflict | 6 |
| wrong_thread | 7 |
| disabled | 8 |

## Memory and Error Contracts

- Represent strings as a UTF-8 pointer plus uint64 length. Check range before converting length to usize. A null pointer is allowed for zero length; nonzero lengths require non-null pointers. Zero reserved fields, represent booleans as u32, and use integer enums that reject unknown required values.
- Borrowed inputs are valid only for the duration of a call and must not be retained. The host keeps API tables at stable addresses until destroy returns. Only the creating side destroys a plugin instance.
- Snapshot reads use caller-provided buffers and required_length. Query the size before reading and include the document Revision. If it changes, return stale_revision and retry. Never free memory through a different allocator.
- Plugins modify documents only through host transaction requests containing DocumentId, expected_revision, and UTF-8 byte ranges. The host validates boundaries, sizes, and permissions. A stale request returns a conflict rather than silently editing a newer revision.
- Return fixed i32 status codes: ok, invalid_argument, unsupported, stale_revision, buffer_too_small, and internal_error. Copy error details into caller-provided buffers; never return pointers into temporary strings.
- Catch unwindable Rust panics at every plugin entry and host callback boundary and convert them into status codes. Never unwind across FFI. Do not expect panic=abort failures to be catchable; OOM or abort may still terminate the process.
- v1 dispatches callbacks serially on the UI thread and prohibits reentrant mutations. Background threads must not call UI APIs directly. Initial plugin commands are limited to short tasks. Cancellation and asynchronous results require a later protocol extension. Timeouts can be recorded, but an in-process thread cannot be forcibly terminated safely.

## Discovery and Lifecycle

Each plugin has a separate subdirectory containing plugin.toml and its target-platform library. In M0 the trusted root is `plugins` next to the executable, or an explicit `--plugin-dir`; installing a plugin there enables it on the next startup. `--no-plugins` bypasses discovery. M4 will provide per-plugin enablement UI. No CWD/PATH scanning or general user-data plugin auto-discovery is implemented.

Manifest fields are id, name, version, abi_major/minor, target, library, and capabilities. The host rejects unknown fields/capabilities, target mismatches, duplicate IDs, and path traversal before loading. The library must be a single filename; canonical manifest/library paths and plugin directories must remain within their respective roots, including after symlink resolution. Windows uses `LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32`; Unix uses `RTLD_NOW | RTLD_LOCAL`. Unix dependency resolution still follows the platform loader's configured search rules; only the staged example's normal system/Rust runtime dependencies have been exercised. The manifest is a preliminary filter; actual negotiation results must agree with it.

An example staged manifest (use the exact build target and matching platform filename):

```toml
id = "org.rusteditor.text-tools"
name = "Text tools"
version = "0.1.0"
abi_major = 1
abi_minor = 0
target = "x86_64-pc-windows-msvc"
library = "rusteditor_text_tools.dll"
capabilities = ["read-selection", "replace-text", "commands"]
```

Loading sequence: discover → validate manifest and target → user enables plugin → load library → resolve symbol → negotiate ABI → initialize → register commands → ready.

Shutdown sequence: reject new calls → remove commands/subscriptions → wait for in-flight calls and plugin work → shutdown → destroy → release all callbacks/instances → release Library last. Function pointers and objects must never outlive their Library.

M0 dispatch is synchronous and the host is not Send/Sync; callbacks reject the wrong thread before accessing mutable UI state. Plugins must not leave work in flight at shutdown. The instance is destroyed before the Library can unload; the host table/context remain alive through unloading. Initialization failure uses the validated shutdown/destroy path and removes partial registrations. If entry ran but returned an invalid table, cleanup cannot be trusted: retain the library, table, and context together. A non-ok shutdown/destroy likewise retains the whole chain until OS process termination. This intentionally trades leaked resources for avoiding calls into an unloaded library or dangling callbacks.

**v1 loads plugins at startup and applies updates after restart.** Disabling a plugin stops dispatch; the library handle is retained until normal exit. Do not replace or hot-unload DLLs during execution. If stopping fails, retain the library, record the failure, and require a restart. Windows may lock loaded DLLs, so stage updates and replace them before the next startup. Future hot reload requires proof that all threads, TLS, callbacks, static objects, and background tasks have stopped, followed by a separate protocol design.

## Required M0 Integration Cases

Build a real text-tools cdylib and load it by absolute path from a host that does not link the plugin crate. Execute selection-to-uppercase and verify startup without plugins. Test missing symbols, incorrect major versions, short structures, missing capabilities, duplicate commands, initialization failure, insufficient buffers, revision conflicts, catchable panics, and cleanup order. Deliberately crashing test plugins must run in subprocesses rather than the ordinary test process.

Validate compatibility by loading plugins built with an older SDK into a newer host. A single workspace build does not establish ABI compatibility. Build and test separately on each platform; publish architecture, ABI, and checksums with releases.

## References

- [Rust linkage](https://doc.rust-lang.org/reference/linkage.html)
- [Rust FFI](https://doc.rust-lang.org/nomicon/ffi.html)
- [libloading Library](https://docs.rs/libloading/latest/libloading/struct.Library.html)
