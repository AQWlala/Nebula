/**
 * T-E-D-08: WebGL 渲染引擎模块 — 统一导出。
 *
 * 为蜂群可视化、知识图谱、记忆地图等场景提供 GPU 加速渲染能力。
 *
 * 用法:
 * ```ts
 * import { WebGLRenderer, Scene, Camera, createSceneNode } from '@/lib/webgl';
 * ```
 */

// 渲染器
export {
  WebGLRenderer,
  DEFAULT_RENDERER_OPTIONS,
  DEFAULT_POINT_OPTIONS,
  DEFAULT_LINE_OPTIONS,
  DEFAULT_TEXT_OPTIONS,
  INITIAL_STATS,
} from './WebGLRenderer';
export type {
  RendererOptions,
  PointDrawOptions,
  LineDrawOptions,
  TextDrawOptions,
  RendererStats,
} from './WebGLRenderer';

// 着色器
export {
  ShaderCache,
  compileShader,
  createProgram,
  POINT_VERTEX_SHADER,
  POINT_FRAGMENT_SHADER,
  LINE_VERTEX_SHADER,
  LINE_FRAGMENT_SHADER,
  TRIANGLE_VERTEX_SHADER,
  TRIANGLE_FRAGMENT_SHADER,
  SHADER_TYPES,
} from './shaders';

// 场景
export { Scene, createSceneNode, getLocalMatrix } from './Scene';
export type { SceneNode, SceneNodeType, RGBA, Vec3 } from './Scene';

// 摄像机
export { Camera } from './Camera';
export type { CameraType, CameraOptions, Vec2 } from './Camera';
