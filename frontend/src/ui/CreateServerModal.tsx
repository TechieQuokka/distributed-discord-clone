// 서버 생성 / 초대 코드로 합류 모달.
import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { createGuild, joinInvite } from "../api/guilds.ts";
import { qk } from "../api/queryKeys.ts";
import { useUi } from "../store/ui.ts";
import { ApiError } from "../api/http.ts";

export function CreateServerModal({ onClose }: { onClose: () => void }) {
  const qc = useQueryClient();
  const selectRealm = useUi((s) => s.selectRealm);
  const [tab, setTab] = useState<"create" | "join">("create");
  const [name, setName] = useState("");
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const errText = (e: unknown) =>
    e instanceof ApiError ? e.message : e instanceof Error ? e.message : String(e);

  async function submit() {
    setBusy(true);
    setError(null);
    try {
      let realmId: string;
      if (tab === "create") {
        if (!name.trim()) throw new Error("서버 이름을 입력하세요");
        const g = await createGuild(name.trim());
        realmId = g.id;
      } else {
        if (!code.trim()) throw new Error("초대 코드를 입력하세요");
        const j = await joinInvite(code.trim());
        realmId = j.realm_id;
      }
      await qc.invalidateQueries({ queryKey: qk.realms() });
      selectRealm(realmId);
      onClose();
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60"
      onClick={onClose}
    >
      <div
        className="w-[440px] rounded-md bg-bg-app p-6"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="mb-1 text-center text-xl font-bold text-text-bright">
          {tab === "create" ? "서버 만들기" : "서버 참가하기"}
        </h2>

        <div className="mb-4 flex gap-2 border-b border-bg-server">
          {(["create", "join"] as const).map((t) => (
            <button
              key={t}
              onClick={() => setTab(t)}
              className={`px-3 py-2 text-sm ${
                tab === t ? "border-b-2 border-accent text-text-bright" : "text-text-muted"
              }`}
            >
              {t === "create" ? "새로 만들기" : "초대 코드로 참가"}
            </button>
          ))}
        </div>

        {tab === "create" ? (
          <>
            <label className="mb-1 block text-xs font-bold uppercase text-text-muted">
              서버 이름
            </label>
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              className="mb-4 w-full rounded bg-bg-server px-3 py-2 text-text-bright outline-none focus:ring-2 focus:ring-accent"
              placeholder="내 서버"
              autoFocus
            />
          </>
        ) : (
          <>
            <label className="mb-1 block text-xs font-bold uppercase text-text-muted">
              초대 코드
            </label>
            <input
              value={code}
              onChange={(e) => setCode(e.target.value)}
              className="mb-4 w-full rounded bg-bg-server px-3 py-2 text-text-bright outline-none focus:ring-2 focus:ring-accent"
              placeholder="aB3xY7zQ"
              autoFocus
            />
          </>
        )}

        {error && <p className="mb-3 text-sm text-dnd">⚠ {error}</p>}

        <div className="flex justify-end gap-2">
          <button onClick={onClose} className="px-4 py-2 text-sm text-text-muted hover:underline">
            취소
          </button>
          <button
            onClick={submit}
            disabled={busy}
            className="rounded bg-accent px-4 py-2 text-sm font-medium text-white hover:bg-accent-hover disabled:opacity-60"
          >
            {busy ? "처리 중…" : tab === "create" ? "만들기" : "참가"}
          </button>
        </div>
      </div>
    </div>
  );
}
