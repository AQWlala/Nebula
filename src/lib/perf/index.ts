/**
 * T-E-D-09: 前端 UI 性能基准测试模块 — 统一导出。
 *
 * 提供渲染性能、交互延迟、内存使用的度量与 CI 基线比对能力。
 *
 * 用法:
 * ```ts
 * import { BenchmarkSuite, MetricCollector, Reporter } from '@/lib/perf';
 * ```
 */

// 指标
export { MetricCollector } from './Metrics';
export type {
  MetricType,
  Metric,
  MetricThreshold,
  MetricSummary,
  ThresholdViolation,
} from './Metrics';

// 基准测试
export { BenchmarkSuite, Benchmark } from './Benchmark';
export type {
  BenchmarkOptions,
  BenchmarkResult,
  Percentiles,
  Stats,
} from './Benchmark';

// 报告
export { Reporter } from './Reporter';
export type { ComparisonResult, ComparisonEntry } from './Reporter';
