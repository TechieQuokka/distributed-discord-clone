// 가입 PoW 퍼즐 솔버 (D18). backend `auth::pow`와 **동일 알고리즘**:
//   nonce(십진 문자열, 0부터)를 찾아 sha256(challenge || ":" || nonce)의 선행 0비트 ≥ difficulty.
// difficulty 18 ≈ 26만 해시 → 메인스레드를 막지 않도록 Web Worker에서 푼다.
import { sha256 } from "@noble/hashes/sha2.js";

const encoder = new TextEncoder();

// digest의 선행 0비트 수 (Rust `leading_zero_bits` 미러).
function leadingZeroBits(bytes: Uint8Array): number {
  let n = 0;
  for (const b of bytes) {
    if (b === 0) {
      n += 8;
    } else {
      // Math.clz32은 32비트 기준 → 바이트(8비트)는 24를 뺀다.
      n += Math.clz32(b) - 24;
      break;
    }
  }
  return n;
}

function satisfies(challenge: string, nonce: string, difficulty: number): boolean {
  const digest = sha256(encoder.encode(`${challenge}:${nonce}`));
  return leadingZeroBits(digest) >= difficulty;
}

export interface PowRequest {
  challenge: string;
  difficulty: number;
}
export interface PowResult {
  nonce: string;
  iterations: number;
  ms: number;
}

self.onmessage = (e: MessageEvent<PowRequest>) => {
  const { challenge, difficulty } = e.data;
  const start = performance.now();
  let nonce = 0;
  // u64 범위지만 현실적으로 difficulty 18은 수십만 내 해결 → Number로 충분.
  for (;;) {
    const s = String(nonce);
    if (satisfies(challenge, s, difficulty)) {
      const result: PowResult = { nonce: s, iterations: nonce + 1, ms: performance.now() - start };
      (self as unknown as Worker).postMessage(result);
      return;
    }
    nonce++;
  }
};
