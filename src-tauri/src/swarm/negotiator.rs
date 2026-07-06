use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use super::agents::AgentOutput;
use crate::llm::{ChatMessage, ChatResponse, LlmGateway};

// ---------------------------------------------------------------------------
// T-E-S-04: MoA (Mixture of Agents) 一等公民
// ---------------------------------------------------------------------------

/// MoA 合议策略。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MoAStrategy {
    /// 投票:各模型独立回答,评分模型打分,取最高分
    Voting,
    /// 级联:按 participants 顺序,简单→复杂逐步升级
    Cascading,
    /// 仲裁:各模型回答后,scoring_model 综合选最优(复用 llm_arbitrate)
    Arbitration,
}

/// MoA 配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoAConfig {
    /// 参与模型列表(如 ["ollama:qwen2.5:3b", "deepseek:deepseek-chat"])
    pub participants: Vec<String>,
    /// 合议策略
    pub strategy: MoAStrategy,
    /// 评分模型(用于 Voting 策略的评分,默认取 participants[0])
    pub scoring_model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NegotiationResult {
    pub chosen: AgentOutput,
    pub method: NegotiationMethod,
    pub conflict_detected: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NegotiationMethod {
    HighConfidence,
    LlmArbitration,
    FallbackHighestConfidence,
}

pub struct Negotiator {
    confidence_threshold: f32,
}

impl Negotiator {
    pub fn new() -> Self {
        Self {
            confidence_threshold: 0.8,
        }
    }

    pub fn with_confidence_threshold(mut self, threshold: f32) -> Self {
        self.confidence_threshold = threshold.clamp(0.0, 1.0);
        self
    }

    pub fn negotiate(&self, outputs: Vec<AgentOutput>) -> NegotiationResult {
        if outputs.len() <= 1 {
            return NegotiationResult {
                chosen: outputs.into_iter().next().unwrap_or_else(|| {
                    AgentOutput::new(
                        super::agents::AgentKind::Generic,
                        "system",
                        "no outputs to negotiate",
                    )
                }),
                method: NegotiationMethod::HighConfidence,
                conflict_detected: false,
            };
        }

        let has_conflict = self.has_conflict(&outputs);

        if !has_conflict {
            let best = self.highest_confidence(outputs);
            return NegotiationResult {
                chosen: best,
                method: NegotiationMethod::HighConfidence,
                conflict_detected: false,
            };
        }

        let best = self.highest_confidence(outputs.clone());
        if best.confidence >= self.confidence_threshold {
            info!(
                target: "nebula.negotiator",
                confidence = best.confidence,
                "high confidence output selected without arbitration"
            );
            return NegotiationResult {
                chosen: best,
                method: NegotiationMethod::HighConfidence,
                conflict_detected: true,
            };
        }

        NegotiationResult {
            chosen: best,
            method: NegotiationMethod::FallbackHighestConfidence,
            conflict_detected: true,
        }
    }

    pub async fn negotiate_with_arbitration(
        &self,
        outputs: Vec<AgentOutput>,
        llm: &LlmGateway,
    ) -> Result<NegotiationResult> {
        if outputs.len() <= 1 || !self.has_conflict(&outputs) {
            let result = self.negotiate(outputs);
            return Ok(result);
        }

        let best = self.highest_confidence(outputs.clone());
        if best.confidence >= self.confidence_threshold {
            return Ok(NegotiationResult {
                chosen: best,
                method: NegotiationMethod::HighConfidence,
                conflict_detected: true,
            });
        }

        match self.llm_arbitrate(outputs, llm).await {
            Ok(chosen) => Ok(NegotiationResult {
                chosen,
                method: NegotiationMethod::LlmArbitration,
                conflict_detected: true,
            }),
            Err(e) => {
                warn!(target: "nebula.negotiator", error = ?e, "LLM arbitration failed; falling back to highest confidence");
                Ok(NegotiationResult {
                    chosen: best,
                    method: NegotiationMethod::FallbackHighestConfidence,
                    conflict_detected: true,
                })
            }
        }
    }

    /// T-E-B-18: 思维树多路径综合仲裁。
    ///
    /// 与 [`negotiate_with_arbitration`](Self::negotiate_with_arbitration)
    /// 的关键差异:
    /// * 后者假设多个 Agent 对**同一任务**给出可能冲突的答案,提示
    ///   LLM "select the best candidate or synthesize"(冲突解决语义);
    /// * 本方法假设多个 ThoughtAgent 从**不同思维视角**给出互补的
    ///   推理路径,提示 LLM "synthesize the perspectives into a single
    ///   comprehensive answer"(多视角综合语义)。
    ///
    /// 降级策略:LLM 仲裁失败时回退到 [`highest_confidence`](Self::highest_confidence)
    /// (既有逻辑,与 `negotiate_with_arbitration` 一致)。
    pub async fn negotiate_paths_with_arbitration(
        &self,
        outputs: Vec<AgentOutput>,
        llm: &LlmGateway,
    ) -> Result<NegotiationResult> {
        if outputs.len() <= 1 {
            // 单路径或空输入:直接走既有 negotiate(无冲突可仲裁)。
            let result = self.negotiate(outputs);
            return Ok(result);
        }

        let best = self.highest_confidence(outputs.clone());

        match self.llm_synthesize_paths(outputs, llm).await {
            Ok(chosen) => Ok(NegotiationResult {
                chosen,
                method: NegotiationMethod::LlmArbitration,
                conflict_detected: false,
            }),
            Err(e) => {
                warn!(
                    target: "nebula.negotiator",
                    error = ?e,
                    "ToT path synthesis failed; falling back to highest confidence"
                );
                Ok(NegotiationResult {
                    chosen: best,
                    method: NegotiationMethod::FallbackHighestConfidence,
                    conflict_detected: false,
                })
            }
        }
    }

    pub fn has_conflict(&self, outputs: &[AgentOutput]) -> bool {
        if outputs.len() < 2 {
            return false;
        }
        let bodies: Vec<&str> = outputs.iter().map(|o| o.body.as_str()).collect();
        let first = bodies[0];
        bodies[1..].iter().any(|b| text_similarity(first, b) < 0.5)
    }

    async fn llm_arbitrate(
        &self,
        outputs: Vec<AgentOutput>,
        llm: &LlmGateway,
    ) -> Result<AgentOutput> {
        let candidates: Vec<String> = outputs
            .iter()
            .enumerate()
            .map(|(i, o)| {
                format!(
                    "Candidate {}: [{}] confidence={:.2}\n{}",
                    i + 1,
                    o.kind.as_str(),
                    o.confidence,
                    o.body
                )
            })
            .collect();

        let prompt = format!(
            "You are an arbitration judge. Multiple AI agents produced conflicting results for the same task. \
             Select the best candidate or synthesize the best parts.\n\n\
             {}\n\n\
             Respond with the best answer. Do not explain your choice.",
            candidates.join("\n\n")
        );

        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: prompt,
            ..Default::default()
        }];
        let response = llm.chat(messages).await?;

        Ok(AgentOutput::new(
            super::agents::AgentKind::Generic,
            "arbitrator",
            response.message.content,
        ))
    }

    /// T-E-B-18: 思维树多视角综合 — 调用 LLM 把 N 条互补路径融合为单一答案。
    ///
    /// 与 `llm_arbitrate` 的差异:
    /// * 提示词使用 "synthesize the perspectives"(综合)而非
    ///   "select the best candidate"(选择),引导 LLM 融合而非挑一个;
    /// * 每个候选标注 path_id 与思维视角(来自 path_id),供 LLM
    ///   感知视角多样性;
    /// * 输出 author 为 "synthesizer"(而非 "arbitrator")。
    async fn llm_synthesize_paths(
        &self,
        outputs: Vec<AgentOutput>,
        llm: &LlmGateway,
    ) -> Result<AgentOutput> {
        let perspectives: Vec<String> = outputs
            .iter()
            .map(|o| {
                let path_label = o
                    .path_id
                    .as_deref()
                    .unwrap_or("path-?");
                format!(
                    "[{path_label}] confidence={:.2}\n{}",
                    o.confidence, o.body
                )
            })
            .collect();

        let prompt = format!(
            "You are a synthesis judge. Multiple AI agents approached the same task from different \
             thinking perspectives (analytical / creative / critical / synthesis). \
             Synthesize the perspectives into a single comprehensive answer that combines the \
             strongest insights from each path while reconciling any tensions.\n\n\
             {}\n\n\
             Respond with the synthesized answer. Do not explain your synthesis.",
            perspectives.join("\n\n")
        );

        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: prompt,
            ..Default::default()
        }];
        let response = llm.chat(messages).await?;

        Ok(AgentOutput::new(
            super::agents::AgentKind::Generic,
            "synthesizer",
            response.message.content,
        ))
    }

    pub(crate) fn highest_confidence(&self, outputs: Vec<AgentOutput>) -> AgentOutput {
        // T-E-S-02: 有 tool_calls 的输出,置信度加权 +0.1 boost。
        outputs
            .into_iter()
            .max_by(|a, b| {
                let a_boost = if a.tool_calls.is_some() { 0.1 } else { 0.0 };
                let b_boost = if b.tool_calls.is_some() { 0.1 } else { 0.0 };
                let a_score = a.confidence + a_boost;
                let b_score = b.confidence + b_boost;
                a_score
                    .partial_cmp(&b_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or_else(|| {
                AgentOutput::new(super::agents::AgentKind::Generic, "system", "no outputs")
            })
    }

    // -----------------------------------------------------------------------
    // T-E-S-04: MoA (Mixture of Agents)
    // -----------------------------------------------------------------------

    /// MoA 入口:按 strategy 分派到投票/级联/仲裁逻辑。
    pub async fn negotiate_moa(
        &self,
        prompt: &str,
        config: &MoAConfig,
        gateway: &LlmGateway,
    ) -> Result<AgentOutput> {
        match config.strategy {
            MoAStrategy::Voting => {
                let messages = vec![ChatMessage::user(prompt)];
                let responses = gateway.chat_parallel(&messages, &config.participants).await;
                let scorer = config
                    .scoring_model
                    .as_deref()
                    .unwrap_or_else(|| config.participants.first().map(|s| s.as_str()).unwrap_or("ollama"));
                self.vote_on_responses(prompt, &responses, scorer, gateway)
                    .await
            }
            MoAStrategy::Cascading => {
                self.cascade_responses(prompt, &config.participants, gateway)
                    .await
            }
            MoAStrategy::Arbitration => {
                let messages = vec![ChatMessage::user(prompt)];
                let responses = gateway.chat_parallel(&messages, &config.participants).await;
                // 把 (provider, Result<ChatResponse>) 转成 AgentOutput 列表
                let outputs: Vec<AgentOutput> = responses
                    .into_iter()
                    .filter_map(|(provider, resp)| {
                        resp.ok().map(|r| AgentOutput {
                            kind: super::agents::AgentKind::Generic,
                            author: provider,
                            body: r.message.content,
                            confidence: 0.5,
                            reasoning_chain: Vec::new(),
                            path_id: None,
                            tool_calls: None,
                        })
                    })
                    .collect();
                match self.llm_arbitrate(outputs, gateway).await {
                    Ok(chosen) => Ok(chosen),
                    Err(e) => {
                        warn!(
                            target: "nebula.negotiator",
                            error = ?e,
                            "MoA arbitration failed; returning first response"
                        );
                        // 仲裁失败时,用 gateway.chat 回退到单模型
                        let fallback = gateway.chat(vec![ChatMessage::user(prompt)]).await?;
                        Ok(AgentOutput::new(
                            super::agents::AgentKind::Generic,
                            "moa_fallback",
                            fallback.message.content,
                        ))
                    }
                }
            }
        }
    }

    /// Voting 策略:对每个响应独立评分,取最高分。
    async fn vote_on_responses(
        &self,
        _prompt: &str,
        responses: &[(String, Result<ChatResponse>)],
        scorer: &str,
        gateway: &LlmGateway,
    ) -> Result<AgentOutput> {
        let mut best_output: Option<AgentOutput> = None;
        let mut best_score: u8 = 0;

        for (provider, resp) in responses {
            let resp = match resp {
                Ok(r) => r,
                Err(e) => {
                    warn!(
                        target: "nebula.negotiator",
                        provider = %provider,
                        error = ?e,
                        "MoA participant failed, skipping"
                    );
                    continue;
                }
            };
            let score = self.score_response(&resp.message.content, scorer, gateway).await;
            info!(
                target: "nebula.negotiator",
                provider = %provider,
                score,
                "MoA vote scored"
            );
            if score > best_score || best_output.is_none() {
                best_score = score;
                best_output = Some(AgentOutput {
                    kind: super::agents::AgentKind::Generic,
                    author: provider.clone(),
                    body: resp.message.content.clone(),
                    confidence: score as f32 / 10.0,
                    reasoning_chain: Vec::new(),
                    path_id: None,
                    tool_calls: None,
                });
            }
        }

        Ok(best_output.unwrap_or_else(|| {
            AgentOutput::new(super::agents::AgentKind::Generic, "moa_voting", "no valid responses")
        }))
    }

    /// Cascading 策略:按顺序调用,前一步输出作为后一步上下文。
    async fn cascade_responses(
        &self,
        prompt: &str,
        providers: &[String],
        gateway: &LlmGateway,
    ) -> Result<AgentOutput> {
        let mut accumulated = String::new();
        let mut last_provider = String::new();

        for provider in providers {
            let messages = if accumulated.is_empty() {
                vec![ChatMessage::user(prompt)]
            } else {
                vec![
                    ChatMessage::user(prompt),
                    ChatMessage::assistant(&accumulated),
                    ChatMessage::user("继续深化以上回答,补充遗漏的要点"),
                ]
            };
            match gateway.chat_with_provider(&messages, provider).await {
                Ok(resp) => {
                    accumulated = resp.message.content;
                    last_provider = provider.clone();
                }
                Err(e) => {
                    warn!(
                        target: "nebula.negotiator",
                        provider = %provider,
                        error = ?e,
                        "MoA cascade step failed, keeping previous output"
                    );
                }
            }
        }

        Ok(AgentOutput {
            kind: super::agents::AgentKind::Generic,
            author: last_provider,
            body: accumulated,
            confidence: 0.7,
            reasoning_chain: Vec::new(),
            path_id: None,
            tool_calls: None,
        })
    }

    /// 用评分模型对单个响应打分(1-10)。
    async fn score_response(
        &self,
        response_text: &str,
        scorer: &str,
        gateway: &LlmGateway,
    ) -> u8 {
        // R2: 评分用固定模板,不拼接用户输入
        let scoring_prompt = format!(
            "Rate this response on a scale of 1-10 based on clarity, accuracy, and completeness. \
             Respond with only a single number.\n\nResponse:\n{}",
            response_text
        );
        let messages = vec![ChatMessage::user(&scoring_prompt)];
        let result = gateway.chat_with_provider(&messages, scorer).await;
        match result {
            Ok(resp) => Self::extract_score(&resp.message.content),
            Err(e) => {
                warn!(
                    target: "nebula.negotiator",
                    scorer = %scorer,
                    error = ?e,
                    "MoA scoring failed, defaulting to 5"
                );
                5
            }
        }
    }

    /// 从 LLM 输出中提取 1-10 的数字评分。
    fn extract_score(text: &str) -> u8 {
        // 评分 prompt 要求 LLM "Respond with only a single number",
        // 但模型可能返回 "7/10" 或 "Score: 8" 等格式。
        // 策略:取第一个 1-10 范围内的数字作为评分。
        for token in text.split_whitespace() {
            if let Ok(n) = token.parse::<u8>() {
                if (1..=10).contains(&n) {
                    return n;
                }
            }
        }
        5 // 默认分数
    }
}

impl Default for Negotiator {
    fn default() -> Self {
        Self::new()
    }
}

fn text_similarity(a: &str, b: &str) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let a_words: std::collections::HashSet<&str> = a.split_whitespace().collect();
    let b_words: std::collections::HashSet<&str> = b.split_whitespace().collect();
    let intersection = a_words.intersection(&b_words).count();
    let union = a_words.union(&b_words).count();
    if union == 0 {
        return 1.0;
    }
    intersection as f32 / union as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_conflict_single_output() {
        let n = Negotiator::new();
        let outputs = vec![AgentOutput::new(
            super::super::agents::AgentKind::Generic,
            "a",
            "hello",
        )];
        let result = n.negotiate(outputs);
        assert!(!result.conflict_detected);
    }

    #[test]
    fn high_confidence_wins() {
        let n = Negotiator::new();
        let outputs = vec![
            AgentOutput {
                kind: super::super::agents::AgentKind::Generic,
                author: "a".into(),
                body: "answer a".into(),
                confidence: 0.9,
                reasoning_chain: Vec::new(),
                path_id: None,
                tool_calls: None,
            },
            AgentOutput {
                kind: super::super::agents::AgentKind::Generic,
                author: "b".into(),
                body: "answer b".into(),
                confidence: 0.5,
                reasoning_chain: Vec::new(),
                path_id: None,
                tool_calls: None,
            },
        ];
        let result = n.negotiate(outputs);
        assert_eq!(result.chosen.author, "a");
    }

    #[test]
    fn text_similarity_identical() {
        assert_eq!(text_similarity("hello world", "hello world"), 1.0);
    }

    #[test]
    fn text_similarity_different() {
        assert!(text_similarity("cat dog", "fish bird") < 0.3);
    }

    // T-E-B-18: negotiate_paths_with_arbitration 测试。

    #[test]
    fn negotiate_paths_single_output_short_circuits() {
        // 单路径输入:走既有 negotiate(无冲突可仲裁)。
        let n = Negotiator::new();
        let outputs = vec![AgentOutput::new(
            super::super::agents::AgentKind::Generic,
            "a",
            "only path",
        )];
        let rt = tokio::runtime::Runtime::new().unwrap();
        let llm = std::sync::Arc::new(crate::llm::LlmGateway::new_test());
        let result = rt
            .block_on(n.negotiate_paths_with_arbitration(outputs, &llm))
            .unwrap();
        assert!(!result.conflict_detected, "single path has no conflict");
        assert_eq!(result.chosen.author, "a");
    }

    #[tokio::test]
    async fn negotiate_paths_falls_back_to_highest_confidence_on_llm_failure() {
        // LLM 端点不可达 → llm_synthesize_paths 失败 → 回退 highest_confidence。
        // 使用指向不存在的本地端点的 OllamaClient,确保 chat() 立即失败。
        let client = std::sync::Arc::new(crate::llm::OllamaClient::new_with_timeout(
            "http://127.0.0.1:1",
            std::time::Duration::from_millis(50),
        ));
        let gw = std::sync::Arc::new(crate::llm::LlmGateway::new(
            client,
            "m",
            "ollama",
            None,
            None,
            None,
            None,
            None,
        ));
        let n = Negotiator::new();
        let outputs = vec![
            AgentOutput {
                kind: super::super::agents::AgentKind::Generic,
                author: "a".into(),
                body: "analytical answer".into(),
                confidence: 0.9,
                reasoning_chain: Vec::new(),
                path_id: Some("path-0".into()),
                tool_calls: None,
            },
            AgentOutput {
                kind: super::super::agents::AgentKind::Generic,
                author: "b".into(),
                body: "creative answer".into(),
                confidence: 0.5,
                reasoning_chain: Vec::new(),
                path_id: Some("path-1".into()),
                tool_calls: None,
            },
        ];
        let result = n
            .negotiate_paths_with_arbitration(outputs, &gw)
            .await
            .unwrap();
        // 仲裁失败 → 回退 highest_confidence。
        assert_eq!(
            result.method,
            NegotiationMethod::FallbackHighestConfidence,
            "LLM failure must fall back to highest confidence"
        );
        // 选中的应是 confidence 最高的 a(0.9)。
        assert_eq!(result.chosen.author, "a");
        assert_eq!(result.chosen.path_id.as_deref(), Some("path-0"));
        // 综合模式不应标记冲突(视角互补,不是冲突)。
        assert!(!result.conflict_detected);
    }

    #[tokio::test]
    async fn negotiate_paths_falls_back_when_empty() {
        // 空输入:走 negotiate 的兜底逻辑(返回 "no outputs to negotiate")。
        let n = Negotiator::new();
        let llm = std::sync::Arc::new(crate::llm::LlmGateway::new_test());
        let outputs: Vec<AgentOutput> = Vec::new();
        let result = n
            .negotiate_paths_with_arbitration(outputs, &llm)
            .await
            .unwrap();
        assert!(!result.conflict_detected);
        assert_eq!(result.chosen.body, "no outputs to negotiate");
    }

    // -----------------------------------------------------------------------
    // T-E-S-04: MoA (Mixture of Agents) 测试
    // -----------------------------------------------------------------------

    /// test_moa_config_serialization: MoAConfig 序列化 round-trip。
    #[test]
    fn test_moa_config_serialization() {
        let config = MoAConfig {
            participants: vec!["ollama:qwen2.5:3b".into(), "deepseek:deepseek-chat".into()],
            strategy: MoAStrategy::Voting,
            scoring_model: Some("deepseek:deepseek-chat".into()),
        };
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: MoAConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.participants, config.participants);
        assert_eq!(deserialized.strategy, MoAStrategy::Voting);
        assert_eq!(deserialized.scoring_model, config.scoring_model);

        // 测试 Cascading 策略序列化
        let config_cascading = MoAConfig {
            participants: vec!["ollama".into()],
            strategy: MoAStrategy::Cascading,
            scoring_model: None,
        };
        let json2 = serde_json::to_string(&config_cascading).unwrap();
        let deser2: MoAConfig = serde_json::from_str(&json2).unwrap();
        assert_eq!(deser2.strategy, MoAStrategy::Cascading);
        assert!(deser2.scoring_model.is_none());
    }

    /// test_moa_strategy_voting: 3 个 mock 响应 → 投票选最高分。
    ///
    /// 直接调用 vote_on_responses,注入预设的 (provider, Result<ChatResponse>) 列表,
    /// 验证评分后选中得分最高的响应。由于评分调用 LLM(测试环境无真实 LLM),
    /// 评分默认回退 5 分,因此所有响应得分相同 — 取第一个。
    #[tokio::test]
    async fn test_moa_strategy_voting() {
        let n = Negotiator::new();
        // 构造 3 个 mock 响应
        let responses: Vec<(String, Result<crate::llm::ChatResponse>)> = vec![
            (
                "provider_a".into(),
                Ok(crate::llm::ChatResponse {
                    model: "a".into(),
                    message: crate::llm::ChatMessage {
                        role: "assistant".into(),
                        content: "Good answer from A".into(),
                        ..Default::default()
                    },
                    done: true,
                    total_duration: None,
                    eval_count: None,
                    ..Default::default()
                }),
            ),
            (
                "provider_b".into(),
                Ok(crate::llm::ChatResponse {
                    model: "b".into(),
                    message: crate::llm::ChatMessage {
                        role: "assistant".into(),
                        content: "Excellent answer from B".into(),
                        ..Default::default()
                    },
                    done: true,
                    total_duration: None,
                    eval_count: None,
                    ..Default::default()
                }),
            ),
            (
                "provider_c".into(),
                Ok(crate::llm::ChatResponse {
                    model: "c".into(),
                    message: crate::llm::ChatMessage {
                        role: "assistant".into(),
                        content: "Poor answer from C".into(),
                        ..Default::default()
                    },
                    done: true,
                    total_duration: None,
                    eval_count: None,
                    ..Default::default()
                }),
            ),
        ];

        // 使用测试 gateway(chat_with_provider 会失败,评分默认 5)
        let gw = std::sync::Arc::new(crate::llm::LlmGateway::new_test());
        let result = n
            .vote_on_responses("test prompt", &responses, "ollama", &gw)
            .await
            .unwrap();
        // 所有评分都回退到 5,取第一个(provider_a)
        assert_eq!(result.author, "provider_a");
        assert_eq!(result.body, "Good answer from A");
        // 5/10 = 0.5
        assert!((result.confidence - 0.5).abs() < 1e-6);
    }

    /// test_moa_strategy_cascading: 2 个 provider → 第二个接收第一个的输出。
    ///
    /// 由于测试环境无真实 LLM,cascade 每步都会失败(使用不可达端点),
    /// 验证空输出时的降级行为。
    #[tokio::test]
    async fn test_moa_strategy_cascading() {
        let n = Negotiator::new();
        // 使用不可达端点,chat_with_provider 会失败
        let client = std::sync::Arc::new(crate::llm::OllamaClient::new_with_timeout(
            "http://127.0.0.1:1",
            std::time::Duration::from_millis(50),
        ));
        let gw = std::sync::Arc::new(crate::llm::LlmGateway::new(
            client,
            "m",
            "ollama",
            None,
            None,
            None,
            None,
            None,
        ));
        let providers = vec!["ollama".into(), "ollama".into()];
        let result = n
            .cascade_responses("test prompt", &providers, &gw)
            .await
            .unwrap();
        // 所有 provider 失败 → accumulated 保持空
        assert!(result.body.is_empty());
    }

    /// test_moa_strategy_arbitration: 3 个 mock 响应 → LLM 仲裁。
    ///
    /// 由于测试环境无真实 LLM,仲裁失败后回退到 gateway.chat()。
    /// 使用不可达端点验证仲裁降级行为。
    #[tokio::test]
    async fn test_moa_strategy_arbitration() {
        let n = Negotiator::new();
        let client = std::sync::Arc::new(crate::llm::OllamaClient::new_with_timeout(
            "http://127.0.0.1:1",
            std::time::Duration::from_millis(50),
        ));
        let gw = std::sync::Arc::new(crate::llm::LlmGateway::new(
            client,
            "m",
            "ollama",
            None,
            None,
            None,
            None,
            None,
        ));
        let config = MoAConfig {
            participants: vec!["ollama".into(), "ollama".into(), "ollama".into()],
            strategy: MoAStrategy::Arbitration,
            scoring_model: None,
        };
        // 仲裁和 fallback chat 都会失败(不可达端点)
        let result = n.negotiate_moa("test", &config, &gw).await;
        // 整个 MoA 链路失败,返回 Err
        assert!(result.is_err(), "arbitration with unreachable LLM should fail");
    }

    /// test_chat_parallel: 2 个 provider 并行 → 返回 2 个结果。
    ///
    /// 使用不可达端点,两个 provider 都失败,但 chat_parallel
    /// 应返回 2 个(非零)结果(每个都是 Err)。
    #[tokio::test]
    async fn test_chat_parallel() {
        let client = std::sync::Arc::new(crate::llm::OllamaClient::new_with_timeout(
            "http://127.0.0.1:1",
            std::time::Duration::from_millis(50),
        ));
        let gw = std::sync::Arc::new(crate::llm::LlmGateway::new(
            client,
            "m",
            "ollama",
            None,
            None,
            None,
            None,
            None,
        ));
        let providers = vec!["ollama".into(), "ollama".into()];
        let messages = vec![crate::llm::ChatMessage::user("hello")];
        let results = gw.chat_parallel(&messages, &providers).await;
        assert_eq!(results.len(), 2, "should return 2 results for 2 providers");
        // 两个都应该是 Err(不可达端点)
        for (provider, result) in &results {
            assert!(result.is_err(), "provider {} should fail", provider);
        }
    }

    /// test_vote_scoring: 评分 prompt 解析 + 分数提取。
    #[test]
    fn test_vote_scoring() {
        // 正常数字
        assert_eq!(Negotiator::extract_score("8"), 8);
        assert_eq!(Negotiator::extract_score("10"), 10);
        assert_eq!(Negotiator::extract_score("1"), 1);

        // 数字在句子中
        assert_eq!(Negotiator::extract_score("I rate this a 7"), 7);
        // "9 out of 10" → 第一个合法数字是 9
        assert_eq!(Negotiator::extract_score("Score: 9 out of 10"), 9);

        // 多个数字取第一个合法评分
        assert_eq!(Negotiator::extract_score("5 8 3"), 5);

        // 超出范围
        assert_eq!(Negotiator::extract_score("0"), 5);  // 0 不在 1-10 范围内
        assert_eq!(Negotiator::extract_score("15"), 5); // 15 不在 1-10 范围内

        // 无数字
        assert_eq!(Negotiator::extract_score("no score here"), 5);

        // 空字符串
        assert_eq!(Negotiator::extract_score(""), 5);

        // 浮点数(无法解析为 u8,回退默认)
        assert_eq!(Negotiator::extract_score("7.5"), 5);
    }

    // T-E-S-02: Negotiator 感知 tool_calls 测试。
    //
    // 两个 Agent: 一个有 tool_calls(boost +0.1),一个没有。
    // 即使 tool_calls Agent 的 confidence 较低,boost 后应被偏好。

    #[test]
    fn test_negotiator_aware_of_tool_calls() {
        use crate::swarm::tool_types::ToolCall;

        let n = Negotiator::new();
        let outputs = vec![
            AgentOutput {
                kind: super::super::agents::AgentKind::Generic,
                author: "with_tools".into(),
                body: "I'll use a tool".into(),
                confidence: 0.7,
                reasoning_chain: Vec::new(),
                path_id: None,
                tool_calls: Some(vec![ToolCall {
                    id: "call_1".to_string(),
                    function_name: "read_file".to_string(),
                    arguments: serde_json::json!({"path": "/tmp/test.txt"}),
                }]),
            },
            AgentOutput {
                kind: super::super::agents::AgentKind::Generic,
                author: "no_tools".into(),
                body: "I have no tools".into(),
                confidence: 0.75,
                reasoning_chain: Vec::new(),
                path_id: None,
                tool_calls: None,
            },
        ];

        let result = n.highest_confidence(outputs);
        // with_tools: 0.7 + 0.1 = 0.8 > no_tools: 0.75
        assert_eq!(
            result.author, "with_tools",
            "Agent with tool_calls should win due to +0.1 boost"
        );
    }
}
