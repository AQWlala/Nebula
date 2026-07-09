/**
 * T-E-D-08: WebGL 引擎模块单元测试。
 *
 * jsdom 无 WebGL 上下文,WebGLRenderer 在构造时自动降级为 no-op 模式,
 * 因此大部分测试可在无 WebGL 环境下运行。
 * ShaderCache 测试使用手工 mock 的 WebGLRenderingContext。
 */
import { describe, it, expect, beforeEach } from 'vitest';
import {
  WebGLRenderer,
  Scene,
  Camera,
  createSceneNode,
  getLocalMatrix,
  ShaderCache,
  DEFAULT_RENDERER_OPTIONS,
  DEFAULT_POINT_OPTIONS,
  DEFAULT_TEXT_OPTIONS,
  INITIAL_STATS,
  POINT_VERTEX_SHADER,
  POINT_FRAGMENT_SHADER,
  LINE_VERTEX_SHADER,
  LINE_FRAGMENT_SHADER,
  TRIANGLE_VERTEX_SHADER,
  TRIANGLE_FRAGMENT_SHADER,
} from '../index';

// ---------------------------------------------------------------------------
// Mock WebGL 上下文(仅供 ShaderCache 测试使用)
// ---------------------------------------------------------------------------

/** 创建最小可用 WebGLRenderingContext mock。 */
function createMockGL(): WebGLRenderingContext {
  const shaders: Record<number, { source: string; compiled: boolean }> = {};
  const programs: Record<number, { shaders: number[]; linked: boolean }> = {};
  let shaderId = 1;
  let programId = 1;
  const mock = {
    VERTEX_SHADER: 0x8b31,
    FRAGMENT_SHADER: 0x8b30,
    COMPILE_STATUS: 0x8b81,
    LINK_STATUS: 0x8b82,
    createShader: (type: number) => {
      const id = shaderId++;
      shaders[id] = { source: '', compiled: true };
      void type;
      return id;
    },
    shaderSource: (shader: number, source: string) => {
      if (shaders[shader]) shaders[shader].source = source;
    },
    compileShader: (shader: number) => {
      if (shaders[shader]) shaders[shader].compiled = true;
    },
    getShaderParameter: (shader: number, _param: number) => {
      return shaders[shader]?.compiled ?? false;
    },
    getShaderInfoLog: (_shader: number) => null,
    deleteShader: (shader: number) => {
      delete shaders[shader];
    },
    createProgram: () => {
      const id = programId++;
      programs[id] = { shaders: [], linked: true };
      return id;
    },
    attachShader: (program: number, shader: number) => {
      if (programs[program]) programs[program].shaders.push(shader);
    },
    linkProgram: (program: number) => {
      if (programs[program]) programs[program].linked = true;
    },
    getProgramParameter: (program: number, _param: number) => {
      return programs[program]?.linked ?? false;
    },
    getProgramInfoLog: (_program: number) => null,
    deleteProgram: (program: number) => {
      delete programs[program];
    },
  };
  return mock as unknown as WebGLRenderingContext;
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

describe('WebGLRenderer (T-E-D-08)', () => {
  let canvas: HTMLCanvasElement;

  beforeEach(() => {
    canvas = document.createElement('canvas');
    canvas.width = 800;
    canvas.height = 600;
  });

  // 1. RendererOptions 默认值
  it('RendererOptions 默认值正确', () => {
    const renderer = new WebGLRenderer(canvas);
    expect(renderer.options.antialias).toBe(DEFAULT_RENDERER_OPTIONS.antialias);
    expect(renderer.options.alpha).toBe(DEFAULT_RENDERER_OPTIONS.alpha);
    expect(renderer.options.premultipliedAlpha).toBe(
      DEFAULT_RENDERER_OPTIONS.premultipliedAlpha
    );
    expect(renderer.options.preserveDrawingBuffer).toBe(
      DEFAULT_RENDERER_OPTIONS.preserveDrawingBuffer
    );
    // 默认值断言
    expect(DEFAULT_RENDERER_OPTIONS.antialias).toBe(true);
    expect(DEFAULT_RENDERER_OPTIONS.alpha).toBe(false);
    expect(DEFAULT_RENDERER_OPTIONS.premultipliedAlpha).toBe(true);
    expect(DEFAULT_RENDERER_OPTIONS.preserveDrawingBuffer).toBe(true);
  });

  // 2. SceneNode 变换矩阵
  it('SceneNode 变换矩阵包含 position 与 scale', () => {
    const node = createSceneNode('n1', 'Point', {
      position: [10, 20, 30],
      scale: [2, 3, 4],
    });
    const m = getLocalMatrix(node);
    // 列主序:translation 在 m[12],m[13],m[14]
    expect(m[12]).toBeCloseTo(10, 6);
    expect(m[13]).toBeCloseTo(20, 6);
    expect(m[14]).toBeCloseTo(30, 6);
    // scale 在对角线 m[0],m[5],m[10]
    expect(m[0]).toBeCloseTo(2, 6);
    expect(m[5]).toBeCloseTo(3, 6);
    expect(m[10]).toBeCloseTo(4, 6);
  });

  // 3. Scene add/remove/clear
  it('Scene 支持增删与清空', () => {
    const scene = new Scene();
    expect(scene.size).toBe(0);
    const a = createSceneNode('a', 'Point');
    const b = createSceneNode('b', 'Point');
    scene.add(a);
    scene.add(b);
    expect(scene.size).toBe(2);
    expect(scene.getNode('a')).toBe(a);
    scene.remove(a);
    expect(scene.size).toBe(1);
    expect(scene.getNode('a')).toBeUndefined();
    scene.clear();
    expect(scene.size).toBe(0);
  });

  // 4. Camera 2D project/unproject 往返
  it('Camera 2D project 与 unproject 互为逆运算', () => {
    const cam = new Camera('2d', { width: 800, height: 600, zoom: 2 });
    const world: [number, number, number] = [50, -30, 0];
    const screen = cam.project(world);
    const back = cam.unproject(screen[0], screen[1]);
    expect(back[0]).toBeCloseTo(world[0], 4);
    expect(back[1]).toBeCloseTo(world[1], 4);
  });

  // 5. Camera zoom/pan
  it('Camera 2D zoom 与 pan 改变 target/zoomLevel', () => {
    const cam = new Camera('2d', { width: 800, height: 600 });
    const initialZoom = cam.zoomLevel;
    cam.zoom(2.0);
    expect(cam.zoomLevel).toBeCloseTo(initialZoom * 2, 6);
    cam.pan(100, 50);
    // pan 后 target 应变化(2D 模式 worldDx = -dx/zoom)
    expect(cam.target[0]).not.toBe(0);
    expect(cam.target[1]).not.toBe(0);
  });

  // 6. Camera fitBounds
  it('Camera 2D fitBounds 调整 zoomLevel 与 target', () => {
    const cam = new Camera('2d', { width: 800, height: 600 });
    cam.fitBounds([0, 0], [400, 300]);
    // 中心点
    expect(cam.target[0]).toBeCloseTo(200, 6);
    expect(cam.target[1]).toBeCloseTo(150, 6);
    // zoomLevel 应使范围填满视口(留 10% 边距 → *0.9)
    // width 400 → zoomX = 800/400 = 2; height 300 → zoomY = 600/300 = 2; min=2; *0.9 = 1.8
    expect(cam.zoomLevel).toBeCloseTo(1.8, 4);
  });

  // 7. RendererStats 初始值
  it('RendererStats 初始值为零', () => {
    const renderer = new WebGLRenderer(canvas);
    const stats = renderer.getStats();
    expect(stats).toEqual(INITIAL_STATS);
    expect(stats.fps).toBe(0);
    expect(stats.drawCalls).toBe(0);
    expect(stats.triangles).toBe(0);
    expect(stats.points).toBe(0);
    expect(stats.lines).toBe(0);
  });

  // 8. ShaderCache 缓存命中
  it('ShaderCache 同源 program 仅编译一次', () => {
    const gl = createMockGL();
    const cache = new ShaderCache(gl);
    expect(cache.size).toBe(0);
    const p1 = cache.getProgram(POINT_VERTEX_SHADER, POINT_FRAGMENT_SHADER);
    expect(cache.size).toBe(1);
    const p2 = cache.getProgram(POINT_VERTEX_SHADER, POINT_FRAGMENT_SHADER);
    expect(p2).toBe(p1); // 同一对象引用 → 命中缓存
    expect(cache.size).toBe(1);
    expect(cache.has(POINT_VERTEX_SHADER, POINT_FRAGMENT_SHADER)).toBe(true);
    // 不同源 → 新建
    cache.getProgram(LINE_VERTEX_SHADER, LINE_FRAGMENT_SHADER);
    expect(cache.size).toBe(2);
    cache.dispose();
    expect(cache.size).toBe(0);
  });

  // 9. PointDrawOptions 默认值
  it('PointDrawOptions 默认值正确', () => {
    expect(DEFAULT_POINT_OPTIONS.size).toBe(4);
    expect(DEFAULT_POINT_OPTIONS.smooth).toBe(true);
  });

  // 10. TextDrawOptions 默认值
  it('TextDrawOptions 默认值正确', () => {
    expect(DEFAULT_TEXT_OPTIONS.font).toBe('12px system-ui');
    expect(DEFAULT_TEXT_OPTIONS.color).toBe('#ffffff');
    expect(DEFAULT_TEXT_OPTIONS.size).toBe(12);
  });

  // 11. SceneNode 可见性过滤
  it('Scene.getVisibleNodes 过滤不可见节点', () => {
    const cam = new Camera('2d', { width: 800, height: 600, zoom: 1 });
    const scene = new Scene();
    scene.add(
      createSceneNode('vis', 'Point', { position: [0, 0, 0], visible: true })
    );
    scene.add(
      createSceneNode('invis', 'Point', { position: [0, 0, 0], visible: false })
    );
    const visible = scene.getVisibleNodes(cam);
    const ids = visible.map((n) => n.id);
    expect(ids).toContain('vis');
    expect(ids).not.toContain('invis');
  });

  // 12. Camera 3D 视图矩阵
  it('Camera 3D 视图矩阵为 4x4 列主序', () => {
    const cam = new Camera('3d', { width: 800, height: 600 });
    const view = cam.getViewMatrix();
    expect(view.length).toBe(16);
    // 3D 视图矩阵应为有效矩阵(非全零)
    const some = Array.from(view).some((v) => v !== 0);
    expect(some).toBe(true);
    // 投影矩阵也应为 16 长度
    const proj = cam.getProjectionMatrix();
    expect(proj.length).toBe(16);
  });

  // 额外:Renderer 在无 WebGL 环境下安全降级
  it('Renderer 在无 WebGL 环境(jsdom)下安全降级', () => {
    const renderer = new WebGLRenderer(canvas);
    // jsdom 无 WebGL,gl 应为 null
    if (renderer.gl === null) {
      // draw 调用应计数但不抛错
      renderer.drawPoints(
        new Float32Array([0, 0, 0]),
        new Float32Array([1, 1, 1, 1])
      );
      renderer.drawLines(
        new Float32Array([0, 0, 0, 1, 1, 0]),
        new Float32Array([1, 1, 1, 1, 1, 1, 1, 1])
      );
      renderer.drawTriangles(
        new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]),
        new Float32Array([1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1])
      );
      const stats = renderer.getStats();
      expect(stats.drawCalls).toBe(3);
      expect(stats.points).toBe(1);
      expect(stats.lines).toBe(1);
      expect(stats.triangles).toBe(1);
    }
    // screenshot 应返回 dataURL 字符串(不抛错)
    const url = renderer.screenshot();
    expect(typeof url).toBe('string');
    expect(url.startsWith('data:')).toBe(true);
    // dispose 可安全调用
    expect(() => renderer.dispose()).not.toThrow();
  });

  // 额外:drawScene 在降级模式下不抛错且更新 stats
  it('drawScene 在降级模式下安全执行', () => {
    const renderer = new WebGLRenderer(canvas);
    const cam = new Camera('2d', { width: 800, height: 600 });
    const scene = new Scene();
    scene.add(createSceneNode('p1', 'Point', { position: [0, 0, 0] }));
    scene.add(createSceneNode('p2', 'Point', { position: [100, 100, 0] }));
    expect(() => renderer.drawScene(scene, cam)).not.toThrow();
    const stats = renderer.getStats();
    // drawScene 至少触发一次 drawPoints(2 个点)
    expect(stats.drawCalls).toBeGreaterThanOrEqual(1);
    expect(stats.points).toBeGreaterThanOrEqual(2);
  });

  // 额外:着色器源码字符串非空
  it('着色器源码为非空字符串', () => {
    expect(POINT_VERTEX_SHADER.length).toBeGreaterThan(0);
    expect(POINT_FRAGMENT_SHADER.length).toBeGreaterThan(0);
    expect(LINE_VERTEX_SHADER.length).toBeGreaterThan(0);
    expect(LINE_FRAGMENT_SHADER.length).toBeGreaterThan(0);
    expect(TRIANGLE_VERTEX_SHADER.length).toBeGreaterThan(0);
    expect(TRIANGLE_FRAGMENT_SHADER.length).toBeGreaterThan(0);
    // 顶点着色器应包含 aPosition/aColor attribute
    expect(POINT_VERTEX_SHADER).toContain('aPosition');
    expect(POINT_VERTEX_SHADER).toContain('aColor');
    expect(TRIANGLE_VERTEX_SHADER).toContain('aPosition');
  });

  // 额外:Camera 3D rotate 仅在 3D 模式生效
  it('Camera 3D rotate 改变 rotation,2D 模式为 no-op', () => {
    const cam3d = new Camera('3d');
    const before3d = [...cam3d.rotation];
    cam3d.rotate(0.5, 0.3);
    expect(cam3d.rotation[0]).toBeCloseTo(before3d[0] + 0.5, 6);
    expect(cam3d.rotation[1]).toBeCloseTo(before3d[1] + 0.3, 6);

    const cam2d = new Camera('2d');
    const before2d = [...cam2d.rotation];
    cam2d.rotate(0.5, 0.3);
    expect(cam2d.rotation).toEqual(before2d);
  });

  // 额外:Scene 支持子节点递归查找与扁平化
  it('Scene 支持子节点递归查找与扁平化', () => {
    const scene = new Scene();
    const parent = createSceneNode('parent', 'Group');
    const child = createSceneNode('child', 'Point', { position: [1, 2, 3] });
    parent.children.push(child);
    scene.add(parent);
    // 顶层查找
    expect(scene.getNode('parent')).toBe(parent);
    expect(scene.getNode('child')).toBeUndefined();
    // 递归查找
    expect(scene.findNode('child')).toBe(child);
    // 扁平化应包含 parent + child
    const flat = scene.flatten();
    expect(flat.length).toBe(2);
    expect(flat.map((n) => n.id)).toContain('child');
  });
});
