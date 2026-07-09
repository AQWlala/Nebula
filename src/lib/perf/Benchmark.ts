/**
 * T-E-D-09: 性能基准测试核心。
 *
 * 提供 BenchmarkSuite(测试套件)与 Benchmark(静态测量工具),
 * 支持预热、多次采样、统计计算(均值/标准差/百分位),可在 CI 中批量运行。
 */

/** 百分位数值。 */
export interface Percentiles {
  p50: number;
  p90: number;
  p95: number;
  p99: number;
}

/** 统计摘要。 */
export interface Stats {
  mean: number;
  median: number;
  stdDev: number;
  min: number;
  max: number;
  percentiles: Percentiles;
}

/** 基准测试选项。 */
export interface BenchmarkOptions {
  /** 正式采样迭代次数,默认 100。 */
  iterations: number;
  /** 预热迭代次数,默认 10。 */
  warmupIterations: number;
  /** 超时阈值(毫秒),默认 5000。 */
  timeout: number;
  /** 最少有效样本数,低于此值标记为未通过,默认 30。 */
  minSamples: number;
}

/** 单次基准测试结果。 */
export interface BenchmarkResult {
  name: string;
  iterations: number;
  totalMs: number;
  avgMs: number;
  minMs: number;
  maxMs: number;
  stdDev: number;
  percentiles: Percentiles;
  opsPerSecond: number;
  passed: boolean;
}

/** 默认选项。 */
const DEFAULT_OPTIONS: BenchmarkOptions = {
  iterations: 100,
  warmupIterations: 10,
  timeout: 5000,
  minSamples: 30,
};

/**
 * 计算已排序数组的指定百分位(线性插值法)。
 * @param sorted 升序排列的样本
 * @param p 百分位(0-100)
 */
function percentileOfSorted(sorted: number[], p: number): number {
  if (sorted.length === 0) return 0;
  if (sorted.length === 1) return sorted[0];
  const index = (p / 100) * (sorted.length - 1);
  const lower = Math.floor(index);
  const upper = Math.ceil(index);
  if (lower === upper) return sorted[lower];
  const weight = index - lower;
  return sorted[lower] * (1 - weight) + sorted[upper] * weight;
}

/** 根据采样数组与选项计算 BenchmarkResult。 */
function computeResult(
  name: string,
  samples: number[],
  options: BenchmarkOptions
): BenchmarkResult {
  const stats = Benchmark.calculateStats(samples);
  const totalMs = samples.reduce((s, v) => s + v, 0);
  return {
    name,
    iterations: samples.length,
    totalMs,
    avgMs: stats.mean,
    minMs: stats.min,
    maxMs: stats.max,
    stdDev: stats.stdDev,
    percentiles: stats.percentiles,
    opsPerSecond: stats.mean > 0 ? 1000 / stats.mean : 0,
    passed: samples.length >= options.minSamples,
  };
}

/**
 * 基准测试套件:批量注册并运行多个基准测试。
 *
 * 用法:
 * ```ts
 * const suite = new BenchmarkSuite('ui-perf');
 * suite.add('render-list', () => renderList(1000), { iterations: 50 });
 * const results = await suite.run();
 * ```
 */
export class BenchmarkSuite {
  private suiteName: string;
  private tests = new Map<
    string,
    { fn: () => void | Promise<void>; options: BenchmarkOptions }
  >();

  constructor(name: string) {
    this.suiteName = name;
  }

  /** 添加一个基准测试。同名测试会被覆盖。 */
  add(
    name: string,
    fn: () => void | Promise<void>,
    options?: Partial<BenchmarkOptions>
  ): void {
    this.tests.set(name, {
      fn,
      options: { ...DEFAULT_OPTIONS, ...options },
    });
  }

  /** 获取所有已注册测试的名称(按注册顺序)。 */
  names(): string[] {
    return [...this.tests.keys()];
  }

  /** 运行所有测试,返回结果数组。 */
  async run(): Promise<BenchmarkResult[]> {
    const results: BenchmarkResult[] = [];
    for (const name of this.tests.keys()) {
      results.push(await this.runSingle(name));
    }
    return results;
  }

  /** 运行单个测试。 */
  async runSingle(name: string): Promise<BenchmarkResult> {
    const test = this.tests.get(name);
    if (!test) {
      throw new Error(`BenchmarkSuite "${this.suiteName}": 未找到名为 "${name}" 的测试`);
    }
    // 预热:不采样,仅触发 JIT / 缓存
    for (let i = 0; i < test.options.warmupIterations; i++) {
      await test.fn();
    }
    // 正式采样
    const samples: number[] = [];
    for (let i = 0; i < test.options.iterations; i++) {
      const start = performance.now();
      await test.fn();
      samples.push(performance.now() - start);
    }
    return computeResult(name, samples, test.options);
  }
}

/**
 * Benchmark 静态工具类:单次测量与统计计算。
 */
export class Benchmark {
  /** 测量异步函数的执行耗时。 */
  static async measure<T>(
    fn: () => Promise<T>
  ): Promise<{ result: T; durationMs: number }> {
    const start = performance.now();
    const result = await fn();
    return { result, durationMs: performance.now() - start };
  }

  /** 测量同步函数的执行耗时。 */
  static measureSync<T>(fn: () => T): { result: T; durationMs: number } {
    const start = performance.now();
    const result = fn();
    return { result, durationMs: performance.now() - start };
  }

  /** 计算样本数组的统计值(均值/中位数/标准差/极值/百分位)。 */
  static calculateStats(samples: number[]): Stats {
    if (samples.length === 0) {
      return {
        mean: 0,
        median: 0,
        stdDev: 0,
        min: 0,
        max: 0,
        percentiles: { p50: 0, p90: 0, p95: 0, p99: 0 },
      };
    }
    const sorted = [...samples].sort((a, b) => a - b);
    const sum = sorted.reduce((s, v) => s + v, 0);
    const mean = sum / sorted.length;
    // 总体标准差
    const variance =
      sorted.reduce((s, v) => s + (v - mean) ** 2, 0) / sorted.length;
    const stdDev = Math.sqrt(variance);
    const mid = Math.floor(sorted.length / 2);
    const median =
      sorted.length % 2 === 0
        ? (sorted[mid - 1] + sorted[mid]) / 2
        : sorted[mid];
    return {
      mean,
      median,
      stdDev,
      min: sorted[0],
      max: sorted[sorted.length - 1],
      percentiles: {
        p50: percentileOfSorted(sorted, 50),
        p90: percentileOfSorted(sorted, 90),
        p95: percentileOfSorted(sorted, 95),
        p99: percentileOfSorted(sorted, 99),
      },
    };
  }
}
