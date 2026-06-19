// 유저명 디렉터리 (zustand) — backend에 GET /users/{id}가 없어, WS 이벤트가 실어주는
// username(RELATIONSHIP_ADD / GUILD_MEMBER_ADD·UPDATE / CHANNEL_RECIPIENT_ADD)과 READY의
// 내 정보를 모아 id→username 사전을 점진 구축한다. 표시명 해상도를 높이는 best-effort.
import { create } from "zustand";
import type { Snowflake } from "../lib/ids.ts";

interface DirectoryState {
  names: Record<Snowflake, string>;
  learn: (id: Snowflake, username: string | null | undefined) => void;
  learnMany: (entries: { id: Snowflake; username?: string | null }[]) => void;
}

export const useDirectory = create<DirectoryState>((set) => ({
  names: {},
  learn: (id, username) =>
    set((s) => (username ? { names: { ...s.names, [id]: username } } : s)),
  learnMany: (entries) =>
    set((s) => {
      const next = { ...s.names };
      for (const e of entries) if (e.username) next[e.id] = e.username;
      return { names: next };
    }),
}));

/** 비훅 접근(이벤트 핸들러용). */
export function learnUser(id: Snowflake, username: string | null | undefined): void {
  useDirectory.getState().learn(id, username);
}
