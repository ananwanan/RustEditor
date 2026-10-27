# Dynamic Library Plugin Protocol Draft v1

Status: design draft, to be frozen after implementation and testing in M0. Targets are runtime loading of DLLs on Windows, shared objects on Linux, and files with the dylib extension on macOS. Rust plugin crates use `crate-type = ["cdylib"]`, distinct from Rust's `dylib` crate type.

## Boundaries

Rust's default ABI does not provide a stable plugin protocol. Use `extern "C"` functions and `#[repr(C)]` structures. Do not pass String, Vec, Result, trait objects, Rust references, or GUI objects across the boundary. Even when both sides are written in Rust, internal layouts must not be assumed identical across compiler versions.

Native plugins run inside the host process with native privileges. They cannot isolate out-of-bounds writes, deadlocks, or process crashes. Capability declarations support API routing and user understanding; they are not a security sandbox. Untrusted extensions should use subprocess IPC or WASM in a future design. Catching panics cannot recover access violations or aborts.

## Entry Point and Negotiation

The sole exported symbol is `rnp_plugin_entry_v1`, with a fixed signature:

```rust
// Illustrative signature; complete structures will be defined in plugin-api in M0.
unsafe extern "C" fn(
    host: *const HostApiV1,
    out_plugin: *mut PluginApiV1,
) -> i32;
```

Rust 2024 implementations use `#[unsafe(no_mangle)]` for the export. This draft fragment is not an implemented SDK.

The host initializes the entire output buffer and sets its writable capacity. Both input and output table headers contain magic, abi_major, abi_minor, and struct_size. The entry signature and header prefix remain fixed. Only files and symbols can be checked before invoking the entry point; returned tables are checked afterward. The plugin must check the host header and output buffer capacity before writing within the mutually supported size. Reject major-version mismatches. Minor versions may only append fields, which require size and capability checks before use. Reject missing required capabilities. Invalid pointers returned by a library cannot be made safe through validation alone; trusted native code is a prerequisite.

HostApiV1 includes at least logging, command registration/removal, current document ID/Revision, selection snapshot reads, and replacement requests with an expected revision. PluginApiV1 includes at least plugin ID, version, an opaque instance, initialize, invoke_command, shutdown, and destroy. Initialization failure must clean up resources already created. Namespace command IDs by plugin ID and reject conflicts.

## Memory and Error Contracts

- Represent strings as a UTF-8 pointer plus uint64 length. Check range before converting length to usize. A null pointer is allowed for zero length; nonzero lengths require non-null pointers. Zero reserved fields, represent booleans as u32, and use integer enums that reject unknown required values.
- Borrowed inputs are valid only for the duration of a call and must not be retained. The host keeps API tables at stable addresses until destroy returns. Only the creating side destroys a plugin instance.
- Snapshot reads use caller-provided buffers and required_length. Query the size before reading and include the document Revision. If it changes, return stale_revision and retry. Never free memory through a different allocator.
- Plugins modify documents only through host transaction requests containing DocumentId, expected_revision, and UTF-8 byte ranges. The host validates boundaries, sizes, and permissions. A stale request returns a conflict rather than silently editing a newer revision.
- Return fixed i32 status codes: ok, invalid_argument, unsupported, stale_revision, buffer_too_small, and internal_error. Copy error details into caller-provided buffers; never return pointers into temporary strings.
- Catch unwindable Rust panics at every plugin entry and host callback boundary and convert them into status codes. Never unwind across FFI. Do not expect panic=abort failures to be catchable; OOM or abort may still terminate the process.
- v1 dispatches callbacks serially on the UI thread and prohibits reentrant mutations. Background threads must not call UI APIs directly. Initial plugin commands are limited to short tasks. Cancellation and asynchronous results require a later protocol extension. Timeouts can be recorded, but an in-process thread cannot be forcibly terminated safely.

## Discovery and Lifecycle

Each plugin has a separate subdirectory under the user plugin directory, containing plugin.toml and its target-platform library. Manifest fields are id, name, version, abi_major/minor, target, library, and capabilities. Normalize paths, confine them to the plugin directory, and reject path traversal and duplicate IDs. Do not scan the working directory or PATH. Implement dependency-library search explicitly for each OS, using controlled DLL search paths on Windows. The manifest is a preliminary filter; actual negotiation results must agree with it.

Loading sequence: discover → validate manifest and target → user enables plugin → load library → resolve symbol → negotiate ABI → initialize → register commands → ready.

Shutdown sequence: reject new calls → remove commands/subscriptions → wait for in-flight calls and plugin work → shutdown → destroy → release all callbacks/instances → release Library last. Function pointers and objects must never outlive their Library.

**v1 loads plugins at startup and applies updates after restart.** Disabling a plugin stops dispatch; the library handle is retained until normal exit. Do not replace or hot-unload DLLs during execution. If stopping fails, retain the library, record the failure, and require a restart. Windows may lock loaded DLLs, so stage updates and replace them before the next startup. Future hot reload requires proof that all threads, TLS, callbacks, static objects, and background tasks have stopped, followed by a separate protocol design.

## Required M0 Integration Cases

Build a real text-tools cdylib and load it by absolute path from a host that does not link the plugin crate. Execute selection-to-uppercase and verify startup without plugins. Test missing symbols, incorrect major versions, short structures, missing capabilities, duplicate commands, initialization failure, insufficient buffers, revision conflicts, catchable panics, and cleanup order. Deliberately crashing test plugins must run in subprocesses rather than the ordinary test process.

Validate compatibility by loading plugins built with an older SDK into a newer host. A single workspace build does not establish ABI compatibility. Build and test separately on each platform; publish architecture, ABI, and checksums with releases.

## References

- [Rust linkage](https://doc.rust-lang.org/reference/linkage.html)
- [Rust FFI](https://doc.rust-lang.org/nomicon/ffi.html)
- [libloading Library](https://docs.rs/libloading/latest/libloading/struct.Library.html)
