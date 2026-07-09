/**
 * T-E-D-08: WebGL 着色器源码集合 + 编译/链路工具 + 程序缓存。
 *
 * 为蜂群可视化、知识图谱、记忆地图等场景提供基础着色器:
 * - POINT_*: 点精灵(节点)
 * - LINE_*:  线段(边)
 * - TRIANGLE_*: 三角形(填充面)
 *
 * 顶点着色器统一接收 attribute vec2/vec3 aPosition + attribute vec4 aColor,
 * 输出 varying vColor;片段着色器输出 gl_FragColor = vColor。
 * 这种极简接口让上层渲染器可以用同一套 buffer 布局驱动三种图元。
 */

/** 点精灵顶点着色器(带 pointSize uniform)。 */
export const POINT_VERTEX_SHADER = `
attribute vec3 aPosition;
attribute vec4 aColor;
uniform float uPointSize;
varying vec4 vColor;
void main() {
  gl_Position = vec4(aPosition, 1.0);
  gl_PointSize = uPointSize;
  vColor = aColor;
}
`;

/** 点精灵片段着色器(可选圆形平滑:smooth 开启时丢弃圆外像素)。 */
export const POINT_FRAGMENT_SHADER = `
precision mediump float;
uniform bool uSmooth;
varying vec4 vColor;
void main() {
  if (uSmooth) {
    // gl_PointCoord ∈ [0,1]²,中心为 (0.5,0.5)
    vec2 c = gl_PointCoord - vec2(0.5);
    float d = dot(c, c);
    if (d > 0.25) discard;
    // 边缘抗锯齿:0.25-0.22 之间线性过渡
    float alpha = 1.0 - smoothstep(0.22, 0.25, d);
    gl_FragColor = vec4(vColor.rgb, vColor.a * alpha);
  } else {
    gl_FragColor = vColor;
  }
}
`;

/** 线段顶点着色器。 */
export const LINE_VERTEX_SHADER = `
attribute vec3 aPosition;
attribute vec4 aColor;
varying vec4 vColor;
void main() {
  gl_Position = vec4(aPosition, 1.0);
  vColor = aColor;
}
`;

/** 线段片段着色器。 */
export const LINE_FRAGMENT_SHADER = `
precision mediump float;
varying vec4 vColor;
void main() {
  gl_FragColor = vColor;
}
`;

/** 三角形顶点着色器。 */
export const TRIANGLE_VERTEX_SHADER = `
attribute vec3 aPosition;
attribute vec4 aColor;
varying vec4 vColor;
void main() {
  gl_Position = vec4(aPosition, 1.0);
  vColor = aColor;
}
`;

/** 三角形片段着色器。 */
export const TRIANGLE_FRAGMENT_SHADER = `
precision mediump float;
varying vec4 vColor;
void main() {
  gl_FragColor = vColor;
}
`;

/** 着色器类型枚举(便于上层引用,避免直接 import WebGL 常量)。 */
export const SHADER_TYPES = {
  VERTEX: 0x8b31, // gl.VERTEX_SHADER
  FRAGMENT: 0x8b30, // gl.FRAGMENT_SHADER
} as const;

/**
 * 编译单个着色器。
 * 失败时抛出 Error,包含 shader info log 便于调试。
 */
export function compileShader(
  gl: WebGLRenderingContext,
  type: number,
  source: string
): WebGLShader {
  const shader = gl.createShader(type);
  if (!shader) {
    throw new Error('WebGL: createShader 返回 null(上下文可能已丢失)');
  }
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  const compiled = gl.getShaderParameter(shader, gl.COMPILE_STATUS);
  if (!compiled) {
    const log = gl.getShaderInfoLog(shader) ?? '(无 info log)';
    gl.deleteShader(shader);
    throw new Error(`WebGL: 着色器编译失败 — ${log}`);
  }
  return shader;
}

/**
 * 链接 vs + fs 为一个 program。
 * 链接失败时删除 program 并抛出 Error。
 */
export function createProgram(
  gl: WebGLRenderingContext,
  vsSource: string,
  fsSource: string
): WebGLProgram {
  const vs = compileShader(gl, gl.VERTEX_SHADER, vsSource);
  const fs = compileShader(gl, gl.FRAGMENT_SHADER, fsSource);

  const program = gl.createProgram();
  if (!program) {
    throw new Error('WebGL: createProgram 返回 null(上下文可能已丢失)');
  }
  gl.attachShader(program, vs);
  gl.attachShader(program, fs);
  gl.linkProgram(program);
  const linked = gl.getProgramParameter(program, gl.LINK_STATUS);
  if (!linked) {
    const log = gl.getProgramInfoLog(program) ?? '(无 info log)';
    gl.deleteProgram(program);
    throw new Error(`WebGL: program 链接失败 — ${log}`);
  }
  // 链接后即可删除 shader 对象(GPU 已持有编译产物)
  gl.deleteShader(vs);
  gl.deleteShader(fs);
  return program;
}

/** ShaderCache 的 key:由 vs + fs 源码拼接而成,保证同源命中。 */
function programKey(vsSource: string, fsSource: string): string {
  // 长度前缀避免简单拼接导致的碰撞
  return `${vsSource.length}:${vsSource}|${fsSource.length}:${fsSource}`;
}

/**
 * 着色器程序缓存。
 *
 * 同一组 (vs, fs) 源码只会编译链接一次,后续 getProgram 直接返回缓存。
 * 用于 WebGLRenderer 的多种图元绘制路径,避免每帧重复编译。
 */
export class ShaderCache {
  private readonly gl: WebGLRenderingContext;
  private readonly cache = new Map<string, WebGLProgram>();

  constructor(gl: WebGLRenderingContext) {
    this.gl = gl;
  }

  /** 获取(必要时创建)指定 vs+fs 对应的 program。 */
  getProgram(vsSource: string, fsSource: string): WebGLProgram {
    const key = programKey(vsSource, fsSource);
    const cached = this.cache.get(key);
    if (cached) return cached;
    const program = createProgram(this.gl, vsSource, fsSource);
    this.cache.set(key, program);
    return program;
  }

  /** 查询缓存中是否已存在指定 program(供测试断言缓存命中)。 */
  has(vsSource: string, fsSource: string): boolean {
    return this.cache.has(programKey(vsSource, fsSource));
  }

  /** 当前缓存的 program 数量。 */
  get size(): number {
    return this.cache.size;
  }

  /** 释放所有缓存的 program(供 renderer.dispose 调用)。 */
  dispose(): void {
    for (const program of this.cache.values()) {
      this.gl.deleteProgram(program);
    }
    this.cache.clear();
  }
}
