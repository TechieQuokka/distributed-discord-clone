import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// backend(=server bin)의 REST/Gateway 주소. 단일 포트(REST_ADDR)에 REST와 WS(/gateway)가 함께 산다.
// env `BACKEND_URL`로 덮어쓸 수 있다(멀티노드 시 특정 노드 지정). 기본 = 단일노드 dev.
const BACKEND = process.env.BACKEND_URL ?? "http://127.0.0.1:8080";
const WS_BACKEND = BACKEND.replace(/^http/, "ws");

// dev proxy로 CORS를 우회한다(규칙1: backend 무수정 / 규칙3: 독립).
//   브라우저 → http://localhost:5173/api/...    → (strip /api) → BACKEND/...
//   브라우저 → ws://localhost:5173/gateway       → WS_BACKEND/gateway
// 같은 origin이라 CORS 불필요 + CLI와 동일 backend를 물어 상호운용 검증 가능(규칙4).
export default defineConfig({
  plugins: [react(), tailwindcss()],
  server: {
    port: 5173,
    proxy: {
      "/api": {
        target: BACKEND,
        changeOrigin: true,
        rewrite: (path) => path.replace(/^\/api/, ""),
      },
      "/gateway": {
        target: WS_BACKEND,
        ws: true,
        changeOrigin: true,
      },
    },
  },
});
