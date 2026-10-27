# 架构规划

日期：2026-10-28。本文是拟实施方案；依赖版本及工具链在 M0 实测后锁定。

## 产品范围

面向日常文本、日志与代码编辑：多标签、行号、状态栏、查找替换、语法高亮、编码转换、会话恢复、插件扩展。优先保证中文输入、文件保存可靠性、启动速度及编辑流畅度。

首个可用版本包含 UTF-8/UTF-8 BOM、UTF-16 LE/BE、GBK 的明确打开与保存策略，CRLF/LF，撤销重做、单文件搜索与替换、基础高亮及一个动态库插件。自动探测只能给出候选，不能保证判断准确。完整 IDE、调试器、插件市场、二进制文件编辑、Notepad++ DLL 兼容均不属于首版。

## workspace 分工

以下是目标目录，按里程碑逐步创建。Cargo workspace 解决构建和代码边界；只有标为 cdylib 的扩展通过运行时加载，其余 crate 编译链接进入宿主。

| 路径 / package | 产物 | 职责 |
|---|---|---|
| apps/desktop / rnp-desktop | bin | 入口、窗口、菜单、多标签、快捷键与组装 |
| crates/editor-core / rnp-editor-core | lib | 文档、编辑事务、选择区、撤销重做、版本号；不依赖 GUI |
| crates/editor-view / rnp-editor-view | lib | 文本排版、可见区绘制、命中测试、滚动和 IME 适配 |
| crates/file-io / rnp-file-io | lib | 编码、换行、文件监测、安全保存与恢复 |
| crates/commands / rnp-commands | lib | 命令 ID、路由、快捷键与启用条件 |
| crates/search / rnp-search | lib | 字面量/正则、单文件及目录检索、替换事务 |
| crates/syntax / rnp-syntax | lib | 语法解析、增量高亮和折叠信息 |
| crates/settings / rnp-settings | lib | 配置、主题、会话与配置迁移 |
| crates/plugin-api / rnp-plugin-api | lib | 最小 C ABI 类型与常量；不依赖 GUI/编辑核心 |
| crates/plugin-sdk / rnp-plugin-sdk | lib | Rust 插件开发包装、边界校验及 panic 转换 |
| crates/plugin-host / rnp-plugin-host | lib | 发现、加载、握手、命令注册和生命周期 |
| plugins/text-tools / rnp-text-tools | cdylib | 大小写转换/行排序等示例扩展 |
| xtask / xtask | bin | 打包、插件复制、验收与发布脚本 |

依赖方向：desktop → view/core/file-io/commands/search/syntax/settings/plugin-host；view → core；file-io/search/syntax → core；plugin-host → plugin-api + commands/core；plugin-sdk → plugin-api；插件 → plugin-sdk。core 不反向依赖 UI 或插件。禁止环形依赖。

不要把所有 crate 都编译成动态库：文档、布局及输入等高频内部调用继续使用 Rust 类型；扩展边界才采用窄 C ABI。内置搜索提供基础能力，插件承载可替换的扩展命令，避免启动必须依赖第三方 DLL。

## 技术选择与验证门槛

| 领域 | 拟采用 | 原因与待验证内容 |
|---|---|---|
| GUI | egui + eframe | Rust 桌面原型效率较高；M0 必须验证 Windows 中文 IME、缩放、剪贴板与快捷键 |
| 文本模型 | ropey | 分块文本模型候选；配合基准验证大文件编辑、索引与快照成本 |
| 编辑视图 | 先用 TextEdit 做交互试验，再做 core 驱动的虚拟化视图 | 普通字符串控件不能直接证明大文件能力；禁止长期保留双份可编辑模型 |
| 高亮 | tree-sitter，按语言增量引入 | 独立语法任务，结果携带文档版本；过期结果丢弃 |
| 搜索 | regex + 自定义跨块扫描层 | rope 分块可能切断匹配；整文档扁平化仅允许受限小文件路径 |
| 编码 | encoding_rs + BOM/UTF-16 专用处理 | GBK 等传统编码与 UTF-16 路径分别验证；不可编码字符必须让用户选择处理方式 |
| 加载器 | libloading | 调用系统装载器；加载安全责任仍在宿主和插件 |
| 配置 | serde + TOML | 人可读配置，显式 schema 版本及迁移 |

GUI 不是最终冻结选择：M0 若输入法/无障碍/布局不能达标，先重新评估 Slint、原生控件或其他工具包，再进入编辑功能开发。不得把依赖版本写成未经构建验证的“最新版”；实施时提交 Cargo.lock，记录 MSRV、许可证及目标平台。

## 核心数据与线程

- DocumentId 与递增 Revision 标识文档及版本。文本、选择区、编码和换行策略分离；修改标记关联最后保存版本。
- 编辑以事务提交，撤销重做保存增量；明确字节、Unicode 标量、grapheme 与屏幕列的区别。ABI 使用 UTF-8 字节偏移，仅允许字符边界；界面移动光标按 grapheme，显示宽度由排版决定。
- UI 线程独占可变编辑状态；后台执行读盘、解析、检索、保存和恢复快照。任务可取消，队列有界，结果必须校验文档版本。
- 保存捕获不可变快照；后台完成后只把对应版本记为已保存，后续编辑仍显示未保存。
- 小文件完整载入；大文件采用独立受限模式，限制高亮、长行布局与撤销预算。GBK/UTF-16 解码及编码映射也计入内存预算。

## 文件可靠性

默认保存同目录临时文件，写入并 flush，按平台能力同步并替换；保留原文件权限，处理只读、磁盘满、文件占用、外部修改冲突。Windows 采用经过验证的替换策略；不同文件系统不承诺统一断电原子性。失败不得清除未保存标记，不能无提示覆盖外部变化。

恢复文件与原文件分离；恢复内容带版本与校验信息；启动询问恢复或丢弃。关闭修改标签必须选择保存/放弃/取消。默认关闭不受信任目录的插件自动加载。

## 开发与发布

M0 创建虚拟 workspace，resolver = "2"，统一 edition、版本和 lint；edition 与 MSRV 经工具链验证后固定。核心单元测试不启动 GUI；宿主有真实动态库集成测试。先配置 main push/PR 的 fmt、clippy 和测试，再建立 Windows/Linux/macOS 构建矩阵；不预先添加自动发布。

首发提供 Windows 便携包，程序目录与用户数据目录分离；插件按平台/架构/ABI 单独打包，不跨平台复用。发布前审查字体、图标、语法包和依赖许可证；项目许可证单独决定，不直接套用 Notepad++ 资源或商标。

## 依据

- Cargo workspace：https://doc.rust-lang.org/cargo/reference/workspaces.html
- Rust linkage：https://doc.rust-lang.org/reference/linkage.html
- Rust FFI：https://doc.rust-lang.org/nomicon/ffi.html
- egui TextEdit：https://docs.rs/egui/latest/egui/widgets/text_edit/struct.TextEdit.html
- libloading：https://docs.rs/libloading/latest/libloading/struct.Library.html

这些文档支持工具能力与边界说明；本文中的选型、目录与里程碑属于本项目设计建议。
