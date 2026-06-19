// 실시간 세션 store (zustand) — Gateway READY/DISPATCH가 채우는 휘발 상태.
// REST-백업 데이터(realms/channels/messages)는 React Query가 소유하고, 여기엔 WS로만
// 도착하는 것(연결상태·presence·읽음상태·관계·채널별 마지막 메시지)을 둔다.
import { create } from "zustand";
import type { GatewayState } from "../gateway/connection.ts";
import type { PresenceStatus, ReadStateView, ReadyPayload, RelationKind } from "../api/types.ts";
import type { Snowflake } from "../lib/ids.ts";

interface SessionState {
  gatewayState: GatewayState | "idle";
  sessionId: string | null;
  presences: Record<Snowflake, PresenceStatus>;
  readStates: Record<Snowflake, ReadStateView>;
  lastMessageIds: Record<Snowflake, Snowflake>;
  relationships: Record<Snowflake, RelationKind>;

  setGatewayState: (s: GatewayState | "idle") => void;
  applyReady: (d: ReadyPayload) => void;
  setPresence: (userId: Snowflake, status: PresenceStatus) => void;
  setReadState: (rs: ReadStateView) => void;
  setRelationship: (userId: Snowflake, kind: RelationKind) => void;
  removeRelationship: (userId: Snowflake) => void;
  bumpLastMessage: (channelId: Snowflake, messageId: Snowflake) => void;
  reset: () => void;
}

const empty = {
  presences: {} as Record<Snowflake, PresenceStatus>,
  readStates: {} as Record<Snowflake, ReadStateView>,
  lastMessageIds: {} as Record<Snowflake, Snowflake>,
  relationships: {} as Record<Snowflake, RelationKind>,
};

export const useSession = create<SessionState>((set) => ({
  gatewayState: "idle",
  sessionId: null,
  ...empty,

  setGatewayState: (s) => set({ gatewayState: s }),

  applyReady: (d) =>
    set(() => {
      const presences: Record<Snowflake, PresenceStatus> = {};
      for (const p of d.presences ?? []) presences[p.user.id] = p.status;
      const readStates: Record<Snowflake, ReadStateView> = {};
      for (const rs of d.read_states ?? []) readStates[rs.channel_id] = rs;
      const lastMessageIds: Record<Snowflake, Snowflake> = {};
      for (const lm of d.last_message_ids ?? []) lastMessageIds[lm.channel_id] = lm.last_message_id;
      const relationships: Record<Snowflake, RelationKind> = {};
      for (const r of d.relationships ?? []) relationships[r.user_id] = r.kind;
      return {
        sessionId: d.session_id,
        presences,
        readStates,
        lastMessageIds,
        relationships,
      };
    }),

  setPresence: (userId, status) =>
    set((s) => ({ presences: { ...s.presences, [userId]: status } })),

  setReadState: (rs) => set((s) => ({ readStates: { ...s.readStates, [rs.channel_id]: rs } })),

  setRelationship: (userId, kind) =>
    set((s) => ({ relationships: { ...s.relationships, [userId]: kind } })),

  removeRelationship: (userId) =>
    set((s) => {
      const next = { ...s.relationships };
      delete next[userId];
      return { relationships: next };
    }),

  bumpLastMessage: (channelId, messageId) =>
    set((s) => ({ lastMessageIds: { ...s.lastMessageIds, [channelId]: messageId } })),

  reset: () => set({ gatewayState: "idle", sessionId: null, ...empty }),
}));
