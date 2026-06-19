// 우측 멤버 목록 — presence 점 + 멤버 액션(DM 보내기·닉 변경·추방).
import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useMembers } from "../hooks/queries.ts";
import { useNameResolver } from "../hooks/names.ts";
import { useSession } from "../store/session.ts";
import { useAuth } from "../store/auth.ts";
import { useUi } from "../store/ui.ts";
import { removeMember, setNick } from "../api/guilds.ts";
import { openDm } from "../api/social.ts";
import { qk } from "../api/queryKeys.ts";
import { presenceColor } from "../lib/display.ts";
import { ApiError } from "../api/http.ts";
import type { Snowflake } from "../lib/ids.ts";

export function MemberList({ realmId }: { realmId: Snowflake }) {
  const qc = useQueryClient();
  const { data: members } = useMembers(realmId);
  const presences = useSession((s) => s.presences);
  const myId = useAuth((s) => s.userId);
  const selectRealm = useUi((s) => s.selectRealm);
  const nameOf = useNameResolver(realmId);
  const [menuFor, setMenuFor] = useState<Snowflake | null>(null);

  const err = (e: unknown) => alert(e instanceof ApiError ? e.message : String(e));

  async function dm(userId: Snowflake) {
    try {
      const c = await openDm(userId);
      qc.invalidateQueries({ queryKey: qk.realms() });
      selectRealm(c.realm_id);
      useUi.getState().selectChannel(c.id);
    } catch (e) {
      err(e);
    }
    setMenuFor(null);
  }

  async function changeNick(userId: Snowflake) {
    const nick = prompt("새 닉네임 (빈 값 = 제거)");
    if (nick === null) return;
    try {
      await setNick(realmId, userId === myId ? "@me" : userId, nick.trim() || null);
      qc.invalidateQueries({ queryKey: qk.members(realmId) });
    } catch (e) {
      err(e);
    }
    setMenuFor(null);
  }

  async function kick(userId: Snowflake) {
    if (!confirm("이 멤버를 추방할까요?")) return;
    try {
      await removeMember(realmId, userId);
      qc.invalidateQueries({ queryKey: qk.members(realmId) });
    } catch (e) {
      err(e);
    }
    setMenuFor(null);
  }

  return (
    <aside className="w-56 shrink-0 overflow-y-auto bg-bg-sidebar px-3 py-4">
      <h3 className="mb-2 text-xs font-bold uppercase text-text-muted">멤버 — {members?.length ?? 0}</h3>
      <div className="space-y-0.5">
        {(members ?? []).map((m) => {
          const status = m.user_id === myId ? (presences[m.user_id] ?? "online") : presences[m.user_id];
          const online = status && status !== "offline";
          const mine = m.user_id === myId;
          return (
            <div key={m.user_id} className="relative">
              <button
                onClick={() => setMenuFor(menuFor === m.user_id ? null : m.user_id)}
                className={`flex w-full items-center gap-2 rounded px-2 py-1 hover:bg-bg-hover ${online ? "" : "opacity-50"}`}
              >
                <div className="relative">
                  <div className="flex h-8 w-8 items-center justify-center rounded-full bg-bg-input text-xs font-bold text-text-bright">
                    {nameOf(m.user_id).slice(0, 2).toUpperCase()}
                  </div>
                  <span
                    className="absolute -right-0.5 -bottom-0.5 h-3 w-3 rounded-full border-2 border-bg-sidebar"
                    style={{ background: presenceColor(status) }}
                  />
                </div>
                <span className="truncate text-sm text-text-normal">
                  {nameOf(m.user_id)}
                  {mine && <span className="text-text-muted"> (나)</span>}
                </span>
              </button>

              {menuFor === m.user_id && (
                <div className="absolute right-0 z-10 mt-1 w-36 rounded bg-bg-server py-1 shadow-lg">
                  {!mine && (
                    <MenuItem onClick={() => dm(m.user_id)}>💬 DM 보내기</MenuItem>
                  )}
                  <MenuItem onClick={() => changeNick(m.user_id)}>✎ 닉네임 변경</MenuItem>
                  {!mine && (
                    <MenuItem danger onClick={() => kick(m.user_id)}>⛔ 추방</MenuItem>
                  )}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </aside>
  );
}

function MenuItem({
  children,
  onClick,
  danger,
}: {
  children: React.ReactNode;
  onClick: () => void;
  danger?: boolean;
}) {
  return (
    <button
      onClick={onClick}
      className={`block w-full px-3 py-1.5 text-left text-sm hover:bg-bg-hover ${danger ? "text-dnd" : "text-text-normal"}`}
    >
      {children}
    </button>
  );
}
