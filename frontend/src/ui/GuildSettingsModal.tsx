// 길드 관리 모달 — 감사 로그(realm) + 웹훅(현재 채널). 역할/권한 편집은 후속(비트마스크 UX).
import { useEffect, useState } from "react";
import {
  auditLabel,
  createWebhook,
  deleteWebhook,
  listAudit,
  listWebhooks,
  type AuditEntryView,
  type WebhookView,
} from "../api/admin.ts";
import { useNameResolver } from "../hooks/names.ts";
import { useUi } from "../store/ui.ts";
import { ApiError } from "../api/http.ts";
import { shortId } from "../lib/display.ts";
import type { Snowflake } from "../lib/ids.ts";

export function GuildSettingsModal({ realmId, onClose }: { realmId: Snowflake; onClose: () => void }) {
  const [tab, setTab] = useState<"audit" | "webhooks">("audit");
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={onClose}>
      <div className="w-[560px] rounded-md bg-bg-app p-5" onClick={(e) => e.stopPropagation()}>
        <div className="mb-4 flex gap-2 border-b border-bg-server">
          {(["audit", "webhooks"] as const).map((t) => (
            <button
              key={t}
              onClick={() => setTab(t)}
              className={`px-3 py-2 text-sm ${tab === t ? "border-b-2 border-accent text-text-bright" : "text-text-muted"}`}
            >
              {t === "audit" ? "감사 로그" : "웹훅"}
            </button>
          ))}
          <button onClick={onClose} className="ml-auto px-2 text-text-muted hover:text-text-bright">✕</button>
        </div>
        {tab === "audit" ? <AuditTab realmId={realmId} /> : <WebhooksTab />}
      </div>
    </div>
  );
}

function AuditTab({ realmId }: { realmId: Snowflake }) {
  const nameOf = useNameResolver(realmId);
  const [entries, setEntries] = useState<AuditEntryView[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    listAudit(realmId).then(setEntries).catch((e) => setError(e instanceof ApiError ? e.message : String(e)));
  }, [realmId]);

  if (error) return <p className="text-sm text-dnd">⚠ {error}</p>;
  return (
    <div className="max-h-96 space-y-1 overflow-y-auto">
      {entries?.length === 0 && <p className="text-sm text-text-muted">기록이 없습니다.</p>}
      {(entries ?? []).map((e) => (
        <div key={e.id} className="rounded bg-bg-sidebar px-3 py-2 text-sm">
          <span className="text-text-bright">{auditLabel(e.action_type)}</span>
          <span className="text-text-muted">
            {" "}· {e.actor_id ? nameOf(e.actor_id) : "시스템"}
            {e.target_id && ` → ${shortId(e.target_id)}`}
          </span>
        </div>
      ))}
    </div>
  );
}

function WebhooksTab() {
  const channelId = useUi((s) => s.selectedChannelId);
  const [hooks, setHooks] = useState<WebhookView[]>([]);
  const [name, setName] = useState("");
  const [created, setCreated] = useState<WebhookView | null>(null);
  const [error, setError] = useState<string | null>(null);

  const err = (e: unknown) => setError(e instanceof ApiError ? e.message : String(e));
  const reload = () => {
    if (channelId) listWebhooks(channelId).then(setHooks).catch(err);
  };
  useEffect(reload, [channelId]);

  if (!channelId) return <p className="text-sm text-text-muted">채널을 먼저 선택하세요.</p>;

  async function create() {
    if (!name.trim()) return;
    try {
      const w = await createWebhook(channelId!, name.trim());
      setCreated(w);
      setName("");
      reload();
    } catch (e) {
      err(e);
    }
  }

  return (
    <div>
      <p className="mb-2 text-xs text-text-muted">현재 채널의 웹훅. 생성 시 토큰은 1회만 표시됩니다.</p>
      <div className="mb-3 flex gap-2">
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="웹훅 이름"
          className="flex-1 rounded bg-bg-server px-3 py-2 text-text-bright outline-none"
        />
        <button onClick={create} className="rounded bg-accent px-4 text-sm text-white hover:bg-accent-hover">생성</button>
      </div>
      {created?.token && (
        <div className="mb-3 break-all rounded bg-bg-server px-3 py-2 text-xs">
          <span className="text-online">✓ 토큰(1회): </span>
          <span className="select-all font-mono text-text-bright">{created.token}</span>
        </div>
      )}
      {error && <p className="mb-2 text-sm text-dnd">⚠ {error}</p>}
      <div className="max-h-72 space-y-1 overflow-y-auto">
        {hooks.length === 0 && <p className="text-sm text-text-muted">웹훅이 없습니다.</p>}
        {hooks.map((w) => (
          <div key={w.id} className="flex items-center justify-between rounded bg-bg-sidebar px-3 py-2 text-sm">
            <span className="text-text-normal">🪝 {w.name}</span>
            <button
              onClick={() => deleteWebhook(w.id).then(reload).catch(err)}
              className="text-xs text-text-muted hover:text-dnd"
            >
              삭제
            </button>
          </div>
        ))}
      </div>
    </div>
  );
}
