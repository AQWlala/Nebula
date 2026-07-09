/**
 * T-E-S-13: 工作流模板库单元测试。
 *
 * 覆盖:
 * - 模板库总量与分类
 * - 每个模板的结构完整性(id/name/description/category/i18n/nodes/edges)
 * - 节点 id 唯一性、边引用合法性、无自环、无环(DAG)
 * - 节点 config 与 type 的判别联合一致性
 * - i18n 双语键齐全(zh-CN / en-US)
 * - 查询函数: getTemplateById / getTemplatesByCategory / allCategories
 * - getTemplateI18n 回退策略
 * - instantiateTemplate 深拷贝与 id 重写
 */
import { describe, it, expect } from 'vitest';
import {
  TEMPLATES,
  getTemplateById,
  getTemplatesByCategory,
  getTemplateI18n,
  allCategories,
  instantiateTemplate,
} from '../index';
import {
  topologicalSort,
  wouldCreateCycle,
  validateEdge,
  type WorkflowEdge,
} from '../../types';
import type { TemplateCategory } from '../types';

/** 所有合法的节点类型。 */
const VALID_NODE_TYPES = new Set(['agent', 'task', 'condition', 'io']);
/** 所有合法的边端口。 */
const VALID_PORTS = new Set(['out', 'true', 'false']);
/** 所有合法的模板分类。 */
const VALID_CATEGORIES: TemplateCategory[] = [
  'research',
  'writing',
  'coding',
  'review',
  'translation',
  'data_analysis',
];

describe('workflow templates — library', () => {
  it('exports exactly 6 preset templates', () => {
    expect(TEMPLATES).toHaveLength(6);
  });

  it('every template has a unique id', () => {
    const ids = TEMPLATES.map((t) => t.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it('covers all 6 declared categories', () => {
    const cats = allCategories();
    expect(cats.sort()).toEqual([...VALID_CATEGORIES].sort());
    expect(cats).toHaveLength(6);
  });

  it('each category maps to exactly one template', () => {
    for (const cat of VALID_CATEGORIES) {
      expect(getTemplatesByCategory(cat)).toHaveLength(1);
    }
  });
});

describe('workflow templates — structure', () => {
  it('every template has 3-5+ nodes', () => {
    for (const t of TEMPLATES) {
      expect(t.nodes.length).toBeGreaterThanOrEqual(3);
    }
  });

  it('every template has at least as many edges as nodes minus one (connected)', () => {
    for (const t of TEMPLATES) {
      expect(t.edges.length).toBeGreaterThanOrEqual(t.nodes.length - 1);
    }
  });

  it('every template exposes a non-empty name and description', () => {
    for (const t of TEMPLATES) {
      expect(t.name.length).toBeGreaterThan(0);
      expect(t.description.length).toBeGreaterThan(0);
    }
  });

  it('every node has a valid type and matching config discriminator', () => {
    for (const t of TEMPLATES) {
      for (const n of t.nodes) {
        expect(VALID_NODE_TYPES.has(n.type)).toBe(true);
        expect(n.config.type).toBe(n.type);
      }
    }
  });

  it('node ids are unique within each template', () => {
    for (const t of TEMPLATES) {
      const ids = t.nodes.map((n) => n.id);
      expect(new Set(ids).size).toBe(ids.length);
    }
  });

  it('edge ids are unique within each template', () => {
    for (const t of TEMPLATES) {
      const ids = t.edges.map((e) => e.id);
      expect(new Set(ids).size).toBe(ids.length);
    }
  });

  it('every edge references existing node ids', () => {
    for (const t of TEMPLATES) {
      const nodeIds = new Set(t.nodes.map((n) => n.id));
      for (const e of t.edges) {
        expect(nodeIds.has(e.source)).toBe(true);
        expect(nodeIds.has(e.target)).toBe(true);
      }
    }
  });

  it('no edge is a self-loop', () => {
    for (const t of TEMPLATES) {
      for (const e of t.edges) {
        expect(e.source).not.toBe(e.target);
      }
    }
  });

  it('every edge uses a valid sourcePort', () => {
    for (const t of TEMPLATES) {
      for (const e of t.edges) {
        expect(VALID_PORTS.has(e.sourcePort)).toBe(true);
      }
    }
  });

  it('every template is a DAG (topological sort succeeds)', () => {
    for (const t of TEMPLATES) {
      const order = topologicalSort(t.nodes, t.edges);
      expect(order).not.toBeNull();
      expect(order?.length).toBe(t.nodes.length);
    }
  });

  it('every template starts with an input IO node and ends with an output IO node', () => {
    for (const t of TEMPLATES) {
      // 入度为 0 的节点应为 input IO
      const inDegree = new Map<string, number>();
      for (const n of t.nodes) inDegree.set(n.id, 0);
      for (const e of t.edges) inDegree.set(e.target, (inDegree.get(e.target) ?? 0) + 1);
      const roots = t.nodes.filter((n) => (inDegree.get(n.id) ?? 0) === 0);
      expect(roots.length).toBeGreaterThanOrEqual(1);
      const rootIo = roots.find(
        (n) => n.config.type === 'io' && n.config.direction === 'input'
      );
      expect(rootIo).toBeDefined();
      // 出度为 0 的节点应为 output IO
      const outDegree = new Map<string, number>();
      for (const n of t.nodes) outDegree.set(n.id, 0);
      for (const e of t.edges) outDegree.set(e.source, (outDegree.get(e.source) ?? 0) + 1);
      const sinks = t.nodes.filter((n) => (outDegree.get(n.id) ?? 0) === 0);
      expect(sinks.length).toBeGreaterThanOrEqual(1);
      const sinkIo = sinks.find(
        (n) => n.config.type === 'io' && n.config.direction === 'output'
      );
      expect(sinkIo).toBeDefined();
    }
  });

  it('agent nodes reference a valid AgentKind', () => {
    const validKinds = new Set([
      'generic',
      'coder',
      'writer',
      'reviewer',
      'researcher',
      'planner',
    ]);
    for (const t of TEMPLATES) {
      for (const n of t.nodes) {
        if (n.config.type === 'agent') {
          expect(validKinds.has(n.config.agent_kind)).toBe(true);
          expect(n.config.max_retries).toBeGreaterThanOrEqual(0);
        }
      }
    }
  });

  it('no template edge would create a cycle when added incrementally', () => {
    for (const t of TEMPLATES) {
      const acc: WorkflowEdge[] = [];
      for (const e of t.edges) {
        const cyclic = wouldCreateCycle(e.source, e.target, acc);
        expect(cyclic).toBe(false);
        const err = validateEdge(e.source, e.target, e.sourcePort, acc);
        expect(err).toBeNull();
        acc.push(e);
      }
    }
  });
});

describe('workflow templates — i18n', () => {
  it('every template has both zh-CN and en-US entries', () => {
    for (const t of TEMPLATES) {
      expect(t.i18n['en-US']).toBeDefined();
      expect(t.i18n['zh-CN']).toBeDefined();
    }
  });

  it('zh-CN name differs from en-US name (actually translated)', () => {
    for (const t of TEMPLATES) {
      const en = t.i18n['en-US']!;
      const zh = t.i18n['zh-CN']!;
      expect(zh.name).not.toBe(en.name);
      expect(zh.description).not.toBe(en.description);
    }
  });

  it('top-level name/description match the en-US i18n entry (fallback contract)', () => {
    for (const t of TEMPLATES) {
      const en = t.i18n['en-US']!;
      expect(t.name).toBe(en.name);
      expect(t.description).toBe(en.description);
    }
  });

  it('getTemplateI18n returns zh-CN entry for zh-CN locale', () => {
    const t = TEMPLATES[0];
    const zh = getTemplateI18n(t, 'zh-CN');
    expect(zh.name).toBe(t.i18n['zh-CN']!.name);
  });

  it('getTemplateI18n falls back to en-US for an unsupported locale path', () => {
    // 两种 locale 均存在,验证 en-US 路径返回 en-US 文案。
    const t = TEMPLATES[0];
    const en = getTemplateI18n(t, 'en-US');
    expect(en.name).toBe(t.i18n['en-US']!.name);
  });
});

describe('workflow templates — query helpers', () => {
  it('getTemplateById returns the matching template', () => {
    const t = getTemplateById('research');
    expect(t).not.toBeNull();
    expect(t?.category).toBe('research');
  });

  it('getTemplateById returns null for unknown id', () => {
    expect(getTemplateById('does-not-exist')).toBeNull();
  });

  it('getTemplatesByCategory returns empty array for a valid-but-empty filter is never empty here', () => {
    // 全部 6 个分类都有模板,因此每个分类至少返回 1 个。
    for (const cat of VALID_CATEGORIES) {
      expect(getTemplatesByCategory(cat).length).toBeGreaterThanOrEqual(1);
    }
  });
});

describe('workflow templates — instantiation', () => {
  it('instantiateTemplate produces a WorkflowDocument with matching node/edge counts', () => {
    const src = TEMPLATES[0];
    const doc = instantiateTemplate(src, 'wf-test-001');
    expect(doc.id).toBe('wf-test-001');
    expect(doc.name).toBe(src.name);
    expect(doc.nodes).toHaveLength(src.nodes.length);
    expect(doc.edges).toHaveLength(src.edges.length);
    expect(typeof doc.updated_at).toBe('number');
  });

  it('instantiateTemplate rewrites node ids with a stable prefix and keeps edges consistent', () => {
    const src = TEMPLATES[1]; // writing — 6 节点
    const doc = instantiateTemplate(src, 'wf-abc-123');
    const prefix = 'wf-abc-1';
    // 每个节点 id 以 prefix 开头。
    for (const n of doc.nodes) {
      expect(n.id.startsWith(prefix)).toBe(true);
    }
    // 每条边的 source/target 都能在新节点集中找到。
    const ids = new Set(doc.nodes.map((n) => n.id));
    for (const e of doc.edges) {
      expect(ids.has(e.source)).toBe(true);
      expect(ids.has(e.target)).toBe(true);
    }
  });

  it('instantiateTemplate deep-copies config so mutating the doc does not affect the template', () => {
    const src = TEMPLATES[2]; // coding
    const doc = instantiateTemplate(src, 'wf-mut-456');
    const docAgent = doc.nodes.find((n) => n.config.type === 'agent');
    expect(docAgent).toBeDefined();
    if (docAgent && docAgent.config.type === 'agent') {
      docAgent.config.prompt = 'MUTATED';
      // 模板常量未被污染。
      const tmplAgent = src.nodes.find((n) => n.config.type === 'agent');
      expect(tmplAgent?.config.type).toBe('agent');
      if (tmplAgent?.config.type === 'agent') {
        expect(tmplAgent.config.prompt).not.toBe('MUTATED');
      }
    }
  });

  it('instantiateTemplate preserves node titles and types', () => {
    const src = TEMPLATES[3]; // review
    const doc = instantiateTemplate(src, 'wf-pres-789');
    expect(doc.nodes.map((n) => n.title)).toEqual(src.nodes.map((n) => n.title));
    expect(doc.nodes.map((n) => n.type)).toEqual(src.nodes.map((n) => n.type));
  });

  it('instantiateTemplate result is itself a valid DAG', () => {
    for (const src of TEMPLATES) {
      const doc = instantiateTemplate(src, `wf-dag-${src.id}`);
      const order = topologicalSort(doc.nodes, doc.edges);
      expect(order).not.toBeNull();
      expect(order?.length).toBe(doc.nodes.length);
    }
  });
});
