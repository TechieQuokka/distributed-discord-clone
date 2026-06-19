// 채널 스레드 목록/생성 패널 (D44). 스레드 클릭 → 그 스레드를 채널로 열기(ChatArea 재사용).
import { useEffect, useState } from "react";
import { archiveThread, createThread, listThreads, type ThreadView } from "../api/threads.ts";
import { useUi } from "../store/ui.ts";
import { ApiError } from "../api/http.ts";
import type { Snowflake } from "../lib/ids.ts";

export function ThreadsPanel({ channelId, onClose }: { channelId: Snowflake; onClose: () => void }) {
  const selectChannel = useUi((s) => s.selectChannel);
  const [threads, setThreads] = useState<ThreadView[]>([]);
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);

  const err = (e: unknown) => setError(e instanceof ApiError ? e.message : String(e));
  const reload = () => listThreads(channelId).then(setThreads).catch(err);

  useEffect(() => {
    reload();
  }, [channelId]);

  async function create() {
    if (!name.trim()) return;
    try {
      await createThread(channelId, name.trim());
      setName("");
      reload();
    } catch (e) {
      err(e);
    }
  }

  async function toggleArchive(t: ThreadView) {
    try {
      await archiveThread(t.id, !t.archived);
      reload();
    } catch (e) {
      err(e);
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center bg-black/60 pt-24" onClick={onClose}>
      <div className="w-[480px] rounded-md bg-bg-app p-5" onClick={(e) => e.stopPropagation()}>
        <h2 className="mb-3 text-lg font-bold text-text-bright">스레드</h2>

        <div className="mb-3 flex gap-2">
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && create()}
            placeholder="새 스레드 이름"
            className="flex-1 rounded bg-bg-server px-3 py-2 text-text-bright outline-none focus:ring-2 focus:ring-accent"
          />
          <button onClick={create} className="rounded bg-accent px-4 text-sm text-white hover:bg-accent-hover">
            만들기
          </button>
        </div>

        {error && <p className="mb-2 text-sm text-dnd">⚠ {error}</p>}

        <div className="max-h-80 space-y-1 overflow-y-auto">
          {threads.length === 0 && <p className="text-sm text-text-muted">스레드가 없습니다.</p>}
          {threads.map((t) => (
            <div key={t.id} className="flex items-center gap-2 rounded px-2 py-1.5 hover:bg-bg-hover">
              <button
                onClick={() => {
                  selectChannel(t.id);
                  onClose();
                }}
                className="flex min-w-0 flex-1 items-center gap-2 text-left"
              >
                <span>🧵</span>
                <span className="truncate text-text-normal">{t.name ?? t.id}</span>
                <span className="text-xs text-text-muted">{t.message_count}개</span>
                {t.archived && <span className="text-xs text-text-muted">(보관됨)</span>}
              </button>
              <button onClick={() => toggleArchive(t)} className="text-xs text-text-muted hover:text-text-bright">
                {t.archived ? "복원" : "보관"}
              </button>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
