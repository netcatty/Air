# 外部参考笔记

> 本文是**外部项目参考记录**，不是 air 的源码事实或编码规范。
> 不强制遵循；当 air 阶段一/二/三（见 `roadmap.md`）需要做相关能力时，可回查本文借鉴。
> 与 air 现状不一致时，**以 air 源码和 `AGENTS.md` 为准**。
> 主要参考对象：OxideTerm（一个 Rust+GPUI 的成熟桌面应用，scope 远大于 air）。
> 参考整理完毕后，`oxideterm-main/` 完整源码副本已从仓库移除（git 未追踪，仅本地保留过查阅副本），本笔记是唯一留存的参考记录。

## 1. 借鉴原则

OxideTerm 是 97-crate 的大型应用（SSH/SFTP/RDP/VNC/Telnet/Serial/AI/插件/云同步/IDE/CLI），air 是 10-15 crate 的代理管理器。**借鉴方法论，不借鉴粒度**：

- ✅ 抄：架构纪律、平台抽象配方、发版自动化模式、安全实践
- ❌ 不抄：97-crate 拆分、3-channel 发版、远端 agent 子项目、DynamicTexture GPU 帧缓冲、Tauri 1.x 兼容

## 2. 值得借鉴的五件事

### 2.1 域 crate 零 I/O + effect 枚举模式

**OxideTerm 做法**：域 crate（`fernomade/runtime` 等）禁止 `import GPUI`，也不直接做 socket/文件 I/O。域层只产出 `SessionAction` 纯效果枚举，由 app 层（embedder）执行 I/O。这让域 crate 无网络、无平台、无 GPUI 也能单测。规则机器可读地写在 `.agents/skills/split-crate-by-responsibility/SKILL.md`。

**对 air 的参考**：`air-mihomo` 可朝这个方向演进——产出 `MihomoAction` 值（start/stop/set-system-proxy/configure-tun），由 `air-app` 执行。air 当前 CLAUDE.md 的"GUI 层只负责展示"是把边界放在 GUI↔app；OxideTerm 进一步把边界推到 app↔域，域层也无 I/O。这是更严但也更可测的境界，**非阶段一硬性要求**，可作为长期重构方向。

### 2.2 "UI 生命周期 ≠ 后端生命周期" 不变量

**OxideTerm 做法**：`.agents/skills/oxideterm-node-session-ownership/SKILL.md` 明确——关闭一个 pane 只移除该消费者，不杀底层连接；连接存活由 registry 决定，不由 UI 决定。

**对 air 的参考（阶段一直接相关）**：air 关窗口/隐藏到托盘时，**mihomo 进程和系统代理设置必须存活**；托盘菜单是后端的真正 owner，不是主窗口。这条不变量应作为阶段一"关闭到托盘"和"退出还原"设计的隐含前提——air 当前 `stop_core_before_exit` 只在退出（非隐藏）时触发，已符合此不变量；新增系统代理开关后，退出还原策略（`SystemProxyRestorePolicy`）也按此原则设计。

### 2.3 三层平台抽象（阶段二必需）

**OxideTerm 配方**：
- 平台特有依赖在 `Cargo.toml` 用 `cfg(target_os)` 声明，不在业务代码散落。
- Linux 桌面集成用 **`ashpd`**（XDG Portal）：通知/自启/文件选择/打开 URI/回收站一个 crate 全覆盖，不为 GNOME/KDE 各写适配。
- 凭证存储用 **`keyring`** 三原生后端（`apple-native`/`windows-native`/`sync-secret-service`）。
- 提权操作隔离成**独立 helper 进程**（如 `oxideterm-rdp-helper`），JSON-RPC 通信，而不是给整个 app 提权。

**对 air 阶段二的参考**：`air-platform` 直接采用这套。`ashpd` 让 Linux TUN/通知/自启从"每个 DE 写适配"变成"一个 crate 搞定"；`keyring` 让 mihomo secret/订阅 URL token 进 OS keychain 而非明文 YAML；TUN 提权走独立 helper 进程而非整体 UAC，契合 air "GUI 保持普通权限"原则。

### 2.4 凭证 zeroize + keyring（安全，低成本高收益）

**OxideTerm 做法**：`SessionKey` 包 `Zeroizing<[u8;16]>`；自定义 `Debug` 显示 `[REDACTED]`；凭证只进 OS credential store 不进明文配置；规则在 `.agents/skills/oxideterm-secret-zeroize/SKILL.md`。

**对 air 的参考**：mihomo secret、订阅 URL token、代理密码应走 `keyring` + `Zeroizing<T>` + 红色 `Debug`。air 当前 CLAUDE.md 的脱敏规则止于"日志脱敏"；OxideTerm 推到"内存不留明文 + 持久化进 OS keychain"。对代理管理器尤其重要——订阅 URL token 泄漏≈账号泄漏。**阶段一非硬性**，可在引入订阅凭证持久化时一并做。

### 2.5 发版自动化（air 已部分采用，可再补两块）

**OxideTerm 做法**：`compose_release_notes.py` 用 marker 模板拼装 release body + 自动下载表；`bump_version.py` 一次同步 workspace 版本 + 所有 README badge + Cargo.lock；tag 用 annotated tag（`git tag -a`）。

**对 air 的参考**：air 的 `release.yml` 已借鉴核心模式（tag 触发、`update_release: true`、`generate_release_notes`）。还差：
- ① 一份 `bump_version` 脚本，避免版本号在 Cargo.toml/Cargo.lock/README badge 多处漂移。
- ② release notes 下载表生成，阶段二多平台后尤其需要。

## 3. 推迟到阶段二/三再看

- **自动更新**：`air-update` crate + `minisign-verify` 签名 + `semver` 防降级 + `reqwest` 的 `system-proxy` 特性。**关键约束**：更新器必须尊重系统代理设置（用户用代理才能访问 GitHub Releases）。阶段二补。
- **Linux Wayland→X11 fallback、虚拟 GPU 检测降级渲染**：抄 `OXIDETERM_PATCHES.md` 的能力探测式 fallback。阶段二。
- **`air-i18n` crate + LEGAL.md**：阶段一先中英双语，阶段三再扩。LEGAL.md 建议覆盖代理凭证/订阅 URL 的隐私声明。
- **benchmark 基建**：代理吞吐/启动时间基准，模式可复用，阶段三。

## 4. 不借鉴

- **97-crate 粒度**：OxideTerm scope 大 5 倍，air 保持 10-15 crate，同样的依赖纪律、更粗粒度。
- **3-channel 发版（stable/beta/gpui-preview）**：air 用 2-channel（stable+beta）即可。
- **DynamicTexture / 远程桌面 GPU 帧缓冲**：与 air 无关。
- **Tauri 1.x updater 兼容**：air 无历史包袱。

## 5. 关于 "agent/ 子项目" 的澄清

OxideTerm 的 `oxideterm-main/agent/` 是**远端主机的 SSH 边带程序**（把 Mosh 协议跑在远端机器上，通过 stdin/stdout JSON-RPC 与本地 app 通信），单独 workspace 单独编译，拷进 app 资源跨机部署。**air 没有这种远端边带需求，不要照搬 `agent/` 子项目结构。**

air 真正需要的是**本地提权 helper**（TUN/系统代理的 UAC 路径），对应的是 OxideTerm 里 `oxideterm-rdp-helper`/`oxideterm-vnc-helper` 这类**本地独立 helper 二进制**模式。而 air **已有等价物**：`crates/air-platform/src/core_service/` 的服务 worker + `elevated_process.rs`。

**决策**：
- 阶段一：沿用现有 `core_service` worker（已以 SYSTEM 权限托管 mihomo），**不新建 helper 子项目**。
- 若阶段二发现 TUN/系统代理需要更独立的提权边界，再考虑拆 `crates/air-helper`——作为**同一 workspace 成员**，不是 OxideTerm 那种独立 workspace。
- 不要建 `agent/` 目录，避免后人误以为要写远端边带。

## 6. 三条架构纪律（供参考，非强制）

借鉴 OxideTerm 时值得记住的三条纪律，分别对应 air roadmap 的三个阶段：
1. **域 crate 零 I/O、只产 effect 枚举**（对应阶段三多内核抽象：把 mihomo/sing-box/xray 都建模成产 action 的域层）。
2. **UI 生命周期 ≠ 后端生命周期**（对应阶段一：托盘 owner 后端，关窗口不杀核/不还原代理）。
3. **平台差异收口在 Cargo.toml + ashpd + keyring + 独立 helper 进程**（对应阶段二跨平台）。

这三条是方法论，不是 air 现状。落到设计时以 `docs/design-phase-1.md` 和 air 源码为准。

## 7. 参考对象的处置

`oxideterm-main/` 目录曾是参考用的完整第三方源码副本，体量大，不属于 air 仓库。本笔记沉淀完毕后已从仓库移除（该目录 git 原本就未追踪，删除不影响版本控制）；本地若仍保留查阅副本也不进版本控制。

CI 参考：air 的 `ci.yml`/`release.yml` 的 `concurrency` 取消策略和 `update_release: true` 模式源自 OxideTerm 的 `native-package.yml`，已落地，无需再保留 `oxideterm-main/.github/` 参考。
