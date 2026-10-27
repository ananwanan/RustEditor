# RustNotePlusPlus

使用 Rust 制作类似 Notepad++ 的桌面文本与代码编辑器。采用 Cargo workspace 拆分功能，通过动态库提供可扩展能力。

当前阶段：**架构与开发规划，尚无可运行编辑器**。本次不预先创建空 crate，也不宣称实现任何里程碑。

- [架构、技术选型与 workspace](docs/ARCHITECTURE.md)
- [动态库插件协议](docs/PLUGIN_ABI.md)
- [开发路线与验收标准](docs/ROADMAP.md)

默认目标：Windows 10/11 x64 优先，随后适配 Linux/macOS；独立桌面程序，不依赖 WebView。独立实现类似体验，不承诺兼容 Notepad++ 的插件二进制。
