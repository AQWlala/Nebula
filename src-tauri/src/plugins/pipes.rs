//! T-E-S-35: Pipe 层示例 — `ContextInjectionPipe`。
//!
//! 同步转换管道:在用户 prompt 前注入一段系统上下文字符串。
//! 这是 `assemble_context` 等既有散落逻辑的统一 trait 化示例。
//! P0 用同步 `transform`;流式 Stream 留 P1。

use anyhow::Result;

use super::traits::{Pipe, PipeInput, PipeOutput};

/// 在 prompt 前注入固定系统上下文的 Pipe。
///
/// 若 `context` 为空,等价于直通(保留原 prompt 与 context 字段)。
pub struct ContextInjectionPipe {
    context: String,
}

impl ContextInjectionPipe {
    pub fn new(context: impl Into<String>) -> Self {
        Self {
            context: context.into(),
        }
    }
}

impl Pipe for ContextInjectionPipe {
    fn name(&self) -> &str {
        "context_injection"
    }

    fn transform(&self, input: PipeInput) -> Result<PipeOutput> {
        if self.context.is_empty() {
            return Ok(PipeOutput {
                prompt: input.prompt,
                context: input.context,
            });
        }
        // 上下文在前,用户 prompt 在后,中间空行分隔。
        let prompt = format!("{}\n\n{}", self.context, input.prompt);
        Ok(PipeOutput {
            prompt,
            context: input.context,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepends_context_to_prompt() {
        let pipe = ContextInjectionPipe::new("You are a helpful assistant.");
        let out = pipe
            .transform(PipeInput::new("What is 2+2?"))
            .expect("create should succeed");
        assert_eq!(out.prompt, "You are a helpful assistant.\n\nWhat is 2+2?");
    }

    #[test]
    fn empty_context_passes_through() {
        let pipe = ContextInjectionPipe::new("");
        let out = pipe
            .transform(PipeInput::new("hello"))
            .expect("create should succeed");
        assert_eq!(out.prompt, "hello");
    }

    #[test]
    fn preserves_context_field() {
        let mut input = PipeInput::new("q");
        input.context = Some("prior".into());
        let pipe = ContextInjectionPipe::new("SYS");
        let out = pipe.transform(input).expect("test op should succeed");
        assert_eq!(out.context.as_deref(), Some("prior"));
        assert!(out.prompt.starts_with("SYS\n\nq"));
    }
}
