# Sovereignty Check: add-agent-eval

> **change**: add-agent-eval
> **评估日期**: 2026-07-11
> **评估人**: AI
> **参考**: ADR-007 数据主权红线

## 评估结论: ✅ 通过(EvalJudge 强制本地 + PII 脱敏)

### 检查项 1: 评估器 LLM 调用是否强制本地?

**状态**: ✅ 通过

- `WorkType::EvalJudge` 的 `is_local_only()` 返回 `true`
- 与 Evolution / SoulCompile / Classifier 一致,强制路由到本地 Ollama
- 即使用户在 `models.json` 中配置了非本地 override,该 override 被忽略
- 代码位置:`src-tauri/src/llm/dispatcher.rs`

**验证方式**:
```rust
// 测试: EvalJudge 必须强制本地
#[test]
fn eval_judge_is_local_only() {
    assert!(WorkType::EvalJudge.is_local_only());
}
```

### 检查项 2: 评估数据是否外发?

**状态**: ✅ 通过

- 评估输入(Agent 输出)在送入 judge 前经过 PII 脱敏(Scrubber)
- 评估结果(评分卡)只写入本地 SQLite,不发送到任何远端服务
- Trace 数据只写入本地 SQLite + 可选 JSONL 文件导出
- 不集成任何第三方评估工具(LangSmith / Langfuse / Braintrust)

### 检查项 3: PII 脱敏是否充分?

**状态**: ✅ 通过

脱敏覆盖以下 PII 类型:

| PII 类型 | Regex 模式 | 替换为 |
|---------|-----------|--------|
| 邮箱 | `[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}` | `[REDACTED]` |
| 电话 | `\d{3}-\d{3,4}-\d{4}` | `[REDACTED]` |
| 身份证 | `\d{15,18}` | `[REDACTED]` |
| 银行卡 | `\d{16,19}` | `[REDACTED]` |

**多层防御**:
1. Regex 脱敏(Scrubber)
2. Judge prompt 声明"忽略个人信息"
3. Judge 本地运行(Ollama,数据不出设备)

### 检查项 4: 评估结果是否影响数据主权?

**状态**: ✅ 通过

- 评估结果不写入记忆层(见 memory-impact.md)
- 评估结果不参与跨设备同步
- 评估结果不外发
- 用户可以随时删除评估数据(`nebula eval clean`)

### 检查项 5: Trace 数据是否包含敏感信息?

**状态**: ⚠️ 需注意(已缓解)

- Trace 数据包含 Agent 的输入/输出文本,可能包含敏感信息
- **缓解 1**: Trace 默认关闭(`NEBULA_EVAL_TRACING=1` 才启用)
- **缓解 2**: Trace 数据只存储在本地 SQLite
- **缓解 3**: Trace 导出 JSONL 时可选启用 PII 脱敏(`--scrub` 选项)
- **缓解 4**: 定期清理 30 天前的 Trace 数据

### 检查项 6: 评测集是否包含真实用户数据?

**状态**: ✅ 通过

- 默认评测集(`evalsets/default.yaml`)使用**合成数据**,不包含真实用户对话
- 回归评测集(`evalsets/regression.yaml`)使用**合成数据**
- 用户可以创建自定义评测集,但需自行确保数据安全
- 文档中明确提示:自定义评测集中的数据会被本地 Ollama 处理

## 总结

| 检查项 | 状态 |
|--------|------|
| EvalJudge 强制本地 | ✅ |
| 评估数据不外发 | ✅ |
| PII 脱敏充分 | ✅ |
| 评估结果不影响数据主权 | ✅ |
| Trace 数据敏感信息 | ⚠️ 已缓解 |
| 评测集不含真实数据 | ✅ |

**结论**: 本 change 符合 ADR-007 数据主权红线,可以进入实现阶段。
