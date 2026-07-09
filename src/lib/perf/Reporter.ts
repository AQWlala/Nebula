/**
 * T-E-D-09: 报告生成器。
 *
 * 将 BenchmarkResult 与 MetricSummary 转换为文本 / JSON / Markdown 报告,
 * 支持与基线对比以在 CI 中检测性能回归。
 */
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import type { BenchmarkResult } from './Benchmark';
import type { MetricSummary } from './Metrics';

/** 基线比较单条记录。 */
export interface ComparisonEntry {
  name: string;
  currentMs: number;
  baselineMs: number;
  changePercent: number;
}

/** 基线比较结果。 */
export interface ComparisonResult {
  improvements: ComparisonEntry[];
  regressions: ComparisonEntry[];
  unchanged: ComparisonEntry[];
}

/** 基线文件名。 */
const BASELINE_FILE = 'baseline.json';

/**
 * 报告生成器:输出多种格式的性能报告,并管理基线文件的读写。
 *
 * 用法:
 * ```ts
 * const reporter = new Reporter('./perf-reports');
 * const text = reporter.generateText(results, summary);
 * reporter.saveBaseline(results);
 * ```
 */
export class Reporter {
  private outputDir: string;

  constructor(outputDir: string) {
    this.outputDir = outputDir;
  }

  /** 生成文本格式报告。 */
  generateText(results: BenchmarkResult[], metrics: MetricSummary): string {
    const lines: string[] = [];
    lines.push('=== Nebula 性能基准报告 ===');
    lines.push(`生成时间: ${new Date().toISOString()}`);
    lines.push('');
    lines.push('[基准测试结果]');
    lines.push(
      '名称\t迭代\t平均(ms)\t最小(ms)\t最大(ms)\t标准差\tp95(ms)\tops/s\t通过'
    );
    for (const r of results) {
      lines.push(
        [
          r.name,
          r.iterations,
          r.avgMs.toFixed(3),
          r.minMs.toFixed(3),
          r.maxMs.toFixed(3),
          r.stdDev.toFixed(3),
          r.percentiles.p95.toFixed(3),
          r.opsPerSecond.toFixed(1),
          r.passed ? 'PASS' : 'FAIL',
        ].join('\t')
      );
    }
    lines.push('');
    lines.push('[指标汇总]');
    lines.push(`总指标数: ${metrics.totalMetrics}`);
    lines.push(`渲染平均(ms): ${metrics.avgRenderMs.toFixed(3)}`);
    lines.push(`交互平均(ms): ${metrics.avgInteractionMs.toFixed(3)}`);
    lines.push(`最大内存(MB): ${metrics.maxMemoryMB.toFixed(3)}`);
    lines.push(`阈值违规: ${metrics.thresholdViolations.length}`);
    for (const v of metrics.thresholdViolations) {
      lines.push(`  - [${v.level}] ${v.metricName}: ${v.value} > ${v.threshold}`);
    }
    return lines.join('\n');
  }

  /** 生成 JSON 格式报告。 */
  generateJson(results: BenchmarkResult[], metrics: MetricSummary): string {
    return JSON.stringify(
      {
        generatedAt: Date.now(),
        results,
        metrics,
      },
      null,
      2
    );
  }

  /** 生成 Markdown 格式报告。 */
  generateMarkdown(results: BenchmarkResult[], metrics: MetricSummary): string {
    const lines: string[] = [];
    lines.push('# Nebula 性能基准报告');
    lines.push('');
    lines.push(`生成时间: ${new Date().toISOString()}`);
    lines.push('');
    lines.push('## 基准测试结果');
    lines.push('');
    lines.push(
      '| 名称 | 迭代 | 平均(ms) | 最小(ms) | 最大(ms) | 标准差 | p95(ms) | ops/s | 通过 |'
    );
    lines.push('| --- | --- | --- | --- | --- | --- | --- | --- | --- |');
    for (const r of results) {
      lines.push(
        `| ${r.name} | ${r.iterations} | ${r.avgMs.toFixed(3)} | ${r.minMs.toFixed(
          3
        )} | ${r.maxMs.toFixed(3)} | ${r.stdDev.toFixed(3)} | ${r.percentiles.p95.toFixed(
          3
        )} | ${r.opsPerSecond.toFixed(1)} | ${r.passed ? '✓' : '✗'} |`
      );
    }
    lines.push('');
    lines.push('## 指标汇总');
    lines.push('');
    lines.push(`- 总指标数: ${metrics.totalMetrics}`);
    lines.push(`- 渲染平均(ms): ${metrics.avgRenderMs.toFixed(3)}`);
    lines.push(`- 交互平均(ms): ${metrics.avgInteractionMs.toFixed(3)}`);
    lines.push(`- 最大内存(MB): ${metrics.maxMemoryMB.toFixed(3)}`);
    lines.push(`- 阈值违规: ${metrics.thresholdViolations.length}`);
    return lines.join('\n');
  }

  /** 将当前结果与基线对比,识别改进 / 回归 / 无变化。 */
  compareWithBaseline(
    current: BenchmarkResult[],
    baseline: BenchmarkResult[]
  ): ComparisonResult {
    const improvements: ComparisonEntry[] = [];
    const regressions: ComparisonEntry[] = [];
    const unchanged: ComparisonEntry[] = [];
    const baselineMap = new Map(baseline.map((b) => [b.name, b]));
    for (const c of current) {
      const b = baselineMap.get(c.name);
      if (!b) continue;
      const currentMs = c.avgMs;
      const baselineMs = b.avgMs;
      const changePercent =
        baselineMs === 0
          ? currentMs === 0
            ? 0
            : 100
          : ((currentMs - baselineMs) / baselineMs) * 100;
      const entry: ComparisonEntry = {
        name: c.name,
        currentMs,
        baselineMs,
        changePercent,
      };
      if (changePercent < 0) improvements.push(entry);
      else if (changePercent > 0) regressions.push(entry);
      else unchanged.push(entry);
    }
    return { improvements, regressions, unchanged };
  }

  /** 将结果保存为基线文件(覆盖已有基线)。 */
  saveBaseline(results: BenchmarkResult[]): void {
    mkdirSync(this.outputDir, { recursive: true });
    writeFileSync(
      join(this.outputDir, BASELINE_FILE),
      JSON.stringify(results, null, 2),
      'utf-8'
    );
  }

  /** 加载基线文件;若不存在或解析失败返回 null。 */
  loadBaseline(): BenchmarkResult[] | null {
    const file = join(this.outputDir, BASELINE_FILE);
    if (!existsSync(file)) return null;
    try {
      const raw = readFileSync(file, 'utf-8');
      return JSON.parse(raw) as BenchmarkResult[];
    } catch {
      return null;
    }
  }
}
