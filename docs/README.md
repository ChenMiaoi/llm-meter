# LLM Meter 文档索引

> 文档基线：v0.1\
> 基线日期：2026-07-13\
> 当前阶段：架构设计，不代表功能已经实现

## 推荐阅读顺序

1. [架构总览](./architecture-overview-v0.1.md)
   - 用于快速理解系统边界、进程关系、依赖方向、数据所有权和交付阶段。
2. [详细架构设计](./llm-meter-architecture-v0.1.md)
   - Core Domain、Provider Adapter、指标、额度、存储、IPC、安全和测试的完整规格。
3. [桌面 UI 设计基线](./desktop-ui-design-v0.1.md)
   - 定义 Waybar Popup 的信息层级、视觉方向、组件状态、响应式与可访问性。
4. [Hyprland 桌面集成](./hyprland-integration-v0.1.md)
   - Linux / Wayland / Hyprland 下的窗口角色、Waybar、systemd user、UWSM 和验收规范。

## 文档职责

| 文档 | 定位 | 规范性 |
|---|---|---|
| `architecture-overview-v0.1.md` | 全局导航和边界摘要 | 摘要性；不得覆盖详细领域契约 |
| `llm-meter-architecture-v0.1.md` | 跨平台核心与通用 Linux 架构 | 核心规范 |
| `desktop-ui-design-v0.1.md` | Popup/Main Window 的视觉和交互 | UI 规范 |
| `hyprland-integration-v0.1.md` | Hyprland 平台 Profile | Hyprland 环境下的补充规范 |

发生冲突时按以下顺序处理：

1. Provider、指标、存储、安全和 IPC 以详细架构设计为准；
2. Popup 的信息层级、组件和可访问性以桌面 UI 设计基线为准；
3. Hyprland 的窗口、会话、Waybar 和桌面行为以 Hyprland 集成文档为准；
4. 架构总览只负责串联，不单独引入新的领域语义。

## 当前已确定的关键决策

- daemon-first：采集、归一化、存储、告警均由 `llm-meterd` 完成；
- Provider-neutral、Capability-driven、Connection-first；
- Secret 只进入操作系统凭据库，不进入 SQLite、IPC 或 WebView；
- Waybar 和桌面 UI 都只消费 daemon 的本地 Snapshot；
- Hyprland v0.1 使用普通 Wayland `xdg-toplevel` + compositor window rule，不引入 layer-shell；
- Hyprland 下不依赖应用自行设置全局窗口坐标；
- daemon 生命周期与图形会话解耦，桌面进程与图形会话绑定；
- 缺失值不是零，任何指标都必须携带来源、范围和统计周期。

## 状态标记

文档中的关键词含义如下：

- **必须**：v0.1 验收所需，不满足即视为架构偏离；
- **应该**：默认方案，只有记录明确理由后才能偏离；
- **可以**：兼容或增强能力，不阻塞 v0.1；
- **待验证**：实现前需要通过 spike、fixture 或目标环境实测确认。
