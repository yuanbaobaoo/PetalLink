# DESIGN.md — PetalLink - 华为云盘客户端开源版

> 设计系统：**PetalLink v2**（logo 品牌蓝 × macOS Native · 毛玻璃 · 小圆角卡片化）
> 视觉学派：**Modern Minimal**（Tech Utility 倾向）
> 原型版本：v2.0 · 2026-07（第三版：logo 蓝 + 小圆角 + 放松布局）

---

## 1. Visual Theme（视觉主题）

**Philosophy**：功能第一，克制美学——像 macOS 原生应用一样精准、高效、无冗余装饰。

**Direction**：`minimal, utilitarian, information-dense, mac-native`

**Personality**：专业、克制、高效、可信赖

**Reference**：macOS Finder + Big Sur 统一工具栏 —— 毛玻璃侧栏、白卡内容区、柔和低饱和投影。

**与 v1 的差异**：主色由 TDesign `#0052D9` 换为 app logo 蓝 `#0053DB`；圆角整体收小（5/8/10/12）；引入品牌渐变、毛玻璃材质与彩色文件类型 tile；功能覆盖与 v1 完全一致，仅视觉重构。

### 原型文件清单（高保真静态原型，唯一视觉事实来源）

| 文件 | 说明 |
|------|------|
| `design/index.html` | v2 原型导航页 + 功能覆盖对照表 |
| `design/01-login.html` | 登录页 — 品牌渐变画布 + 玻璃拟态卡片（默认/授权中/异常三态） |
| `design/02-main.html` | 主界面 — 毛玻璃侧栏 + 64px 工具栏 + 文件列表（常态/空文件夹/搜索结果） |
| `design/03-sync-states.html` | 同步状态 — 引导 banner 三态 + sync-bar 多态 + 失败详情弹窗 |
| `design/04-transfer.html` | 传输队列 Popover（440×580）— stat-pill 统计条 + 九种任务状态 |
| `design/05-file-ops.html` | 文件操作 — 深色悬浮批量栏 + 右键菜单 + 四个对话框 |
| `design/06-settings.html` | 设置页 — 分组导航 240px + 白卡面板 + 毛玻璃保存底栏 |
| `design/07-logs.html` | 日志查看 — 描边过滤 chips + 白卡日志列表 |
| `design/08-update.html` | 更新 — 侧栏渐变更新卡片 + 更新对话框五态 |
| `design/09-tray.html` | 托盘菜单（原生）+ Toast 深色浮丸 |
| `design/assets/v2.css` | 共享样式表，全部设计 token 的唯一来源 |
| `design/assets/sprite.js` | 内联 SVG 图标 sprite（32 个，`#i-*`） |

前端实现对齐方式：`app/styles/tokens.css` 复刻 v2.css 令牌（并保留旧语义别名映射）；图标 sprite 与 `design/assets/sprite.js` 保持一致（`app/components/IconSprite.vue`）。

---

## 2. Color Palette（调色板）

### Primary（品牌蓝 · 取自 logo 主色 #0053DB）

| Token | HEX | Usage |
|-------|-----|-------|
| `--brand-50` | `#EFF4FE` | 选中行/导航激活底、软色按钮底、chip 底 |
| `--brand-100` | `#DCE8FC` | 软色按钮 hover、spinner 轨道 |
| `--brand-200` | `#B7D0F7` | 输入聚焦描边环 |
| `--brand-400` | `#4A8BF0` | 品牌渐变起点、空态 badge 图标 |
| `--brand-500` | `#0053DB` | 主色：按钮、链接、选中态 |
| `--brand-600` | `#0047B8` | 按下态 |
| `--brand-700` | `#003A99` | 深色强调 |
| `--grad-brand` | `linear-gradient(135deg, #4A8BF0, #0053DB)` | 主按钮、logo tile、头像、更新卡片、进度条 |
| `--grad-brand-soft` | `linear-gradient(135deg, #EFF4FE, #DCE8FC)` | 空态 badge 底 |

### Neutral（墨色 + 背景 + 线条）

| Token | Value | Usage |
|-------|-------|-------|
| `--ink-900` | `#1C1C1E` | 正文、文件名 |
| `--ink-700` | `#3A3A3E` | 次级强调、幽灵按钮文字 |
| `--ink-600` | `#56565C` | 次级信息、目录树文字 |
| `--ink-400` | `#8E8E93` | 辅助信息、meta |
| `--ink-300` | `#AEAEB2` | 占位符、分组标题、footer |
| `--bg-app` | `#F5F5F7` | 页面背景（设置页） |
| `--bg-card` | `#FFFFFF` | 卡片、内容区 |
| `--bg-fill` | `#F1F1F3` | 输入框底、灰底 chip、进度条轨道 |
| `--bg-hover` | `#F7F7F9` | 列表行 hover |
| `--line` | `rgba(0,0,0,0.06)` | 常规分隔线（0.5~1px） |
| `--line-strong` | `rgba(0,0,0,0.1)` | 强调分隔、描边 chip |

### Semantic（功能色 · 柔和现代）

| Token | HEX | Usage |
|-------|-----|-------|
| `--ok` / `--ok-bg` | `#0CA678` / `#E3F5EE` | 已同步、完成、成功 |
| `--warn` / `--warn-bg` | `#F08C00` / `#FFF3DE` | 冲突、等待、警告 |
| `--err` / `--err-bg` | `#E5484D` / `#FDECEC` | 失败、删除、危险操作 |
| `--info` / `--info-bg` | `#3B82F6` / `#E8F0FE` | 下载方向、信息提示 |

### macOS 窗口专用色

| Token | Value | Usage |
|-------|-------|-------|
| 关闭 / 最小化 / 最大化 | `#FF5F57` / `#FFBD2E` / `#28C840` | traffic light |

### 深色组件（仅两类）

批量操作栏与 Toast 使用近黑毛玻璃浮层：`rgba(28,28,30,0.92~0.94)` + `backdrop-filter: blur(8px)`，危险项文字 `#FDA4AF`，成功图标 `#4ADE80`，错误图标 `#FB7185`。

> 当前版本仅提供浅色主题；深色模式令牌暂未定义。

---

## 3. Typography（排版）

### Font Stack

```css
--font-family: -apple-system, BlinkMacSystemFont, "SF Pro Text", "PingFang SC", "Helvetica Neue", sans-serif;
--font-mono: "SF Mono", "Menlo", "Monaco", "Consolas", monospace;
```

等宽字体用于：路径、文件 ID、SHA256、日志 meta、OAuth 地址。

### Scale（字号阶梯）

| Level | Token | Size | Weight | Usage |
|-------|-------|------|--------|-------|
| Title Medium | `--font-title-md` | 20px | 600 | 登录卡片品牌名 |
| 对话框标题 | — | 17px | 600 | dialog / popover 标题 |
| Title Small | `--font-title-sm` | 16px | 600 | 区块标题、侧栏 Logo |
| Body | `--font-body` | 14px | 400 | 文件名、正文、按钮 |
| Body Small | `--font-body-sm` | 13px | 400 | 次级信息、面包屑、设置描述(12.5) |
| Caption | `--font-caption` | 12px | 400/500 | chip、时间戳、表头(11.5) |
| 分组标签 | — | 11px | 600 | 侧栏/设置分组（letter-spacing 0.4px） |

数字统一 `font-variant-numeric: tabular-nums`（配额、进度、统计）。

---

## 4. Radius / Shadow / Spacing

### 圆角（整体收小）

| Token | Value | Usage |
|-------|-------|-------|
| `--radius-sm` | 5px | chip、文字按钮、小按钮 |
| `--radius-md` | 8px | 按钮、输入框、菜单项、导航项、方向 tile |
| `--radius-lg` | 10px | 卡片、面板、对话框 badge、菜单/批量栏 |
| `--radius-xl` | 12px | 对话框、传输 Popover、登录卡片 |
| `--radius-full` | 999px | 开关、进度条、分隔短线 |

### 阴影（柔和、低饱和）

| Token | Value | Usage |
|-------|-------|-------|
| `--sh-sm` | `0 1px 2px rgba(16,24,40,.05)` | 卡片（配 `0 0 0 0.5px var(--line)` 细描边） |
| `--sh-md` | `0 4px 16px -2px rgba(16,24,40,.08)` | 悬浮卡 |
| `--sh-lg` | `0 12px 32px -8px rgba(16,24,40,.14)` | 批量栏、Toast |
| `--sh-pop` | `0 24px 64px -12px rgba(16,24,40,.22)` | 对话框、菜单、Popover |
| `--sh-brand` | `0 4px 14px -4px rgba(0,83,219,.35)` | 渐变主按钮、logo tile、更新卡片 |

### 间距（4-grid）

`--space-xs: 4` / `--space-sm: 8` / `--space-md: 12` / `--space-lg: 16` / `--space-xl: 24` / `--space-xxl: 32`

### 布局尺寸

| Token | Value | Usage |
|-------|-------|-------|
| `--sidebar-width` | 248px | 主界面侧栏（毛玻璃） |
| `--settings-nav-width` | 240px | 设置页导航 |
| `--appbar-height` | 64px | 主界面工具栏 |
| `--sync-bar-height` | 44px | 同步状态条（min-height） |
| `--breadcrumb-height` | 40px | 面包屑 |
| `--file-header-height` / `--file-row-height` | 38px / 56px | 文件表头/行 |
| `--transfer-popover-width` / `-height` | 440px / 580px | 传输队列弹层 |

### 材质（毛玻璃）

侧栏 `rgba(247,247,249,0.85) + blur(20px)`；菜单 `rgba(255,255,255,0.96) + blur(16px)`；登录卡片 `rgba(255,255,255,0.9) + blur(16px)`；设置底栏 `rgba(255,255,255,0.85) + blur(12px)`；遮罩 `rgba(28,28,30,0.36) + blur(3px)`。

---

## 5. Icons（图标）

TDesign 风格线性图标：`stroke-width 1.5`、`viewBox 0 0 24 24`、`currentColor`、圆角线帽。

- 全局唯一一份 sprite：`app/components/IconSprite.vue`（与 `design/assets/sprite.js` 同步，32 个 `#i-*` symbol）
- 通过 `<MateIcon name="folder" :size="16" />` 引用（`use href="#i-folder"`）
- 目录树文件夹图标固定橙 `#F0A63C`（激活态品牌蓝）；文件类型 tile 见 §6 文件列表

---

## 6. Components（组件规范）

### 按钮（五级体系，MateButton）

| 变体 | 样式 | Usage |
|------|------|-------|
| `primary` | 36px，grad-brand 渐变 + `--sh-brand`，hover `brightness(1.06)`，禁用 `opacity .45` 去阴影 | 主操作（同步索引/登录/保存） |
| `soft` | 36px，brand-50 底 brand-500 字，hover brand-100 | 次要强调（传输队列） |
| `icon-text`（幽灵） | 36px 透明底 ink-700 字 + ink-400 图标，hover bg-fill | 工具栏次级（Finder） |
| `text` | 品牌蓝文字，hover brand-50 底 | 文字链接（重新授权/更换目录） |
| `icon` | 36×36 圆形，ink-400，hover bg-fill + ink-700 | 圆形图标按钮（设置/关闭） |

危险态：primary → `var(--err)` 实心 + 红色发光阴影；幽灵/软色 → 红字 + err-bg hover。

### chip（MateTag）

24px 高、radius-sm、12px/500；七色：brand / ok / warn / err / gray / info + 描边可点变体（`box-shadow: inset 0 0 0 1.5px`，选中态品牌蓝描边 + brand-50 底，用于日志过滤）。小尺寸 20px（chip--xs）。

### banner（MateInfoBanner）

radius-lg、无底框、padding 12/14：info（brand-50 底 + 品牌蓝图标）/ warn（warn-bg）/ err（err-bg）/ success（ok-bg），正文 ink-700。

### 对话框（MateDialog）

460~480px、radius-xl、`--sh-pop`、无底框；header 24px padding + 40px icon badge（brand-50/err-bg/ok-bg 三态）+ 17px 标题；body 14px ink-600 行高 1.65；footer 右对齐间距 10px，**无顶部分隔线**。遮罩 `rgba(28,28,30,.36) + blur(3px)`。

### 菜单（MatePopupMenu / 右键菜单）

min-width 200px、毛玻璃、radius-lg、`--sh-pop`、padding 6px；菜单项 36px、radius-md、hover bg-fill；危险项红字 + err-bg hover；分隔线 1px `var(--line)` 内缩 10px。

### Toast（深色浮丸）

近黑 `rgba(28,28,30,.92) + blur(8px)`、radius-lg、13px/500 白字；成功绿勾 `#4ADE80` / 警告 `#FBBF24` / 错误 `#FB7185` 图标。

### 输入体系

- 文本/搜索（MateTextField/MateSearchField）：38px、bg-fill 底、radius-md、无底框；聚焦 → 白底 + `inset 0 0 0 2px var(--brand-200)` 描边环
- 数字（MateNumberField）：同上，居中数字 + ink-400 单位后缀
- 步进器（MateStepper）：bg-fill 胶囊容器 padding 3；按钮 30×30 radius-sm，hover 白底 + 品牌蓝 + `--sh-sm`
- 开关（MateSwitch）：46×28 iOS 胶囊，off `#E3E3E6` / on brand-500，knob 22px 白圆
- 复选（MateCheckbox）：18px、1.5px ink-300 描边、radius-sm，选中/半选 brand-500 实心

### 进度 / 空态

- 线性进度（MateLinearProgress）：6px、bg-fill 轨道、grad-brand 填充（ok/err/ink-300 变体）
- 空态（MateEmpty）：72px grad-brand-soft 圆角 badge + brand-400 图标 + 15px/600 ink-700 标题 + 13px ink-400 描述

### 文件列表（FileListView）

- 表头 38px：11.5px/600/letter-spacing 0.3px ink-400，可排序列 hover 品牌蓝
- 行 56px、radius-md、hover bg-hover、选中 brand-50
- **文件类型 tile（ftile 32px 圆角 6px）**：folder `#FFF4DE/#F0A63C` · doc `#EEF2FF/#6366F1` · image `#FDE7F3/#EC4899` · video `#F3E8FF/#8B5CF6` · file `bg-fill/ink-400`；图片/视频用缩略图填满 tile
- 行状态图标：已同步 `i-local` 绿 / 本地占位 ink-600 / 仅云端 `i-cloud` ink-300
- 底部 footer 36px：12px ink-300「N 项 · 已全部加载」
- 批量操作栏：44px 深色悬浮胶囊（见 §2 深色组件），白色幽灵按钮 + 危险项 `#FDA4AF`

### 传输队列 Popover（440×580）

header 60px（17px 标题）→ stat-pill 统计条（处理中/等待中/已完成灰底 pill + 历史失败 err-bg pill + 清空菜单）→ 任务行 min-height 68px：36px 方向 tile（上传 brand-50 / 下载 info-bg / 删除 bg-fill）+ 20px 方向迷你 chip + 文件名 13.5px/500 + 进度条或红色错误文案 + 右侧状态文本。

### 设置页

- 导航 240px：分组标签（11px/600 ink-300）+ nav-item 46px radius-md（激活 brand-50 + 品牌蓝）
- 内容区 bg-app padding 28/32；settings-panel 白卡（radius-lg + `--sh-sm` + 0.5px 细描边，padding 4/24）
- group-header 12px/600 ink-400 大写感；setting-row 16px 纵向 padding + 底线（末行无）
- 底栏 64px 毛玻璃：保存（primary）+ 重置（text）+ 右侧「配置已保存」ok chip
- 信息展示：info-row（label 96px ink-400）/ props-row（label 88px ink-400，mono 值 12px）

### 登录页

品牌渐变画布（径向 brand-100/brand-50 光斑 + 150° 浅色线性渐变）+ 3 个低透明装饰圆；玻璃卡片 480px（白 90% + blur 16 + radius-xl + `--sh-pop`，padding 44/40/32）；72px 品牌图标 + 20px/600 标题 + 44×3 渐变分隔线；主按钮 46px radius-lg；授权中 = brand-50 横条 + spinner + 灰字「取消授权」。

### 更新（Sidebar 卡片 + 对话框）

- 侧栏更新卡片：grad-brand 渐变 + `--sh-brand`，标题行（版本号 + 20px 白色半透明关闭钮）+ 白底品牌蓝字「立即更新」按钮；下载中态为白色进度条
- 更新对话框五态：发现新版本（changelog 灰底面板 + 稍后提醒/立即更新）→ 下载中（渐变进度条 + 后台下载）→ 等待传输（spinner + 禁用按钮）→ 更新就绪（ok badge + 立即重启）→ 更新失败（err badge + 重试）

---

## 7. Animations（动画）

| 动画 | 参数 | Usage |
|------|------|-------|
| `spin` | 1s linear infinite | 同步中/加载图标旋转 |
| `pulse` | 1.6s ease-in-out infinite（opacity 1→.35） | sync-bar 状态呼吸点 |
| `menu-fade-in` | 0.12s ease-out（scale .96→1） | 菜单弹出 |
| `popover-in` | 0.12s ease-out（translateY -4→0） | Popover 滑入 |
| `dialog-fade-in` | 0.15s ease-out | 对话框淡入 |
| `toast-in` | 0.2s ease-out（translateY 8→0） | Toast 浮入 |

统一提供 `prefers-reduced-motion` 降级。

---

## 8. 前端实现约束

- 所有颜色/尺寸必须引用 `app/styles/tokens.css` 中的 CSS 变量，禁止硬编码（ftile 配色等组件级常量除外，需在注释中说明来源）
- 旧语义别名（`--color-brand`、`--bg-page`、`--text-primary` 等）保留并映射到 v2 值；新增代码优先使用 v2 原生命名（`--brand-500`、`--ink-900`、`--bg-fill`、`--line`）
- 组件库：`app/components/mate/`（Mate 前缀），业务组件不得绕过 Mate 重复造轮子
- 深色模式暂未提供，新增组件无需适配 `prefers-color-scheme`

---

> **文档版本**：v2.0（对齐 `design/` 高保真原型第三版）
