/**
 * T-E-S-13: 工作流模板类型定义。
 *
 * 定义预置工作流模板的接口、分类枚举与 i18n 结构。
 * 与现有 WorkflowNode / WorkflowEdge 类型(types.ts)完全兼容,
 * 模板可直接被画布加载为 WorkflowDocument。
 */
import type { WorkflowNode, WorkflowEdge } from '../types';
import type { Locale } from '../../../i18n';

/** 模板分类。 */
export type TemplateCategory =
  | 'research'
  | 'writing'
  | 'coding'
  | 'review'
  | 'translation'
  | 'data_analysis';

/** 模板 i18n 条目:每种语言下的名称与描述。 */
export interface TemplateI18nEntry {
  /** 显示名称。 */
  name: string;
  /** 模板描述。 */
  description: string;
}

/** 模板 i18n 字典:键为 Locale,值为对应文案。 */
export type TemplateI18n = Partial<Record<Locale, TemplateI18nEntry>>;

/** 模板默认值:用户实例化模板时可填充的占位参数。 */
export interface TemplateDefaultValues {
  [key: string]: string | number | boolean;
}

/**
 * 预置工作流模板。
 *
 * - id: 模板唯一标识(snake_case)。
 * - name / description: 英文回退文案(与 i18n.en-US 一致)。
 * - i18n: 中英双语文案键(至少包含 zh-CN / en-US)。
 * - nodes / edges: 与 WorkflowDocument 兼容的节点与连线。
 * - default_values: 占位默认值(如 topic / language 等)。
 */
export interface WorkflowTemplate {
  /** 模板唯一标识(snake_case)。 */
  id: string;
  /** 显示名称(英文回退)。 */
  name: string;
  /** 模板描述(英文回退)。 */
  description: string;
  /** 模板分类。 */
  category: TemplateCategory;
  /** 中英双语文案。 */
  i18n: TemplateI18n;
  /** 节点列表(与 WorkflowNode 兼容)。 */
  nodes: WorkflowNode[];
  /** 连线列表(与 WorkflowEdge 兼容)。 */
  edges: WorkflowEdge[];
  /** 占位默认值。 */
  default_values: TemplateDefaultValues;
}

/** 标准画布节点水平间距(像素),用于模板布局。 */
export const TEMPLATE_NODE_GAP_X = 280;
/** 标准画布节点垂直基线(像素),用于模板布局。 */
export const TEMPLATE_NODE_Y = 240;
/** 节点宽度(与 types.ts 的 NODE_WIDTH 一致)。 */
export const TEMPLATE_NODE_WIDTH = 180;
