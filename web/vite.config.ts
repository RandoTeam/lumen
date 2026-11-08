import { defineConfig } from "vite";
import preact from "@preact/preset-vite";

export default defineConfig({
  base: "./",
  plugins: [preact()],
  server: { host: "127.0.0.1", port: 5173, strictPort: true },
  build: {
    target: "es2022",
    assetsInlineLimit: 8192,
    rollupOptions: {
      output: {
        entryFileNames: "app.js",
        chunkFileNames: "chunk.js",
        assetFileNames: "f-[name][extname]",
      },
    },
  },
});
