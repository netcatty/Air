# 阶段一设计文档：系统代理 / TUN 托盘开关与首页聚合

> 本文档是 `docs/roadmap.md` 阶段一的落地设计，供后续 AI 开发代理按此实现。
> 源码与文档不一致时以源码为准，并在本文档补齐差异；修改相关链路时同步更新 `.codex/*.md` 流程文档。
> 所有新增 Rust 代码必须用 UTF-8、含具体中文注释；所有平台差异必须收口在 `air-platform`，业务层禁止出现 `cfg(target_os)`。

## 0. 设计前提（贯穿全文的硬性约束）

本设计遵循两条架构纪律（详见 `AGENTS.md` 编码规范；外部方法论背景见 `docs/references.md`）：

1. **平台差异三层收口**：① 平台特有依赖在 `Cargo.toml` 用 `cfg(target_os)` 声明；② 平台行为差异通过 `air-platform` trait 抽象收口（本设计新增 `SystemProxyControl`/`TunControl`），业务层只见 trait 对象；③ 需要管理员权限的操作由独立 helper/服务进程承载（复用现有 `core_service` worker + `elevated_process`），GUI 保持普通权限，UI 回调不触发 UAC 以外的提权路径。**本设计不新建 `agent/` 子项目**——air 无远端边带需求，提权走现有本地服务路径即可。

2. **UI 生命周期 ≠ 后端生命周期**：关闭主窗口或隐藏到托盘时，mihomo 进程、系统代理设置和后台任务必须存活；后端（核心进程/系统代理）的真正 owner 是托盘与 app runtime，不是主窗口。只有显式退出才走 `stop_core_before_exit()` 并按 `SystemProxyRestorePolicy` 还原系统代理。这条直接约束本设计的：§4.5（退出还原只在退出触发，隐藏不触发）、§5（托盘菜单作为后端 owner 的控制面）、§6.3（Dashboard 只读投影 snapshot，不持有后端生命周期）。

> 第三条纪律"域 crate 零 I/O、只产 effect 枚举"是长期方向、非阶段一硬性，未纳入本设计；见 `docs/references.md` §2.1 与 `AGENTS.md` 架构纪律段。

## 1. 背景与目标

阶段一要在 Windows 主平台补齐 Clash 类管理器的基线闭环：安装后无需手动改系统设置即可完成日常使用。
本设计覆盖三块新增能力：

1. **系统代理开关** — 一键启用/禁用 Windows HTTP/HTTPS 系统代理，指向 mihomo `mixed-port`，状态回填，退出/停核按策略还原。
2. **TUN 虚拟网卡开关** — 一键启用/禁用 TUN 模式，提权核心进程走 `platform::core_service`，UI 不直接调 Win32。
3. **托盘右键菜单 + 首页聚合** — 菜单项状态与主窗口/内核/代理/TUN 实时同步；首页作为状态总入口。

### 1.1 现状锚点（实现前必须核对源码）

- `AppCommand`（`crates/air-app/src/command.rs`）当前**没有** `SetSystemProxy` / `SetTunEnabled` 任何系统代理/TUN 运行态命令。现有 TUN 流程是 UI 把 `tun.enable` 写进 YAML 后派发 `SaveConfig`（见 `crates/air-ui/src/shell/actions.rs` 的 `dispatch_saved_tun_toggle`）——这是**配置持久化**，不是**运行态激活**，二者必须分离（见 §4.4）。
- `AppSnapshot`（`crates/air-app/src/events.rs`）字段为 `{ runtime, active_profile, runtime_info, controller_addr, core_service, last_error }`，**没有** `system_proxy_enabled` / `tun_active`。
- `air-platform/src/tray.rs` 的 `TrayEvent` 只有 `{ ToggleWindow, ShowWindow, HideWindow, StartCore, StopCore, Quit }`；菜单 ID 常量 `MENU_SHOW=1001` … `MENU_QUIT=1005`，`show_tray_menu` 每次右键**静态重建**菜单，无勾选态。
- `air-platform/src/lib.rs` 已声明 `PlatformKind { Windows, Macos, Linux, Android, Unknown }`，注释预留 `platform::tun`，但当前 `pub mod` 列表里**没有** `system_proxy` 或 `tun` 模块。
- `air-app/src/services.rs::stop_core_before_exit()` 负责停 mihomo + Windows 服务兜底，是退出还原系统代理的挂载点。
- `TunConfig` / `TunConfigSettings` / `TunOptionPrivilege`（`air-config/src/tun.rs`）明确**只输出诊断，不执行权限提升，也不修改系统路由**——TUN 配置模型与 TUN 运行态激活是两件事。
- `AppRoute`（`crates/air-ui/src/routes.rs`）当前 `ALL` 数组 6 项：`Subscriptions / ProxyGroups / Connections / RulesProxy / OverrideScript / Settings`，**没有** Dashboard/首页路由。
- `AppError::Platform(PlatformError { Unsupported, OperationFailed })`（`air-error/src/lib.rs`）已能承载非 Windows 平台的降级诊断。

## 2. 总体架构

```
┌──────────────────────────── air-ui (GUI 层) ─────────────────────────────┐
│  Dashboard 首页  ·  托盘菜单  ·  状态栏  ·  设置页系统代理/TUN 开关        │
│         ↑ 只读展示 + 收集用户输入，不调 Win32，不写系统注册表             │
└──────────────────────────────────┬───────────────────────────────────────┘
                                   │ AppCommand::SetSystemProxy / SetTunEnabled
                                   │ AppSnapshot（含 system_proxy_enabled / tun_active）
┌──────────────────────────── air-app (编排层) ────────────────────────────┐
│  AppCommandRouter  ·  AppServices  ·  AppStateStore  ·  退出还原钩子     │
│         ↑ 唯一写入口；命令路由覆盖成功/失败/取消/核心未运行并脱敏         │
└──────────────────────────────────┬───────────────────────────────────────┘
                                   │ trait SystemProxyControl / TunControl
┌──────────────────────────── air-platform (平台隔离层) ───────────────────┐
│  system_proxy 模块（Windows 实现 + 非 Windows unsupported 占位）         │
│  tun 模块（运行态激活，复用 core_service 提权；与 air-config/tun.rs 分离）│
└───────────────────────────────────────────────────────────────────────────┘
```

### 2.1 模块边界与依赖方向

| 新增/改动            | crate           | 职责                                                  | 禁止                 |
| -------------------- | --------------- | ----------------------------------------------------- | -------------------- |
| `system_proxy` mod   | `air-platform`  | trait + Windows 注册表实现 + 非 Windows 占位          | 出现 `cfg` 在业务层  |
| `tun` 运行态 mod     | `air-platform`  | trait + Windows 实现（复用 `core_service` 提权路径）  | 调 Win32 在 GUI      |
| `SetSystemProxy` / `SetTunEnabled` 命令 | `air-app` | 路由 + 错误脱敏 + 状态写 snapshot            | 直接读写注册表       |
| snapshot 新字段      | `air-app`       | `system_proxy_enabled` / `tun_active` 投影            | —                    |
| 托盘菜单扩展         | `air-platform` tray + `air-ui` shell | 新 `TrayEvent` 变体 + 勾选态 | 菜单里调业务 API |
| Dashboard 首页       | `air-ui`        | 聚合 snapshot 运行态；不依赖 fake 数据               | 业务逻辑进 GUI       |

`air-app` 不依赖 `air-ui`；GUI 启动由 `air-desktop` 分流后调用。`air-config`/`air-mihomo` 不依赖 `air-storage`。

## 3. `air-platform` 平台抽象设计

### 3.1 `system_proxy` 模块

新增 `crates/air-platform/src/system_proxy.rs`，与 `tray.rs`/`window.rs` 同级，在 `lib.rs` 注册 `pub mod system_proxy;`。

```rust
// crates/air-platform/src/system_proxy.rs
use air_error::AppResult;

/// 系统代理（HTTP/HTTPS）运行态控制抽象。
/// 业务层只调本 trait，平台差异（Windows 注册表 / macOS networksetup / Linux NetworkManager）
/// 由各平台实现收口。本 trait 只表达“当前系统代理“Shebang 到 mihomo 入口的运行态，
/// 不读写用户 YAML —— 配置持久化由 air-settings/air-config 负责。
pub trait SystemProxyControl: Send + Sync {
    /// 读取当前系统代理开关与代理地址（已脱敏处理端口可读，地址不视为敏感）。
    fn current_state(&self) -> AppResult<SystemProxyState>;

    /// 启用系统代理，指向 mihomo mixed-port。`host:port` 由 app 层从运行配置注入，
    /// 平台实现不得硬编码 mihomo 地址或 secret。
    fn enable(&self, target: ProxyTarget) -> AppResult<()>;

    /// 禁用系统代理。不删除用户在启用前已有的自定义代理覆盖值，
    /// 由 `SystemProxyControl` 实现内部记录“启用前快照”用于还原（见 §4.5）。
    fn disable(&self) -> AppResult<()>;
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SystemProxyState {
    pub enabled: bool,
    /// 当前生效的代理服务器地址（如 127.0.0.1:7890）；禁用时为 None。
    pub server: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ProxyTarget {
    pub host: String, // 通常 127.0.0.1
    pub port: u16,    // 来自 mihomo mixed-port
}

/// 工厂：按编译目标返回平台实现。非 Windows 返回 unsupported 占位，
/// 为阶段二预留，避免业务层出现 cfg(target_os)。
pub fn default_system_proxy_control() -> Box<dyn SystemProxyControl> {
    #[cfg(windows)]
    {
        Box::new(WindowsSystemProxyControl::new())
    }
    #[cfg(not(windows))]
    {
        Box::new(UnsupportedSystemProxyControl)
    }
}
```

**Windows 实现要点**（`WindowsSystemProxyControl`，`#[cfg(windows)]`）：

- 代理设置落在注册表 `HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Internet Settings`：`ProxyEnable`（DWORD）+ `ProxyServer`（字符串，`host:port`）。
- 写值用 `RegSetValueExW`；写完后发 `INTERNET_OPTION_SETTINGS_CHANGED` + `INTERNET_OPTION_REFRESH` 通知让运行中进程感知。
- **替换而非叠加** `WinINET` 默认代理；不碰 WinHTTP 代理（`netsh winhttp set proxy` 需要管理员，留给 TUN/服务路径）。
- 启用前把 `ProxyServer` 旧值快照进进程内 `OnceLock<Mutex<Option<String>>>`；`disable` 时只回写 `ProxyEnable=0`，**不覆盖**用户原 `ProxyServer`（避免误删用户自定义配置）。是否回写 `ProxyServer` 由 `RestorePolicy` 决定（见 §4.5）。
- secret/认证头不经过本模块；mihomo 的 `external-controller` secret 永远不出现在系统代理链路。

**非 Windows 占位**（`UnsupportedSystemProxyControl`）：

```rust
#[cfg(not(windows))]
struct UnsupportedSystemProxyControl;

#[cfg(not(windows))]
impl SystemProxyControl for UnsupportedSystemProxyControl {
    fn current_state(&self) -> AppResult<SystemProxyState> {
        Err(PlatformError::Unsupported("当前平台尚未实现系统代理控制".into()).into())
    }
    fn enable(&self, _: ProxyTarget) -> AppResult<()> {
        Err(PlatformError::Unsupported("当前平台尚未实现系统代理控制".into()).into())
    }
    fn disable(&self) -> AppResult<()> {
        Err(PlatformError::Unsupported("当前平台尚未实现系统代理控制".into()).into())
    }
}
```

### 3.2 `tun` 运行态激活模块

新增 `crates/air-platform/src/tun.rs`。**注意**：这与 `air-config/src/tun.rs`（配置模型）是两个 crate 的同名概念，必须用注释显式区分：

```rust
// crates/air-platform/src/tun.rs
// 本模块只负责 TUN “运行态激活/停用”：调用平台服务把 mihomo 切到 TUN 模式并托管提权进程。
// TUN “配置模型”（enable/stack/auto_route/dns_hijack 等字段）属于 air-config::tun，
// 二者通过 app 层衔接：配置模型决定“目标态”，本 trait 把目标态落到运行系统。
use air_error::AppResult;
use air_platform::core_service::CoreServiceSnapshot;

/// TUN 运行态控制抽象。业务层不直接调 Win32 / wintun / route。
pub trait TunControl: Send + Sync {
    /// 当前 TUN 是否处于激活态（来自内核服务运行态 + mihomo /configs 回读）。
    fn current_state(&self) -> AppResult<TunRuntimeState>;

    /// 激活 TUN。需要管理员权限时由实现内部走 core_service / elevated_process，
    /// UI 不得触发 UAC 弹窗以外的提权路径。
    fn activate(&self, plan: TunActivationPlan) -> AppResult<()>;

    /// 停用 TUN，回到普通系统代理/规则模式。
    fn deactivate(&self) -> AppResult<()>;
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TunRuntimeState {
    pub active: bool,
    /// 激活失败或降级时的诊断（已脱敏）；active=false 但 reason=None 表示用户主动关闭。
    pub reason: Option<String>,
}

#[derive(Clone, Debug)]
pub struct TunActivationPlan {
    /// 来自 air-config::TunConfigSettings 的目标态快照（stack/auto_route/dns_hijack 等）。
    /// 本 trait 不解析它，只在调用 core_service 启动 mihomo 时透传给运行配置合并。
    pub target_config_yaml: String,
}
```

**Windows 实现要点**（`WindowsTunControl`，`#[cfg(windows)]`）：

- TUN 必须以管理员权限运行 mihomo。激活路径优先走 `core_service`（已托管的内核服务）：若服务已安装且运行，直接 `PUT /configs` 切到含 `tun.enable=true` 的运行配置；若服务未就绪，则触发 `InstallCoreService` + `StartCoreService`（已存在的 `AppCommand`），由服务 worker 以 SYSTEM 权限重启 mihomo。
- **绝不**在 GUI 回调里 `ShellExecuteExW("runas")` 直接拉起 mihomo —— 提权只能经 `core_service` 或 `elevated_process::ElevatedChild`。
- 停用：把运行配置 `tun.enable=false` 写出并 `PUT /configs`；若 TUN 是由服务托管的唯一理由（即非 TUN 模式下用户不需要服务），按 `TunServicePolicy` 决定是否同时停服务（默认保留服务运行，避免频繁 UAC，由设置可调）。
- 与系统代理互斥策略：TUN 激活时**不强制关闭**系统代理（mihomo TUN 模式下系统代理通常无副作用），但 UI 在 TUN 激活态下把系统代理开关置灰为“TUN 模式下无需系统代理”，避免用户混乱。

**非 Windows 占位**：同 §3.1 模式，`UnsupportedTunControl` 返回 `PlatformError::Unsupported`。

### 3.3 工厂与注入

`AppServices`（`air-app/src/services.rs`）新增两个字段：

```rust
pub struct AppServices {
    // ... 现有字段 ...
    pub system_proxy: Box<dyn air_platform::system_proxy::SystemProxyControl>,
    pub tun: Box<dyn air_platform::tun::TunControl>,
}
```

`AppServices::new()` / `with_paths()` 用 `air_platform::system_proxy::default_system_proxy_control()` 和 `air_platform::tun::default_tun_control()` 注入。**业务层永远只看到 trait 对象**，平台切换不改动业务代码。

## 4. `air-app` 编排层设计

### 4.1 新增 `AppCommand` 变体

`crates/air-app/src/command.rs` 追加：

```rust
/// 启用/禁用 Windows 系统代理（HTTP/HTTPS），指向 mihomo mixed-port。
/// target 由 app 层从运行配置注入，不在命令载荷里硬编码 mihomo 地址。
SetSystemProxy {
    enabled: bool,
},
/// 激活/停用 TUN 运行态。配置目标态由 app 层从 air-config::TunConfig 合并得到，
/// 不在这里传整段 YAML，避免命令载荷膨胀和 secret 泄漏。
SetTunEnabled {
    enabled: bool,
},
/// 启动后被 UI/托盘轮询刷新系统代理/TUN 投影；核心未运行时安全降级。
RefreshPlatformProxyState,
```

`kind()` / `log_payload()` 同步补齐。`log_payload` 序列化的载荷里**不得**出现 secret/订阅 URL/代理密码（本命令只带 `enabled: bool`，天然安全）。

### 4.2 命令路由处理器

新增 `crates/air-app/src/router/platform_proxy.rs`（router 模块下与 `proxy`/`core` 并列），在 `router.rs::execute_command` 的 `match` 里追加分支：

```rust
AppCommand::SetSystemProxy { enabled } => {
    platform_proxy::handle_set_system_proxy(&context, enabled).await
}
AppCommand::SetTunEnabled { enabled } => {
    platform_proxy::handle_set_tun_enabled(&context, enabled).await
}
AppCommand::RefreshPlatformProxyState => {
    platform_proxy::handle_refresh_platform_proxy_state(&context).await
}
```

**`handle_set_system_proxy` 流程**：

1. 从 `core_config_store.load_user_config()` 读 `mixed-port`（无则回退默认 `7890`，并在日志里记”使用默认 mixed-port”）。`host` 固定 `127.0.0.1`。
2. `enabled=true` → `services.system_proxy.enable(ProxyTarget{host,port})`；`false` → `disable()`。
3. 成功后调 `services.system_proxy.current_state()` 回读，写 `snapshots.set_system_proxy(...)`，发 `AppEvent::SnapshotChanged`。
4. 失败：`tracing::warn!` 记 `redact_log_value(&error.to_string())`，`snapshots.set_last_error(redacted)`，发 `AppEvent::UserVisibleError { message: redacted }`。
5. **核心未运行**：系统代理开关**允许独立于核心**操作（用户可能想先关代理再停核），不作为前置失败；但 UI 提示“核心未运行，代理指向的端口暂不可用”。

**`handle_set_tun_enabled` 流程**：

1. **前置检查**：`snapshots.snapshot().runtime` 必须为 `Running`，否则返回 `UserVisibleError`（提示“TUN 需要核心运行中”），不静默失败。
2. 构造 `TunActivationPlan { target_config_yaml }`：`target_config_yaml` 来自 `build_effective_runtime_config()` 合并结果里把 `tun.enable` 设为目标值后的 YAML 片段（**只传 tun 段**，不传整份含 secret 的配置）。
3. `enabled=true` → `services.tun.activate(plan)`；`false` → `deactivate()`。
4. 成功后回读 `tun.current_state()`，写 snapshot，发事件。
5. 权限/服务未就绪失败：诊断必须脱敏，提示用户去设置页安装内核服务或重启应用提权，不暴露 SCM 内部错误码原文。

**`handle_refresh_platform_proxy_state`**：并发调 `system_proxy.current_state()` + `tun.current_state()`，任一失败只记日志不阻塞另一项；结果写 snapshot。供 UI 进入 Dashboard、托盘右键弹出前调用。

### 4.3 `AppSnapshot` 字段扩展

`crates/air-app/src/events.rs`：

```rust
pub struct AppSnapshot {
    // ... 现有字段 ...
    #[serde(default)]
    pub system_proxy: SystemProxyState,        // 来自 air_platform::system_proxy
    #[serde(default)]
    pub tun: TunRuntimeState,                  // 来自 air_platform::tun
}
```

`AppStateStore`（`crates/air-app/src/state.rs`）仿照 `set_core_service` 新增 `set_system_proxy` / `set_tun`，沿用 `update_snapshot_inner` + 仅在实际变化时发 `SnapshotChanged` 的现有模式（避免无变化抖动触发 UI 重渲染）。

### 4.4 配置持久化 vs 运行态激活（核心边界）

**这是阶段一最容易出错的边界**，必须显式分离：

| 维度       | 配置持久化（已有）                | 运行态激活（新增）                      |
| ---------- | --------------------------------- | --------------------------------------- |
| 目的       | 用户 YAML 里 `tun.enable` 的意图  | 当前系统里 TUN 是否真的开着             |
| 落地路径   | `AppCommand::SaveConfig`          | `AppCommand::SetTunEnabled`             |
| 写入目标   | `core.common.config.yaml`         | 注册表/服务/运行配置 + mihomo `PUT /configs` |
| 是否回写用户 YAML | 是                          | **否**（运行态不能污染用户 YAML）       |

现有 `dispatch_saved_tun_toggle`（`air-ui/src/shell/actions.rs`）只做配置持久化——**保留**，作为设置页“保存 TUN 配置”入口；新增“开关 TUN”按钮派发 `SetTunEnabled`，二者解耦。UI 文案要区分：“保存 TUN 配置”（写 YAML） vs “启用 TUN”（立即激活）。

`SetTunEnabled` 在激活成功后，**不**自动把 `tun.enable` 回写用户 YAML；用户在设置页保存的配置决定下次启动意图，运行态开关只影响当前会话。这与 CLAUDE.md “运行态 API 返回的临时状态不能直接写回用户 YAML”一致。

### 4.5 退出/停核还原策略

`SystemProxyState` 还原策略由 `AppSettings` 新增字段控制（见 §6.1）：

```rust
pub enum SystemProxyRestorePolicy {
    /// 退出时若代理由本应用启用，则禁用；用户自配的代理保持不动。
    DisableIfManaged,
    /// 退出时保持当前系统代理状态（用户可能希望代理持续）。
    Keep,
}
```

挂载点：`AppServices::stop_core_before_exit()`（`services.rs:322`）在停 mihomo 之前插入：

```rust
// 退出前按策略还原系统代理，避免遗留脏代理导致用户断网。
if matches!(self.settings_store.load()?.system_proxy_restore, SystemProxyRestorePolicy::DisableIfManaged) {
    if let Err(error) = self.system_proxy.disable() {
        tracing::warn!(error = %redact_log_value(&error.to_string()), "failed to restore system proxy on exit");
        // 还原失败不阻塞退出，但要记日志；UI 已不可见，不发 UserVisibleError。
    }
}
```

TUN 退出处理：`stop_core_before_exit` 停 mihomo 时 TUN 自然失效（mihomo 进程退出释放 wintun 适配器），无需显式 `tun.deactivate()`；但日志要记“核心退出，TUN 随之停用”。

### 4.6 日志与脱敏

- 系统代理/TUN 关键流程必须有 `tracing::info!`/`warn!`，载荷只含 `enabled`/`active`/端口/host（`127.0.0.1`），**不含** secret/订阅 URL/代理密码。
- 错误经 `redact_log_value(&error.to_string())` 后再写 `last_error` 和 `UserVisibleError`。
- `mixed-port` 数值不脱敏（非敏感）；`external-controller` 地址在 TUN 流程日志里只记端口不记完整 URL。

## 5. 托盘右键菜单设计

### 5.1 新增 `TrayEvent` 变体

`crates/air-platform/src/tray.rs`：

```rust
pub enum TrayEvent {
    ToggleWindow,
    ShowWindow,
    HideWindow,
    StartCore,
    StopCore,
    ToggleSystemProxy,   // 新增：切换系统代理（具体方向由 UI 读 snapshot 决定）
    ToggleTun,           // 新增：切换 TUN
    Quit,
}
```

### 5.2 菜单结构（动态勾选态）

`show_tray_menu` 当前每次右键静态重建——**保持重建模式**（Windows 托盘菜单本就是弹窗前一次性构造），但增加勾选态参数。新增 API：

```rust
/// 菜单勾选态，由 UI 从 AppSnapshot 投影后在右键弹窗前注入。
#[derive(Clone, Copy, Default)]
pub struct TrayMenuState {
    pub core_running: bool,
    pub system_proxy_enabled: bool,
    pub tun_active: bool,
}

// TrayHandle 新增方法，供 UI 调用设置下次右键菜单的勾选态。
impl TrayHandle {
    pub fn set_menu_state(&self, state: TrayMenuState) { /* 写入 OnceLock<Mutex<TrayMenuState>> */ }
}
```

菜单项与 ID 扩展（接续现有 `1005`）：

```
显示窗口          MENU_SHOW       1001
隐藏窗口          MENU_HIDE       1002
─────────────
启动内核          MENU_START_CORE 1003   (core_running 时置灰 / 或语义切换为“停止内核”)
停止内核          MENU_STOP_CORE  1004
─────────────
✓ 系统代理        MENU_SYSTEM_PROXY 1006   (system_proxy_enabled 时勾选)
✓ TUN 模式        MENU_TUN          1007   (tun_active 时勾选)
─────────────
退出              MENU_QUIT        1005
```

Windows 实现用 `AppendMenuW` 的 `MF_CHECKED` / `MF_UNCHECKED` 表达勾选；`MF_GRAYED` 表达“核心未运行时 TUN 不可点”。

### 5.3 UI 侧托盘事件处理

`crates/air-ui/src/shell/lifecycle.rs::spawn_tray_event_loop`（100ms 轮询）已存在；扩展 `Shell::handle_tray_event`（`actions.rs`）：

- `TrayEvent::ToggleSystemProxy` → 读 `snapshot.system_proxy.enabled`，派发 `SetSystemProxy { enabled: !current }`。
- `TrayEvent::ToggleTun` → 读 `snapshot.tun.active`，派发 `SetTunEnabled { enabled: !current }`；若 `runtime != Running`，弹全局通知“TUN 需要核心运行中”。
- **右键弹窗前**：UI 在轮询到 `WM_RBUTTONUP` 前无法预知，因此采用“每次 `SnapshotChanged` 时主动调 `tray_handle.set_menu_state(TrayMenuState::from(&snapshot))`”的策略，保证弹窗时勾选态是最新的。

### 5.4 单一通知入口

复用现有全局通知（`shell/components::push_global_notice`），不新开并行通知渠道。托盘菜单点击触发的成功/失败经命令路由 → `AppEvent::UserNotification` / `UserVisibleError` → 现有通知组件，与首页/设置页按钮走同一链路。

## 6. `air-ui` 首页与设置页设计

### 6.1 `AppSettings` 扩展

`crates/air-settings/src/model.rs`：

```rust
pub struct AppSettings {
    // ... 现有字段 ...
    #[serde(default)]
    pub system_proxy_restore: SystemProxyRestorePolicy,
    /// 启动时是否自动启用系统代理（核心进入 Running 后）。
    #[serde(default)]
    pub auto_enable_system_proxy: bool,
}
```

`app.config.toml` 反序列化必须对未知字段宽容（serde `default` + 未启用 `deny_unknown_fields`），与现有兼容策略一致。

### 6.2 新增 `AppRoute::Dashboard`

`crates/air-ui/src/routes.rs`：

```rust
pub enum AppRoute {
    Dashboard,        // 新增，置于 ALL 首位
    Subscriptions,
    ProxyGroups,
    // ... 其余不变 ...
}
```

`ALL` 数组改为 `[Dashboard, Subscriptions, ProxyGroups, Connections, RulesProxy, OverrideScript, Settings]`（7 项），同步更新 `routes.rs` 两个测试（`route_menu_order_matches_title_bar_design` 的期望 vec、`ALL` 长度）。`Dashboard::descriptor()`：label “首页”、title “仪表盘”、icon 选 `Icon::LayoutDashboard`（需确认 `icons` 模块有此图标，无则复用 `Icon::Gauge` 或新增 SVG 到 `crates/air-ui/assets/icons/`）。

### 6.3 Dashboard 页面状态

新增 `crates/air-ui/src/pages/dashboard/`（state.rs / render.rs / runtime_projection.rs 沿用现有页面目录范式）。**状态来源**：

| 展示项                | 来源                                            |
| --------------------- | ----------------------------------------------- |
| 内核运行状态           | `snapshot.runtime`                              |
| 系统代理开关 + 状态   | `snapshot.system_proxy`                         |
| TUN 开关 + 状态       | `snapshot.tun`                                  |
| controller 地址       | `snapshot.controller_addr`（脱敏后展示端口）   |
| 内核服务状态           | `snapshot.core_service`                         |
| 运行模式 / 流量 / 内存 | `snapshot.runtime_info` + `MihomoStreamEvent`   |
| 最近错误               | `snapshot.last_error`                           |

**禁止**使用 `fake()`；测试夹具改名为 `fake_for_test` 并限定 `#[cfg(test)]`。页面构造只从 `AppSnapshot` + 事件流取数。

首屏进入派发 `RefreshPlatformProxyState` + 现有 `RefreshProxies`/`RefreshRules`（按需）。开关按钮点击 → `SetSystemProxy` / `SetTunEnabled`，与托盘菜单共享同一路由。

### 6.4 状态栏扩展

`crates/air-ui/src/shell/render.rs::render_status_bar` 在内核状态项旁追加系统代理/TUN 微指示（小圆点 + tooltip），点击跳转 Dashboard。复用现有 4px 栅格 / 8px 圆角规范（`OPTIMIZATION_FOUNDATION.md`）。

## 7. 测试要求（CLAUDE.md 硬性约束）

### 7.1 `air-platform` 单元测试

- `system_proxy.rs`：`menu_ids_map_to_tray_events` 范式，测 `SystemProxyState` 默认值、`ProxyTarget` 构造、非 Windows 占位返回 `Unsupported`。
- `tun.rs`：`TunRuntimeState` 序列化往返；非 Windows 占位测。
- `tray.rs`：新增 `MENU_SYSTEM_PROXY` / `MENU_TUN` 的 `menu_event_from_id` 映射测试；`TrayMenuState` 默认值测试。

### 7.2 `air-app` 命令路由测试（仿 `router.rs` 现有测试）

必须在 `router.rs::tests` 新增：
- `set_system_proxy_success_updates_snapshot` — mock `SystemProxyControl`，断言 snapshot 字段更新 + `SnapshotChanged` 事件。
- `set_system_proxy_failure_emits_redacted_visible_error` — mock 返回错，断言 `UserVisibleError` 消息已脱敏（不含原始注册表路径/错误码原文）。
- `set_tun_enabled_is_noop_or_error_when_core_not_running` — `runtime=Idle` 派发 `SetTunEnabled{true}`，断言发 `UserVisibleError` 且不调 `tun.activate`。
- `set_tun_enabled_running_activates_and_reads_back_state` — `runtime=Running` + mock `TunControl`，断言 snapshot.tun.active 更新。
- `refresh_platform_proxy_state_partial_failure_does_not_block_other` — system_proxy 失败、tun 成功，断言 snapshot.tun 仍更新且不发 `UserVisibleError`（只 warn 日志）。

Mock 方案：因 `SystemProxyControl`/`TunControl` 是 trait 对象注入 `AppServices`，测试用 in-temp 构造时替换为 stub struct（记录调用），参照现有 `MonitoringStreamServer` 的 fake server 思路。

### 7.3 配置往返测试

- `AppSettings` 新增字段的 `app.config.toml` 反序列化对未知字段宽容测试（旧配置文件无新字段时回退默认）。
- `SystemProxyState` / `TunRuntimeState` 的 `serde` 往返（纳入 `AppSnapshot` 序列化测试）。

### 7.4 GUI 状态 reducer 测试

`crates/air-ui/src/shell.rs` 现有 `traffic_stream_event_updates_status_bar_*` 测试范式：新增 `snapshot_change_updates_dashboard_proxy_state`、`tray_toggle_system_proxy_dispatches_command_with_inverted_flag`（纯状态 reducer，不启动 GPUI 渲染）。

### 7.5 脱敏验证

`set_system_proxy_failure_emits_redacted_visible_error` 必须断言 `UserVisibleError.message` 不含：原始注册表路径、SCM 错误码、订阅 URL、secret。沿用 `core_log_tail_line_is_redacted_for_monitoring_event` 的断言风格。

## 8. 落地步骤（建议顺序）

1. **`air-platform` 抽象先行**：新增 `system_proxy.rs` / `tun.rs` trait + Windows 实现 + 非 Windows 占位 + 单测。`lib.rs` 注册模块。`cargo check` + `cargo test` 通过。
2. **`air-app` 接入**：`AppServices` 注入 trait 对象；`command.rs` 新增 3 个命令变体；`router/platform_proxy.rs` 实现；`events.rs`/`state.rs` 扩展 snapshot；`services.rs` 退出还原。补 router 测试。
3. **`air-platform/tray.rs` 扩展**：新 `TrayEvent` 变体 + `TrayMenuState` + 菜单项 + 勾选态 + 单测。
4. **`air-ui` 托盘事件**：`handle_tray_event` 处理新变体；`SnapshotChanged` 时 `set_menu_state`。
5. **`air-ui` Dashboard**：`AppRoute::Dashboard` + 页面状态 + render；更新 routes 测试。
6. **`air-settings`**：新增 `SystemProxyRestorePolicy` / `auto_enable_system_proxy` + 设置页控件。
7. **`.codex` 文档同步**：更新 `gui_core_interaction.md`、`software_and_core_lifecycle.md`、`data_flow_and_storage.md`，记录系统代理/TUN 新链路与配置-vs-激活边界。

每步完成后 `cargo fmt && cargo check && cargo test`，Windows 主平台必须全绿；macOS/Linux 允许 unsupported 路径编译通过（trait 占位保证 `cfg` 不进业务层）。

## 9. 完成判据（与 roadmap.md 阶段一一致）

- Windows 下可一键启用/禁用系统代理并正确回填；退出/停核按 `SystemProxyRestorePolicy` 还原。
- TUN 开关可用；权限不足/服务未就绪时给出脱敏诊断，UI 不直接调 Win32。
- 托盘右键菜单含系统代理/TUN 项，勾选态与主窗口/内核/代理/TUN 实时一致。
- Dashboard 首页聚合运行态，无 fake 数据污染生产路径。
- 所有新增命令有成功/失败/取消/核心未运行测试，错误脱敏有断言。
- 平台逻辑全部隔离在 `air-platform`，`air-app`/`air-ui` grep 不到 `cfg(target_os)`。

## 10. 风险与开放问题

- **mixed-port 缺失**：用户配置无 `mixed-port` 时系统代理指向何处？默认回退 `7890` 并日志提示，需在设置页可配。
- **TUN 与现有 `dispatch_saved_tun_toggle` 共存**：两条路径（配置保存 vs 运行态激活）UI 文案必须清晰，否则用户会困惑“我开了 TUN 为何没生效”（可能只保存了配置未激活）。
- **服务托管 TUN 的停服务策略**：`TunServicePolicy` 默认保留服务运行，但便携模式/用户洁癖场景可能希望停 TUN 即停服务——留为设置项，阶段一先实现“保留服务”默认。
- **macOS/Linux 真实实现**留给阶段二；本阶段占位必须返回明确 `Unsupported` 诊断，不静默断功能。
