import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Tauri serves the dev frontend from a fixed port and needs a clean stderr,
// so these are not defaults to be tidied away later.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: { target: "chrome110", sourcemap: true },
});
