# 对话系统 行为契约

> **领域**: chat
> **状态**: 当前系统行为 (v2.3.0)
> **最后更新**: 2026-07-11

## 概述

对话系统是用户与 Nebula 交互的主要界面,负责消息收发、多模型调度、流式响应、工具调用展示与上下文窗口管理。系统通过 UnifiedModelDispatcher 统一调度 LLM,支持流式输出,并在前端以消息气泡 + 工具调用卡片呈现。

## Requirements

### Requirement: 多模型调度
The system SHALL dispatch chat LLM calls through the UnifiedModelDispatcher with a provider fallback chain.
- 调度优先级:DeepSeek(OpenAI 兼容层)→ Ollama(本地 fallback)→ Anthropic(远端兜底)
- WorkType = Chat 时,根据 `models.json` 中 `work_type_overrides` 路由到具体 (provider, model)
- 本地路径持有独立 `CircuitBreaker`,与远端 Gateway 断路器解耦
- `UNIFIED_DISPATCHER_ENABLED=0` 可禁用 dispatcher,回退到 LlmGateway 直连(安全网)
- 进化类 WorkType(Evolution/SoulCompile/Classifier)强制走本地 Ollama

#### Scenario: DeepSeek 优先调度
- **WHEN** 用户发送聊天消息且 DeepSeek 已配置
- **THEN** UnifiedModelDispatcher 优先路由到 DeepSeek(OpenAI 兼容层)
- **AND** 若 DeepSeek 不可用,回退到 Ollama,再回退到 Anthropic

#### Scenario: 断路器独立熔断
- **WHEN** 远端 Gateway 连续失败触发断路器
- **THEN** 本地 Ollama 路径不受影响(独立断路器)
- **AND** 聊天继续走本地 Ollama 响应

### Requirement: 流式响应
The system SHALL stream assistant responses token-by-token to the frontend via Tauri events.
- `chat_stream` 命令返回 `BoxStream<Result<StreamToken>>`
- 前端通过 Tauri event 监听增量 token,实时渲染到消息气泡
- 流式过程中支持取消(CancellationToken)
- 流式完成后,完整响应持久化到 L1 记忆层

#### Scenario: 流式输出实时渲染
- **WHEN** 用户发送消息并触发 `chat_stream`
- **THEN** 助手回复以 token 粒度实时流入前端消息气泡
- **AND** 用户可在流式过程中看到逐字输出

#### Scenario: 流式取消
- **WHEN** 用户在流式过程中点击"停止"
- **THEN** CancellationToken 触发,流式终止
- **AND** 已接收的部分响应保留在消息气泡中

### Requirement: 工具调用卡片
The system SHALL render tool/function calls as interactive cards in the chat UI.
- 工具调用经 `ToolCallCard` 组件渲染,显示工具名、参数、状态(pending/running/done/error)
- 支持展开/折叠参数与返回值
- 工具调用结果可被 LLM 引用作为后续推理上下文
- 工具调用经 `full_injection_scan` 安全扫描后执行

#### Scenario: 工具调用展示
- **WHEN** LLM 决定调用工具(如 `web_search`)
- **THEN** 前端渲染 `ToolCallCard`,显示工具名与参数
- **AND** 工具执行期间状态为 `running`,完成后变为 `done`
- **AND** 用户可点击展开查看返回值

### Requirement: 消息气泡 UI
The system SHALL render conversation messages as bubbles with user/assistant differentiation.
- 用户消息与助手消息视觉区分(对齐方向、颜色)
- 支持 Markdown 渲染(marked + DOMPurify 清毒)
- 代码块语法高亮(highlight.js)
- Mermaid 图表内联渲染
- 工具调用卡片嵌入消息气泡内

#### Scenario: Markdown 渲染
- **WHEN** 助手回复包含 Markdown 格式(标题、列表、代码块)
- **THEN** 消息气泡渲染为格式化 HTML
- **AND** 代码块语法高亮,Mermaid 图表内联渲染
- **AND** HTML 经 DOMPurify 清毒,防 XSS

### Requirement: 上下文窗口管理
The system SHALL assemble the LLM context window from active memory layers (L1/L2/L3/L4/L6) with token budget control.
- 进入上下文窗口的层:L1(会话历史)、L2(跨会话经验)、L3(事实)、L4(蒸馏知识)、L6(原则)
- L0(临时)、L5(反思)、L7(核心价值)不直接进入上下文窗口
- Token 预算控制:CostPolicy 限制 `max_tokens_per_task` 与 `daily_task_limit`
- TokenJuice 三级压缩:超预算时压缩历史对话
- 语义缓存(SemanticCache):cosine ≥ 0.92 且未过 TTL(1h)时短路返回缓存响应

#### Scenario: 上下文窗口组装
- **WHEN** 构造 LLM 请求上下文
- **THEN** 从 L1/L2/L3/L4/L6 层检索相关记忆
- **AND** 按 token 预算截断,超出部分经 TokenJuice 压缩

#### Scenario: 语义缓存命中
- **WHEN** 用户查询与缓存中某条目 cosine 相似度 ≥ 0.92 且未过 TTL
- **THEN** 直接返回缓存的响应文本,跳过 LLM 调用
- **AND** 累加 `semantic_cache_hits` 计数器

### Requirement: 注入防护
The system SHALL scan all chat inputs for prompt injection, dangerous commands, and invisible Unicode before processing.
- `full_injection_scan` 检测三类攻击:Prompt 注入模式、SSH 后门/恶意命令、不可见 Unicode
- Critical 级别注入/凭证泄露:直接拦截,返回错误提示
- 非 Critical 但不安全:记录警告但继续处理(降级模式)
- 不可见 Unicode 被 `strip_invisible_unicode` 移除后传入 LLM

#### Scenario: 关键注入拦截
- **WHEN** 用户消息包含覆盖 System Prompt 的注入模式或凭证泄露
- **THEN** 请求被拦截,返回"输入包含潜在的安全风险(注入攻击或凭证泄露),已被拦截"
- **AND** 累加注入命中计数,记录 warn 日志
