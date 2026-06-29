import vue from "@vitejs/plugin-vue";
import vueJsx from "@vitejs/plugin-vue-jsx";
import { defineConfig } from "vite";

// Creates the browser bundle that later can be embedded by the Rust side.
export default defineConfig({
  plugins: [vue(), vueJsx()],
});
