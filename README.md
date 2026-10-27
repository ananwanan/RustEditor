# RustNotePlusPlus

A Rust desktop text and code editor inspired by Notepad++. Features are separated into a Cargo workspace, with extensions provided through dynamically loaded libraries.

Current status: **architecture and development planning; no runnable editor yet**. Empty placeholder crates have not been created, and no implementation milestone is complete.

- [Architecture, technology choices, and workspace](docs/ARCHITECTURE.md)
- [Dynamic library plugin protocol](docs/PLUGIN_ABI.md)
- [Development roadmap and acceptance criteria](docs/ROADMAP.md)

Initial target: Windows 10/11 x64, followed by Linux and macOS. The application runs as a standalone desktop program without a WebView. It independently implements a similar editing experience and does not promise binary compatibility with Notepad++ plugins.

## Documentation language

All project documentation must be written and maintained in English, including README files, architecture and design documents, development plans, contributor guides, and future documentation additions.
