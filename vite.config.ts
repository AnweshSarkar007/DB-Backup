import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Vite config tuned for Tauri: fixed dev port, no clearing the terminal (so Rust logs stay visible).
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    watch: {
      ignored: ["**/target/**"],
    },
  },
  build: {
    outDir: "dist",
    target: "es2021",
    sourcemap: false,
  },
});
