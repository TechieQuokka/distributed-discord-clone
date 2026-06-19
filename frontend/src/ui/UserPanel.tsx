// 사이드바 하단 유저 패널 — 내 정보 + presence(op3) 변경 + 연결상태 + 설정 + 로그아웃.
import { useState } from "react";
import { useAuth } from "../store/auth.ts";
import { useSession } from "../store/session.ts";
import { getGatewayClient } from "../gateway/RealtimeProvider.tsx";
import { SettingsModal } from "./SettingsModal.tsx";
import { initials, presenceColor, shortId } from "../lib/display.ts";
import type { PresenceStatus } from "../api/types.ts";

const GATEWAY_LABEL: Record<string, string> = {
  idle: "연결 안 됨",
  connecting: "연결 중…",
  identifying: "인증 중…",
  resuming: "재개 중…",
  reconnecting: "재연결 중…",
  ready: "온라인",
  closed: "종료됨",
};

export function UserPanel() {
  const userId = useAuth((s) => s.userId);
  const username = useAuth((s) => s.username);
  const logout = useAuth((s) => s.logout);
  const gatewayState = useSession((s) => s.gatewayState);
  const myStatus = useSession((s) => (userId ? s.presences[userId] : undefined));
  const [settings, setSettings] = useState(false);

  const setPresence = (status: "online" | "idle" | "dnd") => {
    getGatewayClient()?.setPresence(status);
    // 낙관적 반영(서버 PRESENCE_UPDATE가 곧 확정).
    if (userId) useSession.getState().setPresence(userId, status);
  };

  const effectiveStatus: PresenceStatus =
    gatewayState === "ready" ? (myStatus ?? "online") : "offline";

  return (
    <div className="flex items-center gap-2 bg-bg-server px-2 py-1.5">
      <div className="relative">
        <div className="flex h-8 w-8 items-center justify-center rounded-full bg-accent text-xs font-bold text-white">
          {initials(username, "?")}
        </div>
        <span
          className="absolute -right-0.5 -bottom-0.5 h-3 w-3 rounded-full border-2 border-bg-server"
          style={{ background: presenceColor(effectiveStatus) }}
          title={GATEWAY_LABEL[gatewayState] ?? gatewayState}
        />
      </div>

      <div className="min-w-0 flex-1">
        <div className="truncate text-sm font-semibold text-text-bright">
          {username ?? (userId ? shortId(userId) : "나")}
        </div>
        <div className="truncate text-xs text-text-muted">{GATEWAY_LABEL[gatewayState] ?? gatewayState}</div>
      </div>

      <select
        value={myStatus ?? "online"}
        onChange={(e) => setPresence(e.target.value as "online" | "idle" | "dnd")}
        className="rounded bg-bg-input px-1 py-1 text-xs text-text-muted outline-none"
        title="상태 변경"
      >
        <option value="online">온라인</option>
        <option value="idle">자리비움</option>
        <option value="dnd">방해 금지</option>
      </select>

      <button
        onClick={() => setSettings(true)}
        title="설정"
        className="rounded px-1.5 py-1 text-text-muted hover:bg-bg-hover hover:text-text-bright"
      >
        ⚙
      </button>

      <button
        onClick={logout}
        title="로그아웃"
        className="rounded px-1.5 py-1 text-text-muted hover:bg-bg-hover hover:text-dnd"
      >
        ⏻
      </button>

      {settings && <SettingsModal onClose={() => setSettings(false)} />}
    </div>
  );
}
