// PoW 솔버 래퍼 — worker를 띄워 challenge를 풀고 nonce를 받는다.
import type { PowRequest, PowResult } from "./pow.worker.ts";

/** challenge+difficulty를 worker에서 풀어 nonce를 반환. 진행상황은 onProgress(선택). */
export function solvePow(challenge: string, difficulty: number): Promise<PowResult> {
  return new Promise((resolve, reject) => {
    const worker = new Worker(new URL("./pow.worker.ts", import.meta.url), { type: "module" });
    worker.onmessage = (e: MessageEvent<PowResult>) => {
      resolve(e.data);
      worker.terminate();
    };
    worker.onerror = (e) => {
      reject(new Error(`pow worker error: ${e.message}`));
      worker.terminate();
    };
    const req: PowRequest = { challenge, difficulty };
    worker.postMessage(req);
  });
}
