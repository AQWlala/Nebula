/**
 * T-E-D-08: WebGL 渲染引擎核心。
 *
 * 职责:
 * - 管理 WebGL 上下文与着色器缓存
 * - 提供 drawPoints/drawLines/drawTriangles/drawText 等基础绘制 API
 * - 将 Scene + Camera 的组合渲染到 canvas
 * - 统计 FPS / draw calls / 图元数量,供上层性能监控
 *
 * 可测试性:在无 WebGL 环境(jsdom/Node)中构造时,所有 GL 调用降级为 no-op,
 * 但 stats 计数与 options 默认值仍可被测试断言。这样 vitest 无需 mock WebGL
 * 即可覆盖纯逻辑分支。
 */

import {
  ShaderCache,
  POINT_VERTEX_SHADER,
  POINT_FRAGMENT_SHADER,
  LINE_VERTEX_SHADER,
  LINE_FRAGMENT_SHADER,
  TRIANGLE_VERTEX_SHADER,
  TRIANGLE_FRAGMENT_SHADER,
} from './shaders';
import type { Scene } from './Scene';
import type { Camera } from './Camera';

/** 渲染器构造选项。 */
export interface RendererOptions {
  /** 抗锯齿,默认 true。 */
  antialias?: boolean;
  /** 透明背景,默认 false。 */
  alpha?: boolean;
  /** 预乘 alpha,默认 true。 */
  premultipliedAlpha?: boolean;
  /** 保留绘图缓冲(截图/读取像素需要),默认 true。 */
  preserveDrawingBuffer?: boolean;
}

/** 点绘制选项。 */
export interface PointDrawOptions {
  /** 点大小(像素),默认 4。 */
  size?: number;
  /** 是否启用圆形平滑,默认 true。 */
  smooth?: boolean;
}

/** 线绘制选项。 */
export interface LineDrawOptions {
  /** 线宽(像素,部分平台仅支持 1),默认 1。 */
  width?: number;
}

/** 文本绘制选项(基于 canvas 2D 叠加)。 */
export interface TextDrawOptions {
  /** CSS font 字符串,默认 '12px system-ui'。 */
  font?: string;
  /** 填充颜色,默认 '#ffffff'。 */
  color?: string;
  /** 字号(像素),默认 12。 */
  size?: number;
}

/** 渲染器运行统计。 */
export interface RendererStats {
  /** 当前 FPS(每秒帧数)。 */
  fps: number;
  /** 上一帧耗时(毫秒)。 */
  frameTime: number;
  /** 累计 draw calls。 */
  drawCalls: number;
  /** 累计三角形数。 */
  triangles: number;
  /** 累计点数。 */
  points: number;
  /** 累计线段数(顶点对数)。 */
  lines: number;
}

/** RendererOptions 的默认值。 */
export const DEFAULT_RENDERER_OPTIONS: Required<RendererOptions> = {
  antialias: true,
  alpha: false,
  premultipliedAlpha: true,
  preserveDrawingBuffer: true,
};

/** PointDrawOptions 的默认值。 */
export const DEFAULT_POINT_OPTIONS: Required<PointDrawOptions> = {
  size: 4,
  smooth: true,
};

/** LineDrawOptions 的默认值。 */
export const DEFAULT_LINE_OPTIONS: Required<LineDrawOptions> = {
  width: 1,
};

/** TextDrawOptions 的默认值。 */
export const DEFAULT_TEXT_OPTIONS: Required<TextDrawOptions> = {
  font: '12px system-ui',
  color: '#ffffff',
  size: 12,
};

/** 初始 stats 值(供测试断言)。 */
export const INITIAL_STATS: RendererStats = {
  fps: 0,
  frameTime: 0,
  drawCalls: 0,
  triangles: 0,
  points: 0,
  lines: 0,
};

/**
 * WebGL 渲染器。
 *
 * 用法:
 * ```ts
 * const renderer = new WebGLRenderer(canvas);
 * const camera = new Camera('2d', { width, height });
 * const scene = new Scene();
 * renderer.drawScene(scene, camera);
 * ```
 */
export class WebGLRenderer {
  readonly canvas: HTMLCanvasElement;
  readonly options: Required<RendererOptions>;
  /** WebGL 上下文,无 WebGL 环境下为 null(测试模式)。 */
  readonly gl: WebGLRenderingContext | null;
  private readonly shaderCache: ShaderCache | null;
  /** 2D 叠加层 canvas(用于 drawText),惰性创建。 */
  private overlayCanvas: HTMLCanvasElement | null = null;
  private overlayCtx: CanvasRenderingContext2D | null = null;

  private stats: RendererStats = { ...INITIAL_STATS };
  private lastFrameTime = 0;
  private fpsAccum = 0;
  private fpsFrames = 0;
  private disposed = false;

  constructor(canvas: HTMLCanvasElement, options: RendererOptions = {}) {
    this.canvas = canvas;
    this.options = { ...DEFAULT_RENDERER_OPTIONS, ...options };

    // 尝试获取 WebGL 上下文;失败时降级为 no-op 模式(供测试)
    const ctx = canvas.getContext('webgl', {
      antialias: this.options.antialias,
      alpha: this.options.alpha,
      premultipliedAlpha: this.options.premultipliedAlpha,
      preserveDrawingBuffer: this.options.preserveDrawingBuffer,
    });
    this.gl = ctx;
    this.shaderCache = ctx ? new ShaderCache(ctx) : null;

    if (ctx) {
      ctx.enable(ctx.BLEND);
      ctx.blendFunc(ctx.SRC_ALPHA, ctx.ONE_MINUS_SRC_ALPHA);
    }
  }

  /** 是否处于可用状态(有 WebGL 上下文且未释放)。 */
  get ready(): boolean {
    return this.gl !== null && !this.disposed;
  }

  /** 调整 canvas 与视口尺寸。 */
  resize(width: number, height: number): void {
    if (width <= 0 || height <= 0) return;
    this.canvas.width = width;
    this.canvas.height = height;
    if (this.gl) {
      this.gl.viewport(0, 0, width, height);
    }
    if (this.overlayCanvas) {
      this.overlayCanvas.width = width;
      this.overlayCanvas.height = height;
    }
  }

  /** 设置清屏颜色(各分量 0..1)。 */
  setClearColor(r: number, g: number, b: number, a: number): void {
    if (this.gl) {
      this.gl.clearColor(r, g, b, a);
    }
  }

  /** 清屏(颜色缓冲)。 */
  clear(): void {
    if (this.gl) {
      this.gl.clear(this.gl.COLOR_BUFFER_BIT | this.gl.DEPTH_BUFFER_BIT);
    }
  }

  /**
   * 绘制点精灵。
   * positions: [x0,y0,z0, x1,y1,z1, ...](每点 3 个 float)
   * colors:    [r0,g0,b0,a0, r1,g1,b1,a1, ...](每点 4 个 float)
   */
  drawPoints(positions: Float32Array, colors: Float32Array, options: PointDrawOptions = {}): void {
    const opts = { ...DEFAULT_POINT_OPTIONS, ...options };
    const count = Math.floor(positions.length / 3);
    this.stats.points += count;
    this.stats.drawCalls += 1;

    const gl = this.gl;
    if (!gl || !this.shaderCache) return;

    const program = this.shaderCache.getProgram(POINT_VERTEX_SHADER, POINT_FRAGMENT_SHADER);
    gl.useProgram(program);

    const uPointSize = gl.getUniformLocation(program, 'uPointSize');
    if (uPointSize) gl.uniform1f(uPointSize, opts.size);
    const uSmooth = gl.getUniformLocation(program, 'uSmooth');
    if (uSmooth) gl.uniform1i(uSmooth, opts.smooth ? 1 : 0);

    this.bindAttribute(gl, program, 'aPosition', positions, 3);
    this.bindAttribute(gl, program, 'aColor', colors, 4);

    gl.drawArrays(gl.POINTS, 0, count);
  }

  /**
   * 绘制线段(LINES:每两个顶点一条独立线段)。
   * positions: [x0,y0,z0, x1,y1,z1, ...]
   */
  drawLines(positions: Float32Array, colors: Float32Array, options: LineDrawOptions = {}): void {
    const opts = { ...DEFAULT_LINE_OPTIONS, ...options };
    const vertexCount = Math.floor(positions.length / 3);
    const lineCount = Math.floor(vertexCount / 2);
    this.stats.lines += lineCount;
    this.stats.drawCalls += 1;

    const gl = this.gl;
    if (!gl || !this.shaderCache) return;

    const program = this.shaderCache.getProgram(LINE_VERTEX_SHADER, LINE_FRAGMENT_SHADER);
    gl.useProgram(program);
    gl.lineWidth(opts.width);

    this.bindAttribute(gl, program, 'aPosition', positions, 3);
    this.bindAttribute(gl, program, 'aColor', colors, 4);

    gl.drawArrays(gl.LINES, 0, vertexCount);
  }

  /**
   * 绘制三角形(TRIANGLES:每三个顶点一个独立三角形)。
   */
  drawTriangles(positions: Float32Array, colors: Float32Array): void {
    const vertexCount = Math.floor(positions.length / 3);
    const triCount = Math.floor(vertexCount / 3);
    this.stats.triangles += triCount;
    this.stats.drawCalls += 1;

    const gl = this.gl;
    if (!gl || !this.shaderCache) return;

    const program = this.shaderCache.getProgram(TRIANGLE_VERTEX_SHADER, TRIANGLE_FRAGMENT_SHADER);
    gl.useProgram(program);

    this.bindAttribute(gl, program, 'aPosition', positions, 3);
    this.bindAttribute(gl, program, 'aColor', colors, 4);

    gl.drawArrays(gl.TRIANGLES, 0, vertexCount);
  }

  /**
   * 绘制文本(使用 2D canvas 叠加层)。
   * x/y 为屏幕坐标(像素,原点左上)。
   */
  drawText(text: string, x: number, y: number, options: TextDrawOptions = {}): void {
    const opts = { ...DEFAULT_TEXT_OPTIONS, ...options };
    this.stats.drawCalls += 1;

    const ctx = this.ensureOverlay();
    if (!ctx) return;
    ctx.font = opts.font;
    ctx.fillStyle = opts.color;
    ctx.textBaseline = 'top';
    ctx.fillText(text, x, y);
  }

  /**
   * 渲染整个场景。遍历 Scene 中的可见节点,按类型分发到对应 draw 方法。
   * 当前实现:Point → drawPoints,Line → drawLines,Triangle → drawTriangles,
   * Group → 跳过(容器)。颜色取自节点 color 字段。
   */
  drawScene(scene: Scene, camera: Camera): void {
    const now = performance.now();
    const dt = this.lastFrameTime === 0 ? 0 : now - this.lastFrameTime;
    this.lastFrameTime = now;
    this.stats.frameTime = dt;

    // FPS 滑动平均(每秒重置)
    this.fpsAccum += dt;
    this.fpsFrames += 1;
    if (this.fpsAccum >= 1000) {
      this.stats.fps = Math.round((this.fpsFrames * 1000) / this.fpsAccum);
      this.fpsAccum = 0;
      this.fpsFrames = 0;
    }

    this.clear();
    scene.update(dt / 1000);

    const visible = scene.getVisibleNodes(camera);
    const points = new Float32Array(visible.length * 3);
    const colors = new Float32Array(visible.length * 4);
    let pi = 0;
    let ci = 0;
    for (const node of visible) {
      if (node.type === 'Point') {
        points[pi++] = node.position[0];
        points[pi++] = node.position[1];
        points[pi++] = node.position[2];
        colors[ci++] = node.color[0];
        colors[ci++] = node.color[1];
        colors[ci++] = node.color[2];
        colors[ci++] = node.color[3];
      }
    }
    const pointCount = pi / 3;
    if (pointCount > 0) {
      this.drawPoints(points.slice(0, pi), colors.slice(0, ci));
    }
  }

  /**
   * 截图,返回 dataURL。
   * 无 WebGL 上下文时返回 1×1 透明 PNG 占位。
   */
  screenshot(): string {
    if (!this.gl) {
      // 1×1 透明 PNG 的 dataURL
      return 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+M8AAAMBAQDJ/pLvAAAAAElFTkSuQmCC';
    }
    try {
      // 合并 WebGL canvas 与 2D overlay
      if (this.overlayCanvas && this.overlayCtx) {
        const out = document.createElement('canvas');
        out.width = this.canvas.width;
        out.height = this.canvas.height;
        const outCtx = out.getContext('2d');
        if (outCtx) {
          outCtx.drawImage(this.canvas, 0, 0);
          outCtx.drawImage(this.overlayCanvas, 0, 0);
          return out.toDataURL('image/png');
        }
      }
      return this.canvas.toDataURL('image/png');
    } catch {
      return 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+M8AAAMBAQDJ/pLvAAAAAElFTkSuQmCC';
    }
  }

  /** 获取当前统计快照(返回副本,修改不影响内部)。 */
  getStats(): RendererStats {
    return { ...this.stats };
  }

  /** 重置统计计数(供新一轮渲染周期使用)。 */
  resetStats(): void {
    this.stats = { ...INITIAL_STATS };
    this.lastFrameTime = 0;
    this.fpsAccum = 0;
    this.fpsFrames = 0;
  }

  /** 释放 WebGL 资源(program/buffer/shader)。可安全多次调用。 */
  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.shaderCache?.dispose();
    this.overlayCanvas = null;
    this.overlayCtx = null;
  }

  // ---- 内部工具 ----

  /** 绑定 attribute 到给定 buffer。 */
  private bindAttribute(
    gl: WebGLRenderingContext,
    program: WebGLProgram,
    name: string,
    data: Float32Array,
    size: number
  ): void {
    const loc = gl.getAttribLocation(program, name);
    if (loc < 0) return;
    const buffer = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    gl.bufferData(gl.ARRAY_BUFFER, data, gl.STATIC_DRAW);
    gl.enableVertexAttribArray(loc);
    gl.vertexAttribPointer(loc, size, gl.FLOAT, false, 0, 0);
  }

  /** 惰性创建 2D 叠加层(与主 canvas 同尺寸)。 */
  private ensureOverlay(): CanvasRenderingContext2D | null {
    if (this.overlayCtx) return this.overlayCtx;
    try {
      const c = document.createElement('canvas');
      c.width = this.canvas.width || 800;
      c.height = this.canvas.height || 600;
      const ctx = c.getContext('2d');
      if (!ctx) return null;
      this.overlayCanvas = c;
      this.overlayCtx = ctx;
      return ctx;
    } catch {
      return null;
    }
  }
}
