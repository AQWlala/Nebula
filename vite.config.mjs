import { defineConfig } from "vite";
import preact from "@preact/preset-vite";

export default defineConfig({
  plugins: [preact()],
  build: {
    rollupOptions: {
      output: {
        // T-S5-B-03: manualChunks 优化 — 将第三方依赖拆分为独立 vendor chunk,
        // 减少首屏主 bundle 体积。组件级拆分由 App.tsx 中的 lazy() 实现。
        manualChunks(id) {
          if (id.includes("monaco-editor") || id.includes("@monaco-editor")) return "monaco";
          if (id.includes("xterm")) return "xterm";
          if (id.includes("fuse.js")) return "fuse";
          // preact 核心 + signals 拆分,主 bundle 不再内联框架代码
          if (id.includes("node_modules/preact/") || id.includes("node_modules/@preact/")) return "preact";
          // Tauri API 全家桶拆分
          if (id.includes("node_modules/@tauri-apps/")) return "tauri";
          // markdown 渲染依赖(marked + highlight.js)拆分,
          // 仅 WritingMode 懒加载后才会拉取此 chunk
          if (id.includes("node_modules/marked/") || id.includes("node_modules/highlight.js/")) return "markdown";
        },
      },
    },
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test-setup.ts"],
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
  },
});
