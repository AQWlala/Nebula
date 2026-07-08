/**
 * Markdown 渲染工具。
 *
 * T-D-F-03: 从 ChatPanel.tsx 和 KnowledgeCardDialog.tsx 提取的共用函数。
 * 消除两处完全相同的 renderMarkdown 实现(逻辑一致,注释各异)。
 *
 * 流程:
 * 1. 预处理 `[[xxx]]` → `<a class="wiki-link" data-slug="xxx">xxx</a>`(可点击)。
 * 2. `marked.parse` 转 HTML。
 * 3. `DOMPurify.sanitize` 清理 XSS(移除 `<script>` / 内联事件处理器等)。
 */

import { marked } from 'marked';
import DOMPurify from 'dompurify';

/**
 * 将 Markdown 文本渲染为经过 XSS 清理的 HTML。
 *
 * 预处理 `[[wiki-link]]` 语法为可点击的 `<a>` 标签,
 * 然后用 marked 解析,最后用 DOMPurify 清理。
 *
 * @param content Markdown 文本
 * @returns 清理后的 HTML 字符串
 */
export function renderMarkdown(content: string): string {
  const withWikiLinks = content.replace(
    /\[\[([^\]]+)\]\]/g,
    (_, s) => `<a class="wiki-link" data-slug="${s}">${s}</a>`,
  );
  const html = marked.parse(withWikiLinks) as string;
  return DOMPurify.sanitize(html, {
    // 允许 <a> 的 data-slug 属性(用于 wiki-link 点击检测)。
    ADD_ATTR: ['data-slug', 'target'],
  });
}
