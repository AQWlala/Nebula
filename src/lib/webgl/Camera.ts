/**
 * T-E-D-08: 2D/3D 摄像机。
 *
 * 设计目标:
 * - 2D 模式:正交投影,支持 pan/zoom/fitBounds,适合蜂群节点、知识图谱的平铺视图
 * - 3D 模式:透视投影,支持 pan/zoom/rotate,适合记忆地图的立体层级视图
 * - project/unproject 互为逆运算,支持鼠标交互(点击命中、框选)
 *
 * 矩阵运算自包含(无 gl-matrix 依赖),保持模块零外部依赖。
 * 所有矩阵采用列主序(Float32Array,长度 16),与 WebGL 上传约定一致。
 *
 * 注意:Camera 类同时需要 `zoom(factor)` 方法与可读的当前缩放值,
 * 二者不能同名,故当前缩放值以 `zoomLevel` 暴露,方法名为 `zoom`。
 */

/** 摄像机类型。 */
export type CameraType = '2d' | '3d';

/** 三维向量/点。 */
export type Vec3 = [number, number, number];

/** 二维向量/点。 */
export type Vec2 = [number, number];

/** 摄像机构造选项。未提供的字段使用默认值。 */
export interface CameraOptions {
  /** 摄像机位置(世界坐标)。2D 默认 [0,0,1];3D 默认 [0,0,5]。 */
  position?: Vec3;
  /** 视线目标点。2D 默认 [0,0,0];3D 默认 [0,0,0]。 */
  target?: Vec3;
  /** up 向量,默认 [0,1,0]。 */
  up?: Vec3;
  /** 透视视野角度(弧度),仅 3D,默认 Math.PI/4。 */
  fov?: number;
  /** 近裁面,默认 0.1。 */
  near?: number;
  /** 远裁面,默认 100。 */
  far?: number;
  /** 初始缩放,2D 模式生效,默认 1。 */
  zoom?: number;
  /** 3D 旋转 [绕 X 轴, 绕 Y 轴](弧度),默认 [0,0]。 */
  rotation?: Vec3;
  /** 视口宽度(像素),默认 800。 */
  width?: number;
  /** 视口高度(像素),默认 600。 */
  height?: number;
}

// ---------------------------------------------------------------------------
// 矩阵运算(列主序,4x4)。仅实现 Camera 所需的最小集合。
// ---------------------------------------------------------------------------

/** 创建 4x4 单位矩阵。 */
function identity(): Float32Array {
  const m = new Float32Array(16);
  m[0] = 1;
  m[5] = 1;
  m[10] = 1;
  m[15] = 1;
  return m;
}

/** 4x4 矩阵乘法: result = a * b。 */
function multiply(a: Float32Array, b: Float32Array): Float32Array {
  const out = new Float32Array(16);
  for (let col = 0; col < 4; col++) {
    for (let row = 0; row < 4; row++) {
      out[col * 4 + row] =
        a[0 * 4 + row] * b[col * 4 + 0] +
        a[1 * 4 + row] * b[col * 4 + 1] +
        a[2 * 4 + row] * b[col * 4 + 2] +
        a[3 * 4 + row] * b[col * 4 + 3];
    }
  }
  return out;
}

/** 4x4 矩阵求逆(高斯-约旦消元)。奇异矩阵返回单位矩阵。 */
function invert(m: Float32Array): Float32Array {
  // 构造增广矩阵 [m | I](8 列 × 4 行,列主序存储为长度 32 数组)
  const aug = new Float32Array(32);
  for (let i = 0; i < 16; i++) aug[i] = m[i];
  aug[16 + 0] = 1;
  aug[16 + 5] = 1;
  aug[16 + 10] = 1;
  aug[16 + 15] = 1;

  for (let col = 0; col < 4; col++) {
    // 选主元
    let pivot = col;
    let maxAbs = Math.abs(aug[col * 4 + col]);
    for (let r = col + 1; r < 4; r++) {
      const v = Math.abs(aug[col * 4 + r]);
      if (v > maxAbs) {
        maxAbs = v;
        pivot = r;
      }
    }
    if (maxAbs < 1e-12) {
      return identity();
    }
    // 交换行
    if (pivot !== col) {
      for (let c = 0; c < 8; c++) {
        const tmp = aug[c * 4 + col];
        aug[c * 4 + col] = aug[c * 4 + pivot];
        aug[c * 4 + pivot] = tmp;
      }
    }
    // 归一化主元行
    const pivotVal = aug[col * 4 + col];
    for (let c = 0; c < 8; c++) {
      aug[c * 4 + col] /= pivotVal;
    }
    // 消去其他行
    for (let r = 0; r < 4; r++) {
      if (r === col) continue;
      const factor = aug[col * 4 + r];
      if (factor === 0) continue;
      for (let c = 0; c < 8; c++) {
        aug[c * 4 + r] -= factor * aug[c * 4 + col];
      }
    }
  }
  const inv = new Float32Array(16);
  for (let i = 0; i < 16; i++) inv[i] = aug[16 + i];
  return inv;
}

/** 平移矩阵。 */
function translation(tx: number, ty: number, tz: number): Float32Array {
  const m = identity();
  m[12] = tx;
  m[13] = ty;
  m[14] = tz;
  return m;
}

/** 缩放矩阵。 */
function scaling(sx: number, sy: number, sz: number): Float32Array {
  const m = new Float32Array(16);
  m[0] = sx;
  m[5] = sy;
  m[10] = sz;
  m[15] = 1;
  return m;
}

/** 绕 X 轴旋转矩阵。 */
function rotationX(rad: number): Float32Array {
  const c = Math.cos(rad);
  const s = Math.sin(rad);
  const m = identity();
  m[5] = c;
  m[6] = s;
  m[9] = -s;
  m[10] = c;
  return m;
}

/** 绕 Y 轴旋转矩阵。 */
function rotationY(rad: number): Float32Array {
  const c = Math.cos(rad);
  const s = Math.sin(rad);
  const m = identity();
  m[0] = c;
  m[2] = -s;
  m[8] = s;
  m[10] = c;
  return m;
}

/** 构造正交投影矩阵(left/right/bottom/top/near/far)。 */
function ortho(
  left: number,
  right: number,
  bottom: number,
  top: number,
  near: number,
  far: number
): Float32Array {
  const m = new Float32Array(16);
  const rl = 1 / (right - left);
  const tb = 1 / (top - bottom);
  const nf = 1 / (near - far);
  m[0] = 2 * rl;
  m[5] = 2 * tb;
  m[10] = 2 * nf;
  m[12] = -(right + left) * rl;
  m[13] = -(top + bottom) * tb;
  m[14] = (far + near) * nf;
  m[15] = 1;
  return m;
}

/** 构造透视投影矩阵(fovy 弧度, aspect = width/height)。 */
function perspective(fovy: number, aspect: number, near: number, far: number): Float32Array {
  const f = 1 / Math.tan(fovy / 2);
  const nf = 1 / (near - far);
  const m = new Float32Array(16);
  m[0] = f / aspect;
  m[5] = f;
  m[10] = (far + near) * nf;
  m[11] = -1;
  m[14] = 2 * far * near * nf;
  return m;
}

/** lookAt 视图矩阵。 */
function lookAt(eye: Vec3, center: Vec3, up: Vec3): Float32Array {
  const zx = eye[0] - center[0];
  const zy = eye[1] - center[1];
  const zz = eye[2] - center[2];
  let zl = Math.hypot(zx, zy, zz);
  if (zl < 1e-12) zl = 1;
  const zxN = zx / zl;
  const zyN = zy / zl;
  const zzN = zz / zl;

  // x = up × z
  const xx = up[1] * zzN - up[2] * zyN;
  const xy = up[2] * zxN - up[0] * zzN;
  const xz = up[0] * zyN - up[1] * zxN;
  let xl = Math.hypot(xx, xy, xz);
  if (xl < 1e-12) xl = 1;
  const xxN = xx / xl;
  const xyN = xy / xl;
  const xzN = xz / xl;

  // y = z × x
  const yx = zyN * xzN - zzN * xyN;
  const yy = zzN * xxN - zxN * xzN;
  const yz = zxN * xyN - zyN * xxN;

  const m = new Float32Array(16);
  m[0] = xxN;
  m[1] = xyN;
  m[2] = xzN;
  m[4] = yx;
  m[5] = yy;
  m[6] = yz;
  m[8] = zxN;
  m[9] = zyN;
  m[10] = zzN;
  m[12] = -(xxN * eye[0] + xyN * eye[1] + xzN * eye[2]);
  m[13] = -(yx * eye[0] + yy * eye[1] + yz * eye[2]);
  m[14] = -(zxN * eye[0] + zyN * eye[1] + zzN * eye[2]);
  m[15] = 1;
  return m;
}

// ---------------------------------------------------------------------------
// Camera
// ---------------------------------------------------------------------------

/**
 * 摄像机:封装视图矩阵 + 投影矩阵,并提供世界↔屏幕坐标互转。
 *
 * 2D 模式采用正交投影 + 平移/缩放;3D 模式采用透视投影 + lookAt + 旋转。
 */
export class Camera {
  readonly type: CameraType;

  position: Vec3;
  target: Vec3;
  up: Vec3;
  fov: number;
  near: number;
  far: number;
  /** 当前缩放值(2D 模式)。用 zoomLevel 而非 zoom,避免与 zoom() 方法同名冲突。 */
  zoomLevel: number;
  rotation: Vec3;
  width: number;
  height: number;

  constructor(type: CameraType, options: CameraOptions = {}) {
    this.type = type;
    this.position = options.position ?? (type === '3d' ? [0, 0, 5] : [0, 0, 1]);
    this.target = options.target ?? [0, 0, 0];
    this.up = options.up ?? [0, 1, 0];
    this.fov = options.fov ?? Math.PI / 4;
    this.near = options.near ?? 0.1;
    this.far = options.far ?? 100;
    this.zoomLevel = options.zoom ?? 1;
    this.rotation = options.rotation ?? [0, 0, 0];
    this.width = options.width ?? 800;
    this.height = options.height ?? 600;
  }

  /** 视图矩阵(世界→视图)。 */
  getViewMatrix(): Float32Array {
    if (this.type === '3d') {
      const base = lookAt(this.position, this.target, this.up);
      const rx = rotationX(this.rotation[0]);
      const ry = rotationY(this.rotation[1]);
      return multiply(multiply(ry, rx), base);
    }
    // 2D:平移到原点 + 缩放(屏幕中心对应 world.target,y 轴翻转)
    const tx = -this.target[0] * this.zoomLevel;
    const ty = this.target[1] * this.zoomLevel; // 翻转 y
    const t = translation(tx, ty, 0);
    const s = scaling(this.zoomLevel, this.zoomLevel, 1);
    return multiply(s, t);
  }

  /** 投影矩阵(视图→裁剪)。 */
  getProjectionMatrix(): Float32Array {
    if (this.type === '3d') {
      const aspect = this.width / Math.max(1, this.height);
      return perspective(this.fov, aspect, this.near, this.far);
    }
    // 2D 正交:以屏幕中心为原点,y 向下
    const halfW = this.width / 2;
    const halfH = this.height / 2;
    return ortho(-halfW, halfW, halfH, -halfH, this.near, this.far);
  }

  /** 视图 × 投影 合并矩阵(世界→裁剪)。 */
  getViewProjectionMatrix(): Float32Array {
    return multiply(this.getProjectionMatrix(), this.getViewMatrix());
  }

  /**
   * 世界坐标 → 屏幕坐标(像素,原点左上)。
   * 流程: world → view → clip → NDC → screen。
   */
  project(point: Vec3): Vec2 {
    const vp = this.getViewProjectionMatrix();
    const cx = vp[0] * point[0] + vp[4] * point[1] + vp[8] * point[2] + vp[12];
    const cy = vp[1] * point[0] + vp[5] * point[1] + vp[9] * point[2] + vp[13];
    const cw = vp[3] * point[0] + vp[7] * point[1] + vp[11] * point[2] + vp[15];
    if (Math.abs(cw) < 1e-12) {
      return [this.width / 2, this.height / 2];
    }
    const ndcX = cx / cw;
    const ndcY = cy / cw;
    // NDC [-1,1] → screen [0,width],[0,height],y 翻转
    const sx = (ndcX + 1) * 0.5 * this.width;
    const sy = (1 - ndcY) * 0.5 * this.height;
    return [sx, sy];
  }

  /**
   * 屏幕坐标(像素,原点左上) → 世界坐标。
   * 2D 模式返回 [wx, wy, 0];3D 模式返回透视展开后的世界点(z 投影到 0 平面)。
   */
  unproject(screenX: number, screenY: number): Vec3 {
    const vp = this.getViewProjectionMatrix();
    const inv = invert(vp);
    // screen → NDC
    const ndcX = (screenX / this.width) * 2 - 1;
    const ndcY = 1 - (screenY / this.height) * 2;
    // NDC → world(取 w=1)
    const wx = inv[0] * ndcX + inv[4] * ndcY + inv[12];
    const wy = inv[1] * ndcX + inv[5] * ndcY + inv[13];
    const wz = inv[2] * ndcX + inv[6] * ndcY + inv[14];
    const ww = inv[3] * ndcX + inv[7] * ndcY + inv[15];
    if (Math.abs(ww) < 1e-12) {
      return [0, 0, 0];
    }
    return [wx / ww, wy / ww, wz / ww];
  }

  /** 平移(屏幕像素增量)。2D 修改 target,3D 修改 eye/target。 */
  pan(dx: number, dy: number): void {
    if (this.type === '3d') {
      const zoomFactor = Math.hypot(
        this.position[0] - this.target[0],
        this.position[1] - this.target[1],
        this.position[2] - this.target[2]
      );
      const k = dx * 0.001 * zoomFactor;
      const ky = dy * 0.001 * zoomFactor;
      this.target = [this.target[0] - k, this.target[1] + ky, this.target[2]];
      this.position = [this.position[0] - k, this.position[1] + ky, this.position[2]];
    } else {
      const worldDx = -dx / this.zoomLevel;
      const worldDy = dy / this.zoomLevel;
      this.target = [this.target[0] + worldDx, this.target[1] + worldDy, this.target[2]];
    }
  }

  /**
   * 缩放。2D 直接调整 zoomLevel;3D 沿视线拉近/拉远 eye。
   * centerX/centerY 为缩放锚点(屏幕坐标),默认视口中心。
   */
  zoom(factor: number, centerX?: number, centerY?: number): void {
    const cx = centerX ?? this.width / 2;
    const cy = centerY ?? this.height / 2;
    if (this.type === '3d') {
      const dx = this.position[0] - this.target[0];
      const dy = this.position[1] - this.target[1];
      const dz = this.position[2] - this.target[2];
      const newDist = Math.hypot(dx, dy, dz) / factor;
      const len = Math.hypot(dx, dy, dz) || 1;
      this.position = [
        this.target[0] + (dx / len) * newDist,
        this.target[1] + (dy / len) * newDist,
        this.target[2] + (dz / len) * newDist,
      ];
      return;
    }
    // 2D:保持缩放锚点对应的世界点不变
    const before = this.unproject(cx, cy);
    this.zoomLevel = Math.max(0.01, this.zoomLevel * factor);
    const after = this.unproject(cx, cy);
    this.target = [
      this.target[0] + (before[0] - after[0]),
      this.target[1] + (before[1] - after[1]),
      this.target[2],
    ];
  }

  /** 3D 旋转(rx 绕 X 轴,ry 绕 Y 轴),2D 模式为 no-op。 */
  rotate(rx: number, ry: number): void {
    if (this.type !== '3d') return;
    this.rotation = [this.rotation[0] + rx, this.rotation[1] + ry, this.rotation[2]];
  }

  /**
   * 调整摄像机使世界范围 [min, max] 完整显示在视口内。
   * 2D:计算 zoomLevel 使范围填满视口,target 设为范围中心。
   * 3D:将 eye 拉远到能看到整个范围。
   */
  fitBounds(min: Vec2, max: Vec2): void {
    const cx = (min[0] + max[0]) / 2;
    const cy = (min[1] + max[1]) / 2;
    const w = Math.max(1e-6, max[0] - min[0]);
    const h = Math.max(1e-6, max[1] - min[1]);
    if (this.type === '3d') {
      const aspect = this.width / Math.max(1, this.height);
      const halfFovH = this.fov * 0.5;
      const halfFovV = Math.atan(Math.tan(halfFovH) * aspect);
      const distW = w / (2 * Math.tan(halfFovH));
      const distH = h / (2 * Math.tan(halfFovV));
      const dist = Math.max(distW, distH) * 1.2; // 留 20% 边距
      this.target = [cx, cy, 0];
      this.position = [cx, cy, dist];
      return;
    }
    const zoomX = this.width / w;
    const zoomY = this.height / h;
    this.zoomLevel = Math.min(zoomX, zoomY) * 0.9; // 留 10% 边距
    this.target = [cx, cy, 0];
  }

  /** 更新视口尺寸(供 renderer.resize 同步)。 */
  setViewport(width: number, height: number): void {
    this.width = width;
    this.height = height;
  }
}
