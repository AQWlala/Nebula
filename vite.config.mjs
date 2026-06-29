import { defineConfig } from "vite";
import preact from "@preact/preset-vite";

export default defineConfig({
  plugins: [preact()],
  build: {
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (id.includes("monaco-editor")) return "monaco";
          if (id.includes("xterm")) return "xterm";
          if (id.includes("fuse.js")) return "fuse";
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
