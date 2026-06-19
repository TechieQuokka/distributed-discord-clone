// 설정 모달 — 내 user id(친구 추가용 공유) + TOTP MFA 설정(enable→verify, D19).
import { useState } from "react";
import { mfaEnable, mfaVerify } from "../api/auth.ts";
import { useAuth } from "../store/auth.ts";
import { ApiError } from "../api/http.ts";

export function SettingsModal({ onClose }: { onClose: () => void }) {
  const userId = useAuth((s) => s.userId);
  const username = useAuth((s) => s.username);
  const [secret, setSecret] = useState<string | null>(null);
  const [uri, setUri] = useState<string | null>(null);
  const [code, setCode] = useState("");
  const [status, setStatus] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const err = (e: unknown) => setError(e instanceof ApiError ? e.message : String(e));

  async function startMfa() {
    setError(null);
    try {
      const v = await mfaEnable();
      setSecret(v.secret);
      setUri(v.otpauth_uri);
      setStatus(null);
    } catch (e) {
      err(e);
    }
  }

  async function verifyMfa() {
    if (!secret || !code.trim()) return;
    setError(null);
    try {
      await mfaVerify(secret, code.trim());
      setStatus("✅ 2단계 인증이 활성화되었습니다.");
      setSecret(null);
      setUri(null);
      setCode("");
    } catch (e) {
      err(e);
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={onClose}>
      <div className="w-[480px] rounded-md bg-bg-app p-6" onClick={(e) => e.stopPropagation()}>
        <h2 className="mb-4 text-xl font-bold text-text-bright">설정</h2>

        <section className="mb-6">
          <h3 className="mb-1 text-xs font-bold uppercase text-text-muted">내 계정</h3>
          <p className="text-text-normal">{username ?? "(이름 없음)"}</p>
          <div className="mt-1 text-xs text-text-muted">
            user id (친구 추가 시 공유):{" "}
            <span className="select-all font-mono text-online">{userId}</span>
          </div>
        </section>

        <section className="mb-4">
          <h3 className="mb-2 text-xs font-bold uppercase text-text-muted">2단계 인증 (TOTP)</h3>
          {!secret && !status && (
            <button onClick={startMfa} className="rounded bg-accent px-4 py-2 text-sm text-white hover:bg-accent-hover">
              2단계 인증 설정 시작
            </button>
          )}
          {secret && (
            <div className="space-y-2">
              <p className="text-sm text-text-muted">인증 앱에 등록:</p>
              <div className="break-all rounded bg-bg-server px-3 py-2 text-xs">
                secret: <span className="select-all font-mono text-online">{secret}</span>
              </div>
              {uri && (
                <div className="break-all rounded bg-bg-server px-3 py-2 text-xs text-text-muted">
                  <span className="select-all">{uri}</span>
                </div>
              )}
              <div className="flex gap-2">
                <input
                  value={code}
                  onChange={(e) => setCode(e.target.value)}
                  placeholder="앱의 6자리 코드"
                  inputMode="numeric"
                  className="flex-1 rounded bg-bg-server px-3 py-2 text-text-bright outline-none"
                />
                <button onClick={verifyMfa} className="rounded bg-online px-4 text-sm text-white">
                  활성화
                </button>
              </div>
            </div>
          )}
          {status && <p className="text-sm text-online">{status}</p>}
        </section>

        {error && <p className="mb-2 text-sm text-dnd">⚠ {error}</p>}

        <div className="flex justify-end">
          <button onClick={onClose} className="rounded bg-bg-input px-4 py-2 text-sm text-text-normal hover:bg-bg-hover">
            닫기
          </button>
        </div>
      </div>
    </div>
  );
}
