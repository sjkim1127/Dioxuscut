import { defineConfig } from 'vite';

// Keep the browser ecosystem payload cacheable and out of the Studio entry
// chunk. Tauri and ordinary Vite preview use the same split output.
export default defineConfig({
  build: {
    rollupOptions: {
      output: {
        manualChunks: {
          three: ['three'],
        },
      },
    },
  },
});
