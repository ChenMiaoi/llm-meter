# LLM Meter 桌面 UI 设计基线

> 版本：v0.1\
> 状态：Draft / 视觉与交互基线\
> 日期：2026-07-13\
> 适用窗口：Waybar Popup、Tray Popup、Main Window\
> 上位架构：[架构总览](./architecture-overview-v0.1.md)\
> 平台约束：[Hyprland 桌面集成](./hyprland-integration-v0.1.md)

## 1. 设计方向

用户提供的参考图体现了一种适合 LLM Meter 的桌面形态：从 Waybar 打开的紧凑浮层，以深色低干扰背景承载多张圆角状态卡，通过少量亮色、细线图和右对齐数值快速传达当前状态。

LLM Meter 借鉴这种视觉语言，但不复制系统监控器的信息结构。设计目标是：

- 像“桌面状态中心”，而不是缩小版后台管理 Dashboard；
- 打开后 2 秒内看清最重要的额度、同步状态和今日用量；
- 高密度但不拥挤，初始视口只展示最关键的一张趋势图；
- 使用 Capability 决定组件是否出现，不为缺失数据保留零值空壳；
- 官方报告、本地观测、派生和估算在视觉上可区分；
- Popup 适合快速查看，Main Window 适合深入分析。

## 2. 参考图的取舍

| 参考特征 | LLM Meter 的采用方式 |
|---|---|
| 靠近 Waybar 的右上浮层 | 作为 Hyprland 默认 Popup Profile；实际位置由 compositor rule 决定 |
| 深色、轻微分层的背景 | 采用三层 surface token，不依赖透明或 blur 才可读 |
| 顶部图标、标题、关闭按钮 | 保留，并增加同步状态与最后更新时间 |
| 多张圆角卡片 | 用于主额度、用量摘要、趋势与连接状态 |
| 黄色与淡紫色双强调色 | 黄色表示主要额度/警告，淡紫表示次要序列；状态色仍按语义定义 |
| 细线趋势图 | 用于 7 日 Token 或费用趋势，首屏最多一张主要图 |
| 左标签、右数值 | 用于紧凑 Metric Row，数值等宽并保持可比较性 |
| 背景纹理和高亮边缘 | 只作为可选装饰，不能降低文字对比度 |

不照搬的部分：

- 不为每个指标都放一张等高折线图；
- 不把所有 Provider 和 Connection 压缩进一个无层级面板；
- 不仅靠黄/紫两种颜色表达正常、警告和错误；
- 不使用固定物理像素坐标模拟 Waybar 锚定；
- 不让装饰图案、发光或透明度成为信息结构的一部分。

## 3. Popup 信息优先级

从上到下固定为以下顺序：

1. Window Header；
2. 全局错误或认证提示，仅异常时出现；
3. Primary Quota Card；
4. Today Summary；
5. Primary Trend；
6. Secondary Quota / Connection List；
7. Footer Actions 与数据时间。

主额度选择规则沿用 Waybar：

1. 用户显式选择的主 Connection 和 Quota Window；
2. 未配置时选择剩余比例最低的有效窗口；
3. 没有有效 Quota 时，以今日 Token 或实际费用作为主卡片；
4. 没有任何用量数据时显示空状态，不显示 `0%`。

## 4. Popup 线框

```text
┌──────────────────────────────────────────┐
│ ◩  LLM Meter             ● 已同步    ×  │
│    OpenAI · ChatGPT Pro     2 分钟前     │
├──────────────────────────────────────────┤
│ Codex 5h window               官方报告  │
│ 63% 剩余                                 │
│ █████████████░░░░░░░                    │
│ 2h 14m 后重置                            │
├──────────────────────────────────────────┤
│ 今日                                     │
│ Token          1.26M    API 费用  $12.43 │
│ 请求             842    缓存效率     41% │
├──────────────────────────────────────────┤
│ 最近 7 天 Token                          │
│ 3.2M ┤        ╭─╮              ╭──╮     │
│      ┤  ╭──╮ ╭╯ ╰──╮  ╭───────╯  ╰─    │
│   0  └────────────────────────────────   │
├──────────────────────────────────────────┤
│ 其他额度                                 │
│ Weekly window                    81%  ›  │
│ Credits                          240  ›  │
├──────────────────────────────────────────┤
│ 最后成功同步 21:28      管理连接  刷新  │
└──────────────────────────────────────────┘
```

该线框表达信息层级，不冻结具体文案、图标、比例或组件高度。

## 5. 核心组件

### 5.1 Window Header

必须包含：

- LLM Meter 图标与标题；
- 当前聚合范围或主 Connection；
- 同步状态和数据年龄；
- 可点击关闭按钮；
- 键盘可达的窗口菜单入口，可在 Main Window 中展开。

关闭按钮使用明确的 hover/focus 状态，点击只关闭当前窗口 surface，不停止 daemon。

### 5.2 Primary Quota Card

显示条件：存在 `QUOTA_WINDOWS` 且至少一个窗口含 Provider 明确报告的比例或额度值。

内容顺序：

```text
窗口名称 → 来源标签 → 剩余比例/值 → 进度条 → 重置时间
```

规则：

- 默认用“剩余”作为主语义；
- 进度条与数字方向一致，避免条表示已用、文字表示剩余；
- Provider 只返回比例时，不显示估算 Token；
- `resets_at` 缺失时省略重置行；
- 临界状态同时使用图标、文字和颜色；
- 多个窗口放入 Secondary Quota List，不在首屏堆叠多个大卡片。

### 5.3 Today Summary

最多显示四个紧凑 Metric：

- 今日 Token；
- 实际 API 费用；
- 请求数；
- 缓存效率。

Capability 不存在或口径不兼容时隐藏对应 Metric，并让剩余项自动重排。Subscription 与 API Platform 的数值不能在未说明范围时合并。

### 5.4 Primary Trend

Popup 首屏最多一张主要趋势图，默认优先级：

1. 用户选择的趋势；
2. 每日 Token；
3. 实际费用；
4. 请求数。

趋势卡必须标出范围与周期，例如“ChatGPT Pro · 最近 7 天 Token”，不能只写“趋势”。

### 5.5 Secondary Quota / Connection List

使用紧凑行而不是大卡片：

```text
状态图标  名称       次要说明       主数值  chevron
```

最多直接显示三行，其余进入“查看全部”。认证失效和 Provider error 排在普通连接之前。

### 5.6 Footer

包含：

- 最后成功同步时间；
- 本地 Snapshot 刷新；
- 显式 Provider 同步动作；
- 管理连接 / 打开 Main Window。

“刷新视图”和“同步 Provider”必须用不同文案和状态。前者不触发外部请求，后者受 daemon 限流策略约束。

## 6. 视觉 Token

以下为方向值，不是最终品牌色冻结：

```text
surface.canvas       #09091d
surface.panel        #0f1029
surface.card         #141532
border.subtle        rgba(168, 161, 255, 0.12)

text.primary         #f4f2ff
text.secondary       #a7a2d8
text.muted           #74709e

accent.primary       #f3ed63   主额度、选中态
accent.secondary     #aaa4ff   次要趋势、链接
state.success        #86d7a2
state.warning        #f3c969
state.critical       #f08a8f
state.offline        #9b97ad
```

布局 Token：

```text
space:       4 / 8 / 12 / 16 / 24
radius:      10 / 14 / 18
border:      1 logical px
popup width: 360 logical px
popup gap:   12 logical px
```

视觉约束：

- 正文文字与背景达到 WCAG AA 对比度；
- 半透明关闭后仍保持足够对比度；
- 用户关闭动画、透明或 blur 时信息层级不改变；
- 数字默认使用 tabular numerals；
- 不要求 Nerd Font 才能理解状态，图标必须有文字或 accessible label。

## 7. 图表规范

- Popup 使用 SVG 或 Canvas 2D，不要求 WebGL；
- Main Window 可以提供更完整图表，但必须保留非 WebGL fallback；
- 同一图最多两条序列，分别使用主/次强调色；
- Cached Input 是 Input 的子集，不能作为额外 Token 重复堆叠；
- 缺失 bucket 用断线或空白表示，不用零线补齐；
- hover/focus tooltip 显示时间、值、单位、Scope 和 Provenance；
- 仅有一两个样本时显示点或数值，不伪造平滑趋势；
- stale 数据在卡片上显示时间标签，不通过降低整个窗口透明度隐藏问题。

## 8. 状态变体

### Ready

- Header 显示正常同步状态；
- 卡片使用正常强调色；
- 数据时间在 Footer 可见。

### Syncing

- 保留旧 Snapshot；
- Header 显示轻量进度，不清空卡片；
- 禁止重复触发同一 Connection 同步。

### Stale / Offline

- 保留最后成功值；
- 顶部显示非阻塞状态条；
- 标注最后成功同步时间；
- 不把历史值伪装成实时值。

### Auth Required

- 在主额度之前显示操作卡；
- 说明受影响的 Connection；
- 提供“重新连接”；
- 其他正常 Connection 仍继续显示。

### Empty

- 区分“尚未添加连接”“Provider 未提供该能力”“同步尚未完成”；
- 不显示全零图表；
- 提供唯一明确的下一步操作。

### Provider Error

- 显示脱敏原因、影响范围和恢复动作；
- 技术错误码进入可复制详情；
- 不展示 Provider 原始响应。

## 9. Popup 交互

- Waybar 左键：toggle Popup；
- Waybar 右键：打开/聚焦 Main Window；
- `Escape`：关闭 Popup；
- 点击关闭按钮：关闭 Popup；
- 点击“固定”：关闭 Popup 并打开 Main Window；
- 失去焦点：可以延迟收起，但系统对话框、OAuth 和文件选择流程不得被取消；
- 所有可点击元素支持键盘 focus ring；
- Popup 打开后初始焦点放在内容容器或首个异常操作，不自动落到关闭按钮；
- 不使用 hover 才能发现的关键操作。

## 10. Popup 与 Main Window 的复用

共享：

- Snapshot ViewModel；
- Metric、Quota、Status、Provenance 组件；
- 主题与格式化规则；
- 空状态和错误状态。

不共享布局：

- Popup 使用单列、有限首屏和渐进展开；
- Main Window 使用导航、筛选、维度表格和完整图表；
- Main Window 不受 Popup 的固定宽高和自动收起规则限制。

不得通过缩放整个 Main Window 页面来生成 Popup。

## 11. 响应式规则

- 360 × 440 是 Waybar 紧凑摘要尺寸，不承载完整趋势和明细；
- 高度不足时内容区滚动，Header 与关键恢复动作保持可达；
- 宽度低于 380 时 Today Summary 从两列变为单列紧凑行；
- 文字缩放到 200% 时不出现水平滚动；
- 长 Connection 名称单行省略，并在 tooltip/accessibility name 中保留全称；
- 多语言文案不依赖固定字符宽度；
- fractional scale 下允许 compositor 取整，边框不得因半像素消失。

## 12. 动效

- Popup 出现使用短促 `popin` 或淡入，建议 120–180 ms；
- 卡片数据变化只高亮变化区域，不让全部卡片闪烁；
- 趋势线首次绘制可以轻量过渡，后续增量更新不重复播放完整动画；
- 遵守 `prefers-reduced-motion`；
- compositor 动画关闭时，应用仍然立即可用；
- 不用持续脉冲表示正常同步。

## 13. UI 验收标准

1. Popup 首屏无需滚动即可看见同步状态、主额度和至少两个主要用量值。
2. 缺失 Metric 不显示为零，也不保留无意义空卡。
3. Subscription 与 API Platform 的范围在所有组合卡片中清晰可辨。
4. 任一警告状态不只依赖颜色表达。
5. 关闭 Popup 不停止 daemon 或 Waybar watch。
6. 键盘可以访问关闭、详情、重新认证、刷新和 Main Window 操作。
7. 200% 文字缩放下关键操作不被裁切。
8. 1.0 与 1.5 scale 下卡片边框和图表清晰。
9. 没有 WebGL 时图表仍可显示。
10. stale 状态保留历史数据并明确显示数据年龄。
11. 首屏主要趋势图不超过一张，避免复制系统监控器的图表密度。
12. 去掉 blur、背景纹理和动画后，信息层级与可读性不变。

## 14. 与其他文档的关系

- Capability、Metric、Scope、Provenance 与 Missing 规则以[详细架构](./llm-meter-architecture-v0.1.md)为准；
- Popup 的 Wayland 定位、Window Role、UWSM 和 Waybar 命令以[Hyprland 集成](./hyprland-integration-v0.1.md)为准；
- 本文只定义视觉信息层级、组件行为和 UI 验收，不定义 Provider API 或持久化结构。
