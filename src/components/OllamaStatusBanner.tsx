/**
 * v2.2: Ollama 不再是强制依赖。
 *
 * 后端 LLM 网关支持多 provider（DeepSeek 优先 → Ollama 兜底 → Anthropic/远程兼容），
 * 前端不再强制要求本地 Ollama 守护进程。此组件保留为空壳以维持 ChatPanel/FloatingChat
 * 的 import 兼容性，后续可在彻底移除引用后删除。
 */
export function OllamaStatusBanner() {
  return null;
}
