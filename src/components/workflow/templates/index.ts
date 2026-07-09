/**
 * T-E-S-13: 工作流模板库入口。
 *
 * 汇总所有预置模板,并提供按 id / category 查询、按 locale 取文案、
 * 实例化为 WorkflowDocument 等纯函数。
 *
 * 约定:
 * - 所有函数均为纯函数,便于单元测试与 reducer 复用。
 * - 模板节点 id 使用稳定前缀(r-/w-/c-/v-/t-/d-),便于在画布上引用。
 */
import type { Locale } from '../../../i18n';
import type { WorkflowDocument, WorkflowNode, WorkflowEdge } from '../types';
import type { WorkflowTemplate, TemplateCategory, TemplateI18nEntry } from './types';
import { researchTemplate } from './research';
import { writingTemplate } from './writing';
import { codingTemplate } from './coding';
import { reviewTemplate } from './review';
import { translationTemplate } from './translation';
import { dataAnalysisTemplate } from './data_analysis';

/** 所有预置模板(顺序即展示顺序)。 */
export const TEMPLATES: readonly WorkflowTemplate[] = [
  researchTemplate,
  writingTemplate,
  codingTemplate,
  reviewTemplate,
  translationTemplate,
  dataAnalysisTemplate,
] as const;

/** 按 id 查找模板,未命中返回 null。 */
export function getTemplateById(id: string): WorkflowTemplate | null {
  return TEMPLATES.find((t) => t.id === id) ?? null;
}

/** 按分类筛选模板。 */
export function getTemplatesByCategory(category: TemplateCategory): WorkflowTemplate[] {
  return TEMPLATES.filter((t) => t.category === category);
}

/**
 * 取模板在指定 locale 下的文案,缺失时回退到 en-US,再回退到模板顶层 name/description。
 */
export function getTemplateI18n(
  template: WorkflowTemplate,
  locale: Locale
): TemplateI18nEntry {
  return (
    template.i18n[locale] ??
    template.i18n['en-US'] ?? {
      name: template.name,
      description: template.description,
    }
  );
}

/** 所有出现过的分类(去重)。 */
export function allCategories(): TemplateCategory[] {
  const seen = new Set<TemplateCategory>();
  for (const t of TEMPLATES) seen.add(t.category);
  return [...seen];
}

/**
 * 将模板深拷贝为一份可独立编辑的 WorkflowDocument。
 *
 * - 复制节点 / 边数组(避免外部修改污染模板常量)。
 * - 用 `seq` 前缀重写节点 / 边 id,防止与画布现有节点冲突。
 * - 设置文档 name 为模板名,updated_at 为当前时间。
 */
export function instantiateTemplate(
  template: WorkflowTemplate,
  docId: string
): WorkflowDocument {
  const prefix = docId.slice(0, 8);
  const nodes: WorkflowNode[] = template.nodes.map((n, i) => ({
    ...n,
    id: `${prefix}-${i}-${n.id}`,
    config: { ...n.config },
  }));
  // 建立 旧 id → 新 id 映射,用于重写边的 source/target。
  const idMap = new Map<string, string>();
  template.nodes.forEach((n, i) => {
    idMap.set(n.id, `${prefix}-${i}-${n.id}`);
  });
  const edges: WorkflowEdge[] = template.edges.map((e, i) => ({
    ...e,
    id: `${prefix}-e${i}`,
    source: idMap.get(e.source) ?? e.source,
    target: idMap.get(e.target) ?? e.target,
  }));
  return {
    id: docId,
    name: template.name,
    nodes,
    edges,
    updated_at: Date.now(),
  };
}

// ---- 重导出,便于外部按需引入 ----
export type {
  WorkflowTemplate,
  TemplateCategory,
  TemplateI18n,
  TemplateI18nEntry,
  TemplateDefaultValues,
} from './types';
export { researchTemplate } from './research';
export { writingTemplate } from './writing';
export { codingTemplate } from './coding';
export { reviewTemplate } from './review';
export { translationTemplate } from './translation';
export { dataAnalysisTemplate } from './data_analysis';
