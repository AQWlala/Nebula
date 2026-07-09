/**
 * T-E-D-09: 性能指标定义与收集。
 *
 * 提供渲染时间、交互延迟、内存使用等 UI 性能指标的采集与汇总能力,
 * 供 BenchmarkSuite 运行时记录、Reporter 生成报告时消费。
 */

/** 指标类型:渲染 / 交互 / 内存 / 网络 / 自定义。 */
export type MetricType = 'render' | 'interaction' | 'memory' | 'network' | 'custom';

/** 指标阈值,按严重程度递进(值越大越严重)。 */
export interface MetricThreshold {
  warn: number;
  error: number;
  critical: number;
}

/** 单条性能指标。 */
export interface Metric {
  name: string;
  type: MetricType;
  value: number;
  unit: string;
  timestamp: number;
  threshold?: MetricThreshold;
}

/** 阈值违规记录。 */
export interface ThresholdViolation {
  metricName: string;
  level: 'warn' | 'error' | 'critical';
  value: number;
  threshold: number;
}

/** 指标汇总。 */
export interface MetricSummary {
  totalMetrics: number;
  byType: Record<MetricType, number>;
  avgRenderMs: number;
  avgInteractionMs: number;
  maxMemoryMB: number;
  thresholdViolations: ThresholdViolation[];
}

/**
 * 检查单个指标是否违反阈值,返回最严重的违规级别(若无违规返回 null)。
 * 约定:值越大越差(适用于耗时、内存等性能指标)。
 */
function checkViolation(
  value: number,
  threshold: MetricThreshold
): 'warn' | 'error' | 'critical' | null {
  if (value > threshold.critical) return 'critical';
  if (value > threshold.error) return 'error';
  if (value > threshold.warn) return 'warn';
  return null;
}

/**
 * 指标收集器:记录并汇总 UI 性能指标。
 *
 * 用法:
 * ```ts
 * const collector = new MetricCollector();
 * collector.start('render');
 * // ... 执行渲染 ...
 * const ms = collector.end('render');
 * collector.recordRender('MyComponent', ms);
 * const summary = collector.summary();
 * ```
 */
export class MetricCollector {
  private metrics: Metric[] = [];
  private timers = new Map<string, number>();

  constructor() {
    // 无额外初始化
  }

  /** 开始计时,以 name 作为唯一标识。 */
  start(name: string): void {
    this.timers.set(name, performance.now());
  }

  /** 结束计时并返回耗时(毫秒)。若未找到对应计时器则抛错。 */
  end(name: string): number {
    const start = this.timers.get(name);
    if (start === undefined) {
      throw new Error(`MetricCollector: 未找到名为 "${name}" 的计时器`);
    }
    const duration = performance.now() - start;
    this.timers.delete(name);
    return duration;
  }

  /** 记录一条原始指标。 */
  record(metric: Metric): void {
    this.metrics.push(metric);
  }

  /** 记录组件渲染耗时。 */
  recordRender(componentName: string, renderTimeMs: number): void {
    this.record({
      name: `render.${componentName}`,
      type: 'render',
      value: renderTimeMs,
      unit: 'ms',
      timestamp: Date.now(),
    });
  }

  /** 记录交互延迟。 */
  recordInteraction(action: string, latencyMs: number): void {
    this.record({
      name: `interaction.${action}`,
      type: 'interaction',
      value: latencyMs,
      unit: 'ms',
      timestamp: Date.now(),
    });
  }

  /** 记录内存使用,以 totalMB 作为 critical 阈值(内存预算)。 */
  recordMemory(usedMB: number, totalMB: number): void {
    this.record({
      name: 'memory.used',
      type: 'memory',
      value: usedMB,
      unit: 'MB',
      timestamp: Date.now(),
      threshold: { warn: totalMB * 0.8, error: totalMB * 0.9, critical: totalMB },
    });
  }

  /** 获取所有指标(返回副本)。 */
  getAll(): Metric[] {
    return [...this.metrics];
  }

  /** 按类型筛选指标。 */
  getByType(type: MetricType): Metric[] {
    return this.metrics.filter((m) => m.type === type);
  }

  /** 清空所有指标与计时器。 */
  clear(): void {
    this.metrics = [];
    this.timers.clear();
  }

  /** 生成指标汇总。 */
  summary(): MetricSummary {
    const byType: Record<MetricType, number> = {
      render: 0,
      interaction: 0,
      memory: 0,
      network: 0,
      custom: 0,
    };
    for (const m of this.metrics) {
      byType[m.type] += 1;
    }

    const renderMetrics = this.getByType('render');
    const interactionMetrics = this.getByType('interaction');
    const memoryMetrics = this.getByType('memory');

    const avgRenderMs =
      renderMetrics.length > 0
        ? renderMetrics.reduce((s, m) => s + m.value, 0) / renderMetrics.length
        : 0;
    const avgInteractionMs =
      interactionMetrics.length > 0
        ? interactionMetrics.reduce((s, m) => s + m.value, 0) / interactionMetrics.length
        : 0;
    const maxMemoryMB =
      memoryMetrics.length > 0
        ? memoryMetrics.reduce((max, m) => Math.max(max, m.value), 0)
        : 0;

    const thresholdViolations: ThresholdViolation[] = [];
    for (const m of this.metrics) {
      if (!m.threshold) continue;
      const level = checkViolation(m.value, m.threshold);
      if (level) {
        thresholdViolations.push({
          metricName: m.name,
          level,
          value: m.value,
          threshold: m.threshold[level],
        });
      }
    }

    return {
      totalMetrics: this.metrics.length,
      byType,
      avgRenderMs,
      avgInteractionMs,
      maxMemoryMB,
      thresholdViolations,
    };
  }
}
