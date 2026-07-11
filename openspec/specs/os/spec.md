# OS 控制 行为契约

> **领域**: os
> **状态**: 当前系统行为 (v2.3.0)
> **最后更新**: 2026-07-11

## 概述

OS 控制模块是 Nebula 与操作系统交互的桥梁,涵盖 UI 自动化(UIAutomator)、VLM 视觉控制器、剪贴板监控、Shell 执行、操作录制/回放、系统托盘与电源管理。Windows 平台为完整实现,macOS/Linux 为骨架占位。

## Requirements

### Requirement: UI 自动化
The system SHALL provide a trait-based UI automation abstraction (UiAutomator) with platform-specific implementations.
- `UiAutomator` trait 定义 UI 自动化接口:元素查找 / 点击 / 输入 / 滚动 / 截图
- Windows:通过 `windows-sys` crate 调用 Win32 API + UIAutomation(完整实现)
- macOS:骨架占位(T-S6-A-01b),返回 `Err`,未接入 AppKit/CoreGraphics
- Linux:骨架占位(T-S6-A-01c),返回 `Err`,需 X11/Wayland 集成
- `OsControllerService` 封装平台调用,通过 `#[cfg(target_os = ...)]` 守卫分发
- `WindowInfo` 结构体:平台无关的窗口信息(hwnd / title / process_name)
- 可在 sidecar 进程中通过 gRPC 暴露,也可进程内直接调用

#### Scenario: Windows 窗口枚举
- **WHEN** 调用 `os_list_windows` 命令
- **THEN** Windows 实现通过 Win32 API 返回所有可见窗口的 `WindowInfo` 列表
- **AND** macOS/Linux 返回 `Err`(骨架未实现)

#### Scenario: 前台窗口读取
- **WHEN** 调用 `os_get_foreground_window`
- **THEN** Windows 实现返回当前前台窗口标题与 hwnd
- **AND** 非 Windows 平台返回 `Err`

### Requirement: VLM 控制器
The system SHALL provide a VLM-driven OS automation mode via screenshot → VLM analysis → action execution loop.
- `VlmController` 融合 VLM 视觉理解 + `UiAutomator` 操作执行
- 闭环:截图 → VLM 分析(`describe_image` 经 UnifiedModelDispatcher 走 Ollama 多模态 API)→ 操作执行 → 循环
- `BoundingBox`:归一化坐标(0.0-1.0),与屏幕分辨率解耦
- `VlmAction` 动作枚举:Click / Type / Scroll / KeyPress / Wait / Screenshot / Done
- `ScreenAnalysis`:VLM 对截图的分析结果
- `VlmExecutionResult` / `VlmStepRecord`:闭环执行结果与单步记录
- 适用场景:Canvas、自绘 UI、远程桌面等 Accessibility API 无法操控的场景

#### Scenario: VLM 闭环自动化
- **WHEN** 用户请求"点击屏幕上的蓝色按钮"
- **THEN** VlmController 截图 → VLM 分析定位蓝色按钮的 BoundingBox
- **AND** 通过 UiAutomator 执行 Click 动作
- **AND** 截图验证结果,若未完成则继续循环

#### Scenario: 不可访问元素兜底
- **WHEN** Accessibility API 无法操控目标元素(如 Canvas 自绘 UI)
- **THEN** VLM 模式通过视觉分析定位并操作
- **AND** 补充 API 模式的覆盖盲区

### Requirement: 剪贴板监控
The system SHALL monitor the clipboard in the background with content detection and sponge absorption.
- `ClipboardWatcherEngine` 后台轮询剪贴板变更
- `ClipboardEvent` / `ClipboardKind`:文本 / 图片 / 文件
- 内容检测:识别剪贴板内容类型(URL / 代码 / 密钥 / 普通文本)
- Sponge 吸收:有价值的剪贴板内容经 SpongeEngine 吸收到记忆系统
- `ClipboardService` 提供跨平台剪贴板读写(`arboard` crate v3.3)

#### Scenario: 剪贴板内容吸收
- **WHEN** 用户复制一段代码到剪贴板
- **THEN** ClipboardWatcherEngine 检测到剪贴板变更
- **AND** 识别内容为代码类型
- **AND** 经 SpongeEngine 吸收为候选记忆(标注 SourceKind = Clipboard)

### Requirement: Shell 执行
The system SHALL execute shell commands with timeout and argument parsing.
- `ShellExecutor` 提供同步 shell 命令执行(带超时)
- `parse_argv`:使用 `shell-words` crate 解析命令行参数
- `ShellOutput`:stdout / stderr / exit_code
- 默认超时:`DEFAULT_TIMEOUT`
- 安全:命令执行前经 `full_injection_scan` 检测危险命令

#### Scenario: Shell 命令执行
- **WHEN** 技能请求执行 `git status`
- **THEN** `ShellExecutor` 解析参数并执行
- **AND** 返回 `ShellOutput`(stdout / stderr / exit_code)
- **AND** 超过 `DEFAULT_TIMEOUT` 时强制终止

#### Scenario: 危险命令拦截
- **WHEN** 命令包含 `rm -rf /` 等危险模式
- **THEN** `scan_dangerous_commands` 命中,命令被拦截
- **AND** 返回安全风险提示

### Requirement: 操作录制与回放
The system SHALL record UI operations and replay them on demand.
- `ActionRecorder` 录制 UI 操作序列(点击 / 输入 / 滚动 / 快捷键)
- 录制结果存储为可回放的脚本
- 回放:按录制的顺序重放操作
- `DesignMode`:可视化设计界面创建自动化流程(无需录制)

#### Scenario: 操作录制与回放
- **WHEN** 用户启动录制,执行一系列 UI 操作后停止
- **THEN** ActionRecorder 保存操作序列
- **AND** 用户选择"回放"后,按录制顺序重放操作

### Requirement: 托盘与电源管理
The system SHALL provide system tray integration and power state management.
- `tray` 模块:Tauri tray-icon,显示应用状态、快速操作菜单
- `power` 模块:`PowerManager` 监听系统睡眠/唤醒事件
- 电源事件:睡眠时暂停 LLM 调用与蜂群任务,唤醒时恢复
- `shortcut` 模块:全局快捷键 `CmdOrCtrl+Shift+H` 唤起应用
- `context_menu`:Windows 右键菜单"问Nebula"(注册表 HKCU 写入)
- `file_handler`:文件关联(.md / .txt / .nebula / .nmemory)与拖入处理
- `notifications`:系统通知(`NotificationLevel`:Info / Warning / Error)

#### Scenario: 系统睡眠暂停
- **WHEN** 系统进入睡眠状态
- **THEN** PowerManager 检测到睡眠事件
- **AND** 暂停所有进行中的 LLM 调用与蜂群任务
- **AND** 唤醒后恢复被暂停的任务

#### Scenario: 全局快捷键唤起
- **WHEN** 用户按下 `CmdOrCtrl+Shift+H`
- **THEN** 应用窗口被唤起/聚焦
- **AND** 若窗口已隐藏则显示,若已聚焦则不影响
