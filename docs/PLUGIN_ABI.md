# 动态库插件协议草案 v1

状态：设计草案，M0 实现和测试后冻结。目标为 DLL（Windows）、SO（Linux）、dylib（macOS 文件后缀）的运行时加载。Rust 插件 crate 使用 `crate-type = ["cdylib"]`；这与 Rust 的 `dylib` crate 类型不同。

## 边界

Rust 默认 ABI 不提供稳定插件协议。采用 `extern "C"` 函数和 `#[repr(C)]` 数据结构；不跨边界传递 String、Vec、Result、trait object、Rust 引用或 GUI 对象。即便全用 Rust 编写，也不能依赖不同编译器生成的内部布局一致。

动态库插件在宿主进程内拥有本机权限，无法隔离越界写、死锁和进程崩溃；能力声明仅用于 API 路由和用户理解，不是安全沙箱。不受信任扩展以后采用子进程 IPC 或 WASM。panic 捕获也不能恢复访问违规或 abort。

## 入口与协商

唯一导出符号 `rnp_plugin_entry_v1`。使用固定签名：

```rust
// 示意签名，完整结构定义在 M0 创建的 plugin-api crate。
unsafe extern "C" fn(
    host: *const HostApiV1,
    out_plugin: *mut PluginApiV1,
) -> i32;
```

Rust 2024 实现导出时使用 `#[unsafe(no_mangle)]`。不把本草案片段当作已实现 SDK。

宿主初始化完整输出缓冲区并设置可写容量；输入、输出表头均包含 magic、abi_major、abi_minor、struct_size。入口签名与表头前缀固定；调用前只能验证文件和符号，调用后才能检查返回表。插件必须先检查 host 表头与 out 缓冲区容量，再写入双方可用长度。major 不符拒绝；minor 只允许尾部追加字段，按大小和 capability 检查后使用；必需能力缺失拒绝。不得对来自库的无效指针宣称可安全验证，可信原生代码是前提。

HostApiV1 至少包含：日志、命令注册/撤销、当前文档 ID/Revision、选区快照读取、带预期版本的替换请求。PluginApiV1 至少包含：插件 ID、版本、opaque instance、initialize、invoke_command、shutdown、destroy。初始化失败必须清理已经创建的资源；命令 ID 采用插件 ID 命名空间并拒绝冲突。

## 内存与错误契约

- 字符串为 UTF-8 的 pointer + uint64 length；length 转换为 usize 前检查范围，长度为零允许空指针，非零必须非空。结构内保留字段清零；布尔使用 u32，枚举使用整数并拒绝未知必需值。
- 输入借用只在调用期间有效，不得缓存。API 表由宿主稳定持有直到 destroy 返回；插件实例只由创建它的一方销毁。
- 快照读取使用调用方提供的缓冲区及 required_length；先查询大小再读取，读取带文档 Revision，变化时返回 stale_revision 并重试。不得跨分配器释放内存。
- 插件修改文档只能请求宿主事务，带 DocumentId、expected_revision 和 UTF-8 字节范围。宿主校验边界、大小及权限；过期返回冲突，禁止静默应用到新版本。
- 返回固定 i32 状态码（ok、invalid_argument、unsupported、stale_revision、buffer_too_small、internal_error）；错误细节经调用方缓冲区复制，禁止用临时字符串指针返回。
- SDK 对每个入口和宿主回调边界捕获可展开的 Rust panic 并转状态码；禁止跨 FFI unwind。不使用 panic=abort 来期待可捕获错误；OOM/abort 仍可结束整个进程。
- v1 回调在 UI 线程串行调度，不允许重入修改；后台线程不得直接调用 UI API。初版插件命令限制为短任务，慢任务协议以后增加取消和异步结果；超时只能记录，不能安全强杀进程内线程。

## 发现和生命周期

用户插件目录下每个插件独立子目录，含 plugin.toml 和目标平台库。manifest 字段：id、name、version、abi_major/minor、target、library、capabilities。规范化路径并限制在插件目录，拒绝路径逃逸和重复 ID；不扫描工作目录或 PATH。依赖库搜索策略按系统显式实现，Windows 使用受控 DLL 搜索路径。manifest 是预筛选，实际协商结果仍需一致。

加载流程：发现 → 校验 manifest/目标平台 → 用户启用 → 装载库 → 查符号 → ABI 协商 → initialize → 注册命令 → 可用。

关闭流程：停止新调用 → 撤销命令/订阅 → 等待在途调用与插件工作结束 → shutdown → destroy → 释放所有回调/实例 → 最后释放 Library。函数指针和对象均不得活得比 Library 更久。

**v1 采用启动加载、重启生效**：关闭插件只停止分发，库句柄保持至正常退出；运行期间不替换、不热卸载 DLL。若停止失败，不卸载库，记录错误并要求重启。Windows 上已加载 DLL 可能被锁定，更新放入暂存目录，下次启动前替换。未来热重载需先证明线程、TLS、回调、静态对象和后台任务全部停止，再另立协议。

## M0 必须验证的集成用例

真实构建 text-tools cdylib，由未链接插件 crate 的宿主按绝对路径加载并执行选区大写；无插件也可启动。验证缺失符号、错误 major、短结构、缺失能力、重复命令、初始化失败、缓冲不足、版本冲突、可捕获 panic，以及清理顺序。编写故意崩溃的测试插件时必须在子进程执行，不能让普通测试进程承载它。

兼容性测试采用旧 SDK 构建的插件与新宿主交叉验证；不能仅用同一次 workspace 构建就宣称 ABI 兼容。每个平台分别构建测试，发布附架构、ABI 和校验和。

参考：https://doc.rust-lang.org/reference/linkage.html 、https://doc.rust-lang.org/nomicon/ffi.html 、https://docs.rs/libloading/latest/libloading/struct.Library.html
