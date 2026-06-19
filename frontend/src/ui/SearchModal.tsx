// 길드 메시지 전문검색 모달 (Q10 FTS). 결과 클릭 → 해당 채널로 이동.
import { useState } from "react";
import { searchMessages } from "../api/messages.ts";
import { useNameResolver } from "../hooks/names.ts";
import { useUi } from "../store/ui.ts";
import { ApiError } from "../api/http.ts";
import type { Snowflake } from "../lib/ids.ts";
import type { MessageView } from "../api/types.ts";

export function SearchModal({ realmId, onClose }: { realmId: Snowflake; onClose: () => void }) {
  const selectChannel = useUi((s) => s.selectChannel);
  const nameOf = useNameResolver(realmId);
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<MessageView[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function run() {
    if (!query.trim()) return;
    setBusy(true);
    setError(null);
    try {
      setResults(await searchMessages(realmId, query.trim(), 50));
    } catch (e) {
      setError(e instanceof ApiError ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center bg-black/60 pt-24" onClick={onClose}>
      <div className="w-[560px] rounded-md bg-bg-app p-5" onClick={(e) => e.stopPropagation()}>
        <div className="mb-3 flex gap-2">
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && run()}
            placeholder="메시지 검색 (websearch: 따옴표/OR/- 지원)"
            className="flex-1 rounded bg-bg-server px-3 py-2 text-text-bright outline-none focus:ring-2 focus:ring-accent"
            autoFocus
          />
          <button onClick={run} disabled={busy} className="rounded bg-accent px-4 text-sm text-white hover:bg-accent-hover disabled:opacity-60">
            {busy ? "검색 중…" : "검색"}
          </button>
        </div>

        {error && <p className="mb-2 text-sm text-dnd">⚠ {error}</p>}

        <div className="max-h-96 overflow-y-auto">
          {results && results.length === 0 && <p className="text-sm text-text-muted">결과가 없습니다.</p>}
          {(results ?? []).map((m) => (
            <button
              key={m.id}
              onClick={() => {
                selectChannel(m.channel_id);
                onClose();
              }}
              className="block w-full rounded px-3 py-2 text-left hover:bg-bg-hover"
            >
              <div className="text-xs text-text-muted">
                {nameOf(m.author_id)} · #채널 {m.channel_id.slice(-6)}
              </div>
              <div className="truncate text-text-normal">{m.content}</div>
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
