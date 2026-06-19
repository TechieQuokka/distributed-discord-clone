// DM 홈 사이드바 — 친구 관리(요청/수락/차단/삭제) + DM 대화 목록. UserPanel 하단.
// username 조회 API가 없어 친구 추가는 **user id**로 한다(상대 id는 멤버목록·메시지에서 확인).
import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useMembers, useRealms, useRelationships } from "../hooks/queries.ts";
import { useNameResolver } from "../hooks/names.ts";
import { useAuth } from "../store/auth.ts";
import { createGroupDm, openDm, putRelationship, removeRelationship } from "../api/social.ts";
import { qk } from "../api/queryKeys.ts";
import { useUi } from "../store/ui.ts";
import { useSession } from "../store/session.ts";
import { UserPanel } from "./UserPanel.tsx";
import { presenceColor } from "../lib/display.ts";
import { ApiError } from "../api/http.ts";
import type { Snowflake } from "../lib/ids.ts";
import type { RealmView, RelationKind } from "../api/types.ts";

const KIND_LABEL: Record<RelationKind, string> = {
  friend: "친구",
  pending_in: "받은 요청",
  pending_out: "보낸 요청",
  blocked: "차단",
};

export function DmSidebar() {
  const qc = useQueryClient();
  const { data: relationships } = useRelationships();
  const { data: realms } = useRealms();
  const presences = useSession((s) => s.presences);
  const selectedRealmId = useUi((s) => s.selectedRealmId);
  const selectRealm = useUi((s) => s.selectRealm);
  const nameOf = useNameResolver(null);
  const [addId, setAddId] = useState("");
  const [error, setError] = useState<string | null>(null);

  const dms = (realms ?? []).filter((r) => r.kind === "dm" || r.kind === "group_dm");

  const refresh = () => qc.invalidateQueries({ queryKey: qk.relationships() });
  const errText = (e: unknown) =>
    e instanceof ApiError ? e.message : e instanceof Error ? e.message : String(e);

  async function addFriend() {
    if (!addId.trim()) return;
    setError(null);
    try {
      await putRelationship(addId.trim(), "friend");
      setAddId("");
      refresh();
    } catch (e) {
      setError(errText(e));
    }
  }

  async function openConversation(friendId: Snowflake) {
    const dm = await openDm(friendId);
    qc.invalidateQueries({ queryKey: qk.realms() });
    selectRealm(dm.realm_id);
    useUi.getState().selectChannel(dm.id);
  }

  async function makeGroup() {
    const raw = prompt("그룹 DM 참가자 user id (콤마 구분)");
    if (!raw?.trim()) return;
    const ids = raw.split(",").map((s) => s.trim()).filter(Boolean);
    if (ids.length === 0) return;
    const name = prompt("그룹 이름 (선택)") || undefined;
    try {
      const dm = await createGroupDm(ids, name);
      qc.invalidateQueries({ queryKey: qk.realms() });
      selectRealm(dm.realm_id);
      useUi.getState().selectChannel(dm.id);
    } catch (e) {
      setError(errText(e));
    }
  }

  return (
    <div className="flex w-60 flex-col bg-bg-sidebar">
      <header className="flex h-12 items-center justify-between border-b border-black/20 px-4 font-semibold text-text-bright shadow-sm">
        <span>다이렉트 메시지</span>
        <button onClick={makeGroup} title="그룹 DM 만들기" className="text-text-muted hover:text-text-bright">
          ＋
        </button>
      </header>

      <div className="border-b border-black/20 p-3">
        <div className="mb-1 text-xs font-bold uppercase text-text-muted">친구 추가 (user id)</div>
        <div className="flex gap-1">
          <input
            value={addId}
            onChange={(e) => setAddId(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && addFriend()}
            placeholder="343…"
            className="min-w-0 flex-1 rounded bg-bg-server px-2 py-1 text-sm text-text-bright outline-none"
          />
          <button onClick={addFriend} className="rounded bg-accent px-2 text-sm text-white hover:bg-accent-hover">
            추가
          </button>
        </div>
        {error && <p className="mt-1 text-xs text-dnd">⚠ {error}</p>}
      </div>

      <div className="flex-1 overflow-y-auto px-2 py-2">
        <div className="px-2 text-xs font-bold uppercase text-text-muted">친구</div>
        {(relationships ?? []).length === 0 && (
          <p className="px-2 py-1 text-xs text-text-muted">아직 친구가 없습니다.</p>
        )}
        {(relationships ?? []).map((r) => (
          <div key={r.user_id} className="flex items-center gap-2 rounded px-2 py-1.5 hover:bg-bg-hover">
            <div className="relative">
              <div className="flex h-8 w-8 items-center justify-center rounded-full bg-bg-input text-xs font-bold text-text-bright">
                {r.user_id.slice(-2)}
              </div>
              <span
                className="absolute -right-0.5 -bottom-0.5 h-3 w-3 rounded-full border-2 border-bg-sidebar"
                style={{ background: presenceColor(presences[r.user_id]) }}
              />
            </div>
            <div className="min-w-0 flex-1">
              <div className="truncate text-sm text-text-normal">{nameOf(r.user_id)}</div>
              <div className="text-xs text-text-muted">{KIND_LABEL[r.kind]}</div>
            </div>
            <div className="flex gap-1">
              {r.kind === "friend" && (
                <button onClick={() => openConversation(r.user_id)} title="DM 열기" className="text-text-muted hover:text-text-bright">
                  💬
                </button>
              )}
              {r.kind === "pending_in" && (
                <button onClick={() => putRelationship(r.user_id, "friend").then(refresh)} title="수락" className="text-online">
                  ✓
                </button>
              )}
              <button
                onClick={() => removeRelationship(r.user_id).then(refresh)}
                title={r.kind === "blocked" ? "차단 해제" : "삭제/거절"}
                className="text-text-muted hover:text-dnd"
              >
                ✕
              </button>
            </div>
          </div>
        ))}

        {dms.length > 0 && (
          <>
            <div className="mt-4 px-2 text-xs font-bold uppercase text-text-muted">다이렉트 메시지</div>
            {dms.map((d) => (
              <DmEntry
                key={d.id}
                d={d}
                selected={d.id === selectedRealmId}
                onClick={() => selectRealm(d.id)}
              />
            ))}
          </>
        )}
      </div>

      <UserPanel />
    </div>
  );
}

// DM 목록 한 항목. 1:1은 상대 이름(멤버 조회+이름 해석)을, 그룹은 그룹명을 보여 서로 구분되게 한다.
// (RealmView엔 recipients가 없어 DM realm의 members 엔드포인트로 상대를 알아낸다 — backend 무수정.)
function DmEntry({ d, selected, onClick }: { d: RealmView; selected: boolean; onClick: () => void }) {
  const myId = useAuth((s) => s.userId);
  const is1to1 = d.kind === "dm";
  const { data: members } = useMembers(is1to1 ? d.id : null);
  const nameOf = useNameResolver(is1to1 ? d.id : null);
  const otherId = is1to1 ? (members ?? []).find((m) => m.user_id !== myId)?.user_id : undefined;
  const label =
    d.kind === "group_dm"
      ? (d.name ?? "그룹 DM")
      : otherId
        ? nameOf(otherId)
        : (d.name ?? "다이렉트 메시지");

  return (
    <button
      onClick={onClick}
      className={`flex w-full items-center gap-2 rounded px-2 py-1.5 text-left text-sm ${
        selected ? "bg-bg-selected text-text-bright" : "text-text-muted hover:bg-bg-hover"
      }`}
    >
      <span className="flex h-8 w-8 items-center justify-center rounded-full bg-bg-input text-xs">
        {d.kind === "group_dm" ? "👥" : "@"}
      </span>
      <span className="truncate">{label}</span>
    </button>
  );
}
