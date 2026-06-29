
# 九头蛇 最终白皮书

## ——基于"黑洞-海绵"记忆引擎与蜂群协作架构的桌面 AI Agent

**版本**：v1.5（实况版）  
**日期**：2026-06-28  
**作者**：Solo Developer  
**状态**：v1.1.7 已实现，本文档追认现状  

---

## 0. 2026-06-28 现状声明

本白皮书原为 v1.0 MVP 设计文档（2026-06-20），假设 10 人团队、800-1200 万预算、7.5 个月周期。

**实际情况**：单人开发，已交付 v1.1.7，功能远超原 v1.0 MVP，实际覆盖大量原计划 v1.5/v2.0 模块。本文档更新为 **v1.5 实况版**。

### 已实现 vs 原计划

| 模块 | 原计划 | 现状 |
|------|--------|------|
| v7.0 记忆 L0-L7 + Sponge/Blackhole | v1.0 | ✅ 已实现 |
| Reflection 反思引擎 | v1.0 | ✅ 已实现 |
| Swarm 蜂群协作（6角色） | v1.0 | ✅ 已实现 |
| Writing/Work/Code 三模式 | v1.0 | ✅ 已实现 |
| Skill 市场 | v2.0 | ✅ 已实现 |
| OS 集成（Shell/Clipboard/通知） | v1.5 | ✅ 已实现 |
| E2EE 加密同步 | v1.5 | ✅ 已实现 |
| gRPC / MCP 协议 | v1.5/v2.0 | ✅ feature-gated |
| Chat 面板 | 未计划 | ✅ 已实现 |
| Memory Map 可视化 | v2.0 | ✅ 已实现 |
| Onboarding 引导 | v1.1 | ✅ 已实现 |
| Prompt 注入防护 + 沙箱 | v1.1/v2.0 | ✅ 已实现 |

### 明确不在此版本

- 移动端
- OS-Controller（真正的 OS 级自动化）
- L5 真 Metacognition（当前为 v0 假意识）
- 团队/多用户支持

---

## 1. 产品定位

**不是聊天 AI**——是能直接干活的数字员工。  
**不是单智能体**——是 AI 小队协同。  
**不是云端依赖**——本地优先 + 云端按需。  
**不是封闭产品**——开源生态（MIT License）。

### 三个核心工作模式

| 模式 | 场景 | 蜂群角色 | 工具 |
|------|------|---------|------|
| **Writing** | 写作/报告/邮件 | Writer + Reviewer | 模板库 + Markdown编辑器 + 导出 |
| **Work** | 任务管理/会议 | Planner + Reviewer | Kanban + 时间追踪 + 会议纪要 |
| **Code** | 编程/调试/部署 | Coder + Reviewer + Planner | Monaco编辑器 + Terminal + Git |

### 其他面板

- **Chat**：自由对话，L1 记忆自动吸收对话
- **Swarm View**：实时查看蜂群 Agent 工作状态
- **Memory**：记忆列表 + 关系图谱可视化
- **Skills**：技能市场——安装/发布/评分可复用 AI 技能

---

## 2. 技术架构

- **桌面框架**：Tauri 2.0 + Rust
- **前端**：Preact + TypeScript + Tailwind CSS
- **数据存储**：SQLite（结构化） + LanceDB（向量）
- **本地 LLM**：Ollama + Qwen2.5-3B（可切换云端模型）
- **进程模型**：单进程（Core/Memory/Swarm/LLM 均在同一 Rust 运行时内）

### Feature Gates（按需编译）

`	oml
[features]
default = ["vector-store"]
grpc = [...]          # gRPC 服务端（默认关闭）
mcp = []              # MCP 协议客户端（默认关闭）
self-evolution = []   # 自我进化模块（默认关闭）
channels = []         # Telegram/Discord 桥接（默认关闭）
did-identity = []     # DID 去中心化身份（默认关闭）
crdt-sync = []        # CRDT 多设备同步（默认关闭）
`

---
