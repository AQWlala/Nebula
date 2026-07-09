/**
 * T-E-D-09: 前端 UI 性能基准测试单元测试。
 *
 * 覆盖 MetricCollector、BenchmarkSuite、Benchmark 静态工具、Reporter 的核心行为,
 * 确保渲染性能、交互延迟、内存使用等指标可在 CI 中可靠度量与比对。
 */
import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {
  MetricCollector,
  BenchmarkSuite,
  Benchmark,
  Reporter,
} from '../index';
import type { BenchmarkResult } from '../index';

// ---------------------------------------------------------------------------
// 辅助函数
// ---------------------------------------------------------------------------

/** 构造一个最小可用的 BenchmarkResult,用于 Reporter 测试。 */
function makeResult(name: string, avgMs: number): BenchmarkResult {
  return {
    name,
    iterations: 10,
    totalMs: avgMs * 10,
    avgMs,
    minMs: avgMs * 0.9,
    maxMs: avgMs * 1.1,
    stdDev: 0.5,
    percentiles: { p50: avgMs, p90: avgMs, p95: avgMs, p99: avgMs },
    opsPerSecond: avgMs > 0 ? 1000 / avgMs : 0,
    passed: true,
  };
}

// ---------------------------------------------------------------------------
// MetricCollector 测试
// ---------------------------------------------------------------------------

describe('MetricCollector (T-E-D-09)', () => {
  // 1. record 存储
  it('record 存储指标并可由 getAll 读取', () => {
    const c = new MetricCollector();
    c.record({ name: 'x', type: 'custom', value: 42, unit: 'ms', timestamp: 1 });
    const all = c.getAll();
    expect(all).toHaveLength(1);
    expect(all[0].value).toBe(42);
    expect(all[0].name).toBe('x');
  });

  // 2. start/end 计时
  it('start/end 测量耗时并返回非负毫秒数', () => {
    const c = new MetricCollector();
    c.start('timer');
    let sum = 0;
    for (let i = 0; i < 1000; i++) sum += i;
    const ms = c.end('timer');
    expect(typeof ms).toBe('number');
    expect(ms).toBeGreaterThanOrEqual(0);
    expect(sum).toBeGreaterThan(0); // 防止循环被优化掉
  });

  // 3. recordRender
  it('recordRender 记录 render 类型指标', () => {
    const c = new MetricCollector();
    c.recordRender('MyComponent', 5.5);
    const renders = c.getByType('render');
    expect(renders).toHaveLength(1);
    expect(renders[0].value).toBe(5.5);
    expect(renders[0].name).toContain('MyComponent');
    expect(renders[0].type).toBe('render');
  });

  // 4. recordInteraction
  it('recordInteraction 记录 interaction 类型指标', () => {
    const c = new MetricCollector();
    c.recordInteraction('click', 12.3);
    const interactions = c.getByType('interaction');
    expect(interactions).toHaveLength(1);
    expect(interactions[0].value).toBe(12.3);
    expect(interactions[0].type).toBe('interaction');
  });

  // 5. recordMemory
  it('recordMemory 记录 memory 类型指标并以 totalMB 为 critical 阈值', () => {
    const c = new MetricCollector();
    c.recordMemory(50, 100);
    const mem = c.getByType('memory');
    expect(mem).toHaveLength(1);
    expect(mem[0].value).toBe(50);
    expect(mem[0].threshold?.critical).toBe(100);
    expect(mem[0].type).toBe('memory');
  });

  // 6. getByType + clear
  it('getByType 按类型筛选,clear 清空所有指标', () => {
    const c = new MetricCollector();
    c.recordRender('A', 1);
    c.recordInteraction('tap', 2);
    expect(c.getByType('render')).toHaveLength(1);
    expect(c.getByType('interaction')).toHaveLength(1);
    expect(c.getByType('memory')).toHaveLength(0);
    c.clear();
    expect(c.getAll()).toHaveLength(0);
  });

  // 7. MetricSummary 生成
  it('summary 生成正确的汇总(总数/分类/平均值/最大内存)', () => {
    const c = new MetricCollector();
    c.recordRender('A', 10);
    c.recordRender('B', 20);
    c.recordInteraction('click', 30);
    c.recordMemory(50, 100);
    c.recordMemory(80, 100);
    const s = c.summary();
    expect(s.totalMetrics).toBe(5);
    expect(s.byType.render).toBe(2);
    expect(s.byType.interaction).toBe(1);
    expect(s.byType.memory).toBe(2);
    expect(s.byType.network).toBe(0);
    expect(s.byType.custom).toBe(0);
    expect(s.avgRenderMs).toBe(15); // (10 + 20) / 2
    expect(s.avgInteractionMs).toBe(30);
    expect(s.maxMemoryMB).toBe(80);
  });

  // 8. ThresholdViolation 检测
  it('summary 检测阈值违规(warn/error/critical)', () => {
    const c = new MetricCollector();
    const threshold = { warn: 10, error: 20, critical: 30 };
    c.record({ name: 'r-warn', type: 'render', value: 15, unit: 'ms', timestamp: 1, threshold });
    c.record({ name: 'r-error', type: 'render', value: 25, unit: 'ms', timestamp: 2, threshold });
    c.record({ name: 'r-crit', type: 'render', value: 35, unit: 'ms', timestamp: 3, threshold });
    c.record({ name: 'r-ok', type: 'render', value: 5, unit: 'ms', timestamp: 4, threshold });
    const s = c.summary();
    const levels = s.thresholdViolations.map((v) => v.level).sort();
    expect(levels).toEqual(['critical', 'error', 'warn']);
    const crit = s.thresholdViolations.find((v) => v.level === 'critical');
    expect(crit?.value).toBe(35);
    expect(crit?.threshold).toBe(30);
    expect(crit?.metricName).toBe('r-crit');
  });
});

// ---------------------------------------------------------------------------
// BenchmarkSuite & Benchmark 测试
// ---------------------------------------------------------------------------

describe('BenchmarkSuite & Benchmark (T-E-D-09)', () => {
  // 9. add + names
  it('add 注册测试,names 返回名称列表', () => {
    const suite = new BenchmarkSuite('s1');
    suite.add('a', () => { /* no-op */ });
    suite.add('b', () => { /* no-op */ });
    expect(suite.names()).toEqual(['a', 'b']);
  });

  // 10. run
  it('run 运行所有测试并返回结果数组', async () => {
    const suite = new BenchmarkSuite('s1');
    suite.add(
      'fast',
      () => { let s = 0; for (let i = 0; i < 100; i++) s += i; },
      { iterations: 5, warmupIterations: 1, minSamples: 1 }
    );
    const results = await suite.run();
    expect(results).toHaveLength(1);
    expect(results[0].name).toBe('fast');
    expect(results[0].iterations).toBe(5);
    expect(results[0].passed).toBe(true);
    expect(results[0].minMs).toBeLessThanOrEqual(results[0].maxMs);
  });

  // 11. runSingle
  it('runSingle 运行指定测试', async () => {
    const suite = new BenchmarkSuite('s1');
    suite.add('x', () => { /* no-op */ }, { iterations: 3, warmupIterations: 1, minSamples: 1 });
    suite.add('y', () => { /* no-op */ }, { iterations: 3, warmupIterations: 1, minSamples: 1 });
    const r = await suite.runSingle('y');
    expect(r.name).toBe('y');
    expect(r.iterations).toBe(3);
  });

  // 12. Benchmark.measure (async)
  it('Benchmark.measure 测量异步函数并返回结果与耗时', async () => {
    const { result, durationMs } = await Benchmark.measure(async () => 42);
    expect(result).toBe(42);
    expect(durationMs).toBeGreaterThanOrEqual(0);
  });

  // 13. Benchmark.measureSync
  it('Benchmark.measureSync 测量同步函数并返回结果与耗时', () => {
    const { result, durationMs } = Benchmark.measureSync(() => 'hello');
    expect(result).toBe('hello');
    expect(durationMs).toBeGreaterThanOrEqual(0);
  });

  // 14. calculateStats 正确性
  it('calculateStats 计算 mean/median/stdDev/min/max 正确', () => {
    const stats = Benchmark.calculateStats([1, 2, 3, 4, 5]);
    expect(stats.mean).toBeCloseTo(3, 6);
    expect(stats.median).toBeCloseTo(3, 6);
    expect(stats.min).toBe(1);
    expect(stats.max).toBe(5);
    // 总体标准差 = sqrt(((1-3)^2+(2-3)^2+(3-3)^2+(4-3)^2+(5-3)^2)/5) = sqrt(2)
    expect(stats.stdDev).toBeCloseTo(Math.sqrt(2), 6);
  });

  // 15. Percentiles 计算
  it('calculateStats 的百分位计算正确(p50/p90/p95/p99)', () => {
    const stats = Benchmark.calculateStats([1, 2, 3, 4, 5]);
    expect(stats.percentiles.p50).toBeCloseTo(3, 6);
    expect(stats.percentiles.p90).toBeCloseTo(4.6, 6);
    expect(stats.percentiles.p95).toBeCloseTo(4.8, 6);
    expect(stats.percentiles.p99).toBeCloseTo(4.96, 6);
  });

  // 16. BenchmarkOptions 默认值
  it('BenchmarkOptions 默认值: iterations=100/warmup=10/minSamples=30', async () => {
    const suite = new BenchmarkSuite('defaults');
    suite.add('d', () => { /* no-op */ });
    const r = await suite.runSingle('d');
    expect(r.iterations).toBe(100);
    expect(r.passed).toBe(true); // 100 >= minSamples(30)
  });

  // 17. 空结果处理
  it('空 suite.run 返回空数组', async () => {
    const suite = new BenchmarkSuite('empty');
    const results = await suite.run();
    expect(results).toEqual([]);
  });

  // 18. 样本不足时 passed 为 false
  it('样本数低于 minSamples 时 passed 为 false', async () => {
    const suite = new BenchmarkSuite('low-samples');
    suite.add('s', () => { /* no-op */ }, { iterations: 2, warmupIterations: 0, minSamples: 10 });
    const r = await suite.runSingle('s');
    expect(r.iterations).toBe(2);
    expect(r.passed).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// Reporter 测试
// ---------------------------------------------------------------------------

describe('Reporter (T-E-D-09)', () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = mkdtempSync(join(tmpdir(), 'nebula-perf-'));
  });

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true });
  });

  // 19. generateText
  it('generateText 生成包含测试名的文本报告', () => {
    const reporter = new Reporter(tmpDir);
    const results = [makeResult('alpha', 10)];
    const summary = new MetricCollector().summary();
    const text = reporter.generateText(results, summary);
    expect(typeof text).toBe('string');
    expect(text.length).toBeGreaterThan(0);
    expect(text).toContain('alpha');
  });

  // 20. generateJson
  it('generateJson 返回可解析的 JSON 并包含结果', () => {
    const reporter = new Reporter(tmpDir);
    const results = [makeResult('beta', 20)];
    const summary = new MetricCollector().summary();
    const json = reporter.generateJson(results, summary);
    const parsed = JSON.parse(json) as { results: BenchmarkResult[] };
    expect(Array.isArray(parsed.results)).toBe(true);
    expect(parsed.results[0].name).toBe('beta');
    expect(parsed.results[0].avgMs).toBe(20);
  });

  // 21. generateMarkdown
  it('generateMarkdown 生成包含表格的 Markdown', () => {
    const reporter = new Reporter(tmpDir);
    const results = [makeResult('gamma', 30)];
    const summary = new MetricCollector().summary();
    const md = reporter.generateMarkdown(results, summary);
    expect(md).toContain('gamma');
    expect(md).toContain('|'); // markdown 表格分隔符
  });

  // 22. compareWithBaseline 改进
  it('compareWithBaseline 识别性能改进', () => {
    const reporter = new Reporter(tmpDir);
    const baseline = [makeResult('op', 100)];
    const current = [makeResult('op', 80)];
    const cmp = reporter.compareWithBaseline(current, baseline);
    expect(cmp.improvements).toHaveLength(1);
    expect(cmp.improvements[0].changePercent).toBeLessThan(0);
    expect(cmp.regressions).toHaveLength(0);
    expect(cmp.unchanged).toHaveLength(0);
  });

  // 23. compareWithBaseline 回归
  it('compareWithBaseline 识别性能回归', () => {
    const reporter = new Reporter(tmpDir);
    const baseline = [makeResult('op', 100)];
    const current = [makeResult('op', 120)];
    const cmp = reporter.compareWithBaseline(current, baseline);
    expect(cmp.regressions).toHaveLength(1);
    expect(cmp.regressions[0].changePercent).toBeGreaterThan(0);
    expect(cmp.improvements).toHaveLength(0);
  });

  // 24. compareWithBaseline 无变化
  it('compareWithBaseline 识别无变化', () => {
    const reporter = new Reporter(tmpDir);
    const baseline = [makeResult('op', 100)];
    const current = [makeResult('op', 100)];
    const cmp = reporter.compareWithBaseline(current, baseline);
    expect(cmp.unchanged).toHaveLength(1);
    expect(cmp.improvements).toHaveLength(0);
    expect(cmp.regressions).toHaveLength(0);
  });

  // 25. saveBaseline / loadBaseline 往返
  it('saveBaseline 与 loadBaseline 往返一致', () => {
    const reporter = new Reporter(tmpDir);
    const results = [makeResult('persist', 50)];
    reporter.saveBaseline(results);
    const loaded = reporter.loadBaseline();
    expect(loaded).not.toBeNull();
    expect(loaded?.[0].name).toBe('persist');
    expect(loaded?.[0].avgMs).toBe(50);
  });

  // 26. loadBaseline 无文件返回 null
  it('loadBaseline 无基线文件时返回 null', () => {
    const reporter = new Reporter(tmpDir);
    expect(reporter.loadBaseline()).toBeNull();
  });

  // 27. 空结果处理
  it('空结果时 Reporter 各方法不抛错', () => {
    const reporter = new Reporter(tmpDir);
    const summary = new MetricCollector().summary();
    expect(() => reporter.generateText([], summary)).not.toThrow();
    expect(() => reporter.generateJson([], summary)).not.toThrow();
    expect(() => reporter.generateMarkdown([], summary)).not.toThrow();
    const cmp = reporter.compareWithBaseline([], []);
    expect(cmp.improvements).toHaveLength(0);
    expect(cmp.regressions).toHaveLength(0);
    expect(cmp.unchanged).toHaveLength(0);
  });
});
