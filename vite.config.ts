import { defineConfig, type UserConfig } from 'vite';
import preact from '@preact/preset-vite';

const config: UserConfig = {
  plugins: [preact()],

  build: {
    minify: 'terser',
    terserOptions: {
      compress: false,
      mangle: false,
    },
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (id.includes('monaco-editor')) return 'monaco';
          if (id.includes('xterm')) return 'xterm';
          if (id.includes('fuse.js')) return 'fuse';
        },
      },
    },
  },
};

export default defineConfig(config);
