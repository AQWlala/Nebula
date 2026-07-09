/**
 * T-E-D-08: 场景图与对象管理。
 *
 * Scene 维护一个 SceneNode 集合,提供增删改查 + 可见性剔除。
 * SceneNode 支持 Group 嵌套(树形结构),每个节点携带自身的
 * position/rotation/scale/color,可由上层映射为蜂群节点、知识图谱实体等。
 *
 * 变换矩阵由本模块按需计算(列主序 4x4),供 renderer 上传 uniform。
 */

import type { Camera } from './Camera';

/** 节点类型。Group 用于组织子节点,本身不直接绘制。 */
export type SceneNodeType = 'Point' | 'Line' | 'Triangle' | 'Group';

/** RGBA 颜色,各分量 0..1。 */
export type RGBA = [number, number, number, number];

/** 三维向量。 */
export type Vec3 = [number, number, number];

/** 场景节点。data 字段供上层挂载业务数据(如记忆 id、关系维度)。 */
export interface SceneNode {
  /** 唯一标识。 */
  id: string;
  /** 图元类型。 */
  type: SceneNodeType;
  /** 世界坐标位置(节点局部原点)。 */
  position: Vec3;
  /** 旋转(弧度,[x,y,z])。 */
  rotation: Vec3;
  /** 缩放([x,y,z])。 */
  scale: Vec3;
  /** 颜色 RGBA(0..1)。 */
  color: RGBA;
  /** 是否可见。false 时连同子节点一并跳过。 */
  visible: boolean;
  /** 子节点(Group 类型通常有子节点)。 */
  children: SceneNode[];
  /** 业务数据载荷,类型由上层决定。 */
  data: unknown;
}

/** 创建 SceneNode 的便捷工厂,提供合理默认值。 */
export function createSceneNode(id: string, type: SceneNodeType, partial: Partial<SceneNode> = {}): SceneNode {
  return {
    id,
    type,
    position: partial.position ?? [0, 0, 0],
    rotation: partial.rotation ?? [0, 0, 0],
    scale: partial.scale ?? [1, 1, 1],
    color: partial.color ?? [1, 1, 1, 1],
    visible: partial.visible ?? true,
    children: partial.children ?? [],
    data: partial.data ?? null,
  };
}

// ---------------------------------------------------------------------------
// 矩阵运算(与本模块需求最小集,列主序 4x4)
// ---------------------------------------------------------------------------

function matIdentity(): Float32Array {
  const m = new Float32Array(16);
  m[0] = 1;
  m[5] = 1;
  m[10] = 1;
  m[15] = 1;
  return m;
}

function matMultiply(a: Float32Array, b: Float32Array): Float32Array {
  const out = new Float32Array(16);
  for (let col = 0; col < 4; col++) {
    for (let row = 0; row < 4; row++) {
      out[col * 4 + row] =
        a[row] * b[col * 4] +
        a[4 + row] * b[col * 4 + 1] +
        a[8 + row] * b[col * 4 + 2] +
        a[12 + row] * b[col * 4 + 3];
    }
  }
  return out;
}

function matTranslate(tx: number, ty: number, tz: number): Float32Array {
  const m = matIdentity();
  m[12] = tx;
  m[13] = ty;
  m[14] = tz;
  return m;
}

function matScale(sx: number, sy: number, sz: number): Float32Array {
  const m = new Float32Array(16);
  m[0] = sx;
  m[5] = sy;
  m[10] = sz;
  m[15] = 1;
  return m;
}

function matRotateX(rad: number): Float32Array {
  const c = Math.cos(rad);
  const s = Math.sin(rad);
  const m = matIdentity();
  m[5] = c;
  m[6] = s;
  m[9] = -s;
  m[10] = c;
  return m;
}

function matRotateY(rad: number): Float32Array {
  const c = Math.cos(rad);
  const s = Math.sin(rad);
  const m = matIdentity();
  m[0] = c;
  m[2] = -s;
  m[8] = s;
  m[10] = c;
  return m;
}

function matRotateZ(rad: number): Float32Array {
  const c = Math.cos(rad);
  const s = Math.sin(rad);
  const m = matIdentity();
  m[0] = c;
  m[1] = s;
  m[4] = -s;
  m[5] = c;
  return m;
}

/**
 * 计算节点的局部模型矩阵(translation × rotation × scale)。
 * 不含父级变换;若需世界矩阵,由上层逐级相乘。
 */
export function getLocalMatrix(node: SceneNode): Float32Array {
  const t = matTranslate(node.position[0], node.position[1], node.position[2]);
  const rx = matRotateX(node.rotation[0]);
  const ry = matRotateY(node.rotation[1]);
  const rz = matRotateZ(node.rotation[2]);
  const s = matScale(node.scale[0], node.scale[1], node.scale[2]);
  // T * Rz * Ry * Rx * S
  return matMultiply(matMultiply(matMultiply(matMultiply(t, rz), ry), rx), s);
}

/**
 * 场景图:管理 SceneNode 集合。
 *
 * 顶层节点存储在 roots 数组中;add/remove 操作顶层节点,
 * 子节点通过 node.children 直接挂载(树形结构)。
 */
export class Scene {
  private readonly roots = new Map<string, SceneNode>();
  /** 按需缓存的扁平化节点列表(含子节点),update 后失效。 */
  private flatCache: SceneNode[] | null = null;

  /** 添加顶层节点。若 id 已存在则覆盖。 */
  add(node: SceneNode): void {
    this.roots.set(node.id, node);
    this.flatCache = null;
  }

  /** 移除顶层节点(及其全部子节点)。 */
  remove(node: SceneNode): void {
    this.roots.delete(node.id);
    this.flatCache = null;
  }

  /** 按 id 移除顶层节点。 */
  removeById(id: string): void {
    if (this.roots.delete(id)) {
      this.flatCache = null;
    }
  }

  /** 清空所有节点。 */
  clear(): void {
    this.roots.clear();
    this.flatCache = null;
  }

  /** 按 id 查找顶层节点。 */
  getNode(id: string): SceneNode | undefined {
    return this.roots.get(id);
  }

  /** 递归查找节点(含子节点)。 */
  findNode(id: string): SceneNode | undefined {
    return this.findIn(this.roots.values(), id);
  }

  private findIn(iter: IterableIterator<SceneNode>, id: string): SceneNode | undefined {
    for (const node of iter) {
      if (node.id === id) return node;
      if (node.children.length > 0) {
        const found = this.findIn(node.children.values(), id);
        if (found) return found;
      }
    }
    return undefined;
  }

  /** 顶层节点数量。 */
  get size(): number {
    return this.roots.size;
  }

  /** 扁平化所有节点(含子节点,深度优先)。 */
  flatten(): SceneNode[] {
    if (this.flatCache) return this.flatCache;
    const out: SceneNode[] = [];
    for (const node of this.roots.values()) {
      this.flattenInto(node, out);
    }
    this.flatCache = out;
    return out;
  }

  private flattenInto(node: SceneNode, out: SceneNode[]): void {
    out.push(node);
    for (const child of node.children) {
      this.flattenInto(child, out);
    }
  }

  /**
   * 每帧更新:遍历节点,调用可选的 onUpdate 回调。
   * deltaTime 单位为秒。当前实现保留扩展点,节点自身无内置动画。
   */
  update(deltaTime: number): void {
    void deltaTime; // 当前无逐帧逻辑,保留接口
    this.flatCache = null;
  }

  /**
   * 返回可见节点列表。
   * 当前实现采用简单策略:visible=true 且投影到屏幕后在视口内的节点。
   * Group 节点视为"总是可见"(其子节点单独判断),以支持容器层级。
   */
  getVisibleNodes(camera: Camera): SceneNode[] {
    const all = this.flatten();
    const visible: SceneNode[] = [];
    const halfW = camera.width / 2;
    const halfH = camera.height / 2;
    for (const node of all) {
      if (!node.visible) continue;
      if (node.type === 'Group') {
        visible.push(node);
        continue;
      }
      const [sx, sy] = camera.project(node.position);
      // 视口剔除(留 10% 余量避免边缘抖动)
      const margin = 0.1;
      if (
        sx >= -halfW * margin &&
        sx <= camera.width + halfW * margin &&
        sy >= -halfH * margin &&
        sy <= camera.height + halfH * margin
      ) {
        visible.push(node);
      }
    }
    return visible;
  }
}
