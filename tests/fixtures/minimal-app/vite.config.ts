import { defineConfig } from "vite";

export default defineConfig({
  // 关闭启动清屏，避免 axctl dev 输出被清掉
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    host: "127.0.0.1",
  },
});
