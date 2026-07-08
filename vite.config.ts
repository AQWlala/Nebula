/**
 * Nebula (nebula) 统一 Vite + Vitest 配置。
 *
 * T-D-C-02: 合并 vite.config.mjs 和 vitest.config.ts 为单一配置源。
 * Vitest 自动从 vite.config.ts 读取 test 字段,无需独立的 vitest.config.ts。
 *
 * 包含:
 * - Preact 插件
 * - resolve.alias(react → preact/compat)
 * - 构建优化(manualChunks)
 * - 测试配置(jsdom + coverage v8)
 */
import { defineConfig } from 'vitest/config';
import preact from '@preact/preset-vite';

export default defineConfig({
  plugins: [preact()],
  resolve: {
    alias: {
      react: 'preact/compat',
      'react-dom': 'preact/compat',
    },
  },
  build: {
    rollupOptions: {
      output: {
        // T-S5-B-03: manualChunks 优化 — 将第三方依赖拆分为独立 vendor chunk,
        // 减少首屏主 bundle 体积。组件级拆分由 App.tsx 中的 lazy() 实现。
        manualChunks(id) {
          if (id.includes('monaco-editor') || id.includes('@monaco-editor')) return 'monaco';
          if (id.includes('xterm')) return 'xterm';
          if (id.includes('fuse.js')) return 'fuse';
          // preact 核心 + signals 拆分,主 bundle 不再内联框架代码
          if (id.includes('node_modules/preact/') || id.includes('node_modules/@preact/')) return 'preact';
          // Tauri API 全家桶拆分
          if (id.includes('node_modules/@tauri-apps/')) return 'tauri';
          // markdown 渲染依赖(marked + highlight.js)拆分,
          // 仅 WritingMode 懒加载后才会拉取此 chunk
          if (id.includes('node_modules/marked/') || id.includes('node_modules/highlight.js/')) return 'markdown';
        },
      },
    },
  },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/test-setup.ts'],
    include: [
      'src/**/__tests__/**/*.{ts,tsx}',
      'src/**/*.{test,spec}.{ts,tsx}',
    ],
    exclude: ['node_modules', 'dist', 'e2e'],
    coverage: {
      provider: 'v8',
      reporter: ['text', 'json', 'html', 'lcov'],
      reportsDirectory: './coverage',
      include: ['src/**/*.{ts,tsx}'],
      exclude: [
        'src/**/*.test.{ts,tsx}',
        'src/**/*.spec.{ts,tsx}',
        'src/test-setup.ts',
        'src/main.tsx',
        'src/vite-env.d.ts',
      ],
      thresholds: {
        // T-D-T-01: 基于实际覆盖率基线提升阈值(2026-07-09)
        // 实测: Stmts 38.75% / Branch 59.31% / Funcs 28.51% / Lines 38.75%
        // 留 ~3-9% 余量防止 CI flaky,后续补充测试后逐步提升至 50%+
        lines: 35,
        functions: 25,
        branches: 50,
        statements: 35,
      },
    },
  },
});
