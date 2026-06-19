// 리액션 집계 (zustand) — backend에 리액션 목록/카운트 조회 API가 없어, WS의
// MESSAGE_REACTION_ADD/_REMOVE 이벤트로 **세션 중** 카운트를 라이브 집계한다.
// seam: 히스토리(접속 전 리액션)는 반영 안 됨 — 지금 일어나는 리액션은 정확.
import { create } from "zustand";
import type { Snowflake } from "../lib/ids.ts";

export interface EmojiTally {
  count: number;
  mine: boolean;
}
// messageId → emoji → tally
type Tallies = Record<Snowflake, Record<string, EmojiTally>>;

interface ReactionsState {
  byMessage: Tallies;
  add: (messageId: Snowflake, emoji: string, isMe: boolean) => void;
  remove: (messageId: Snowflake, emoji: string, isMe: boolean) => void;
}

export const useReactions = create<ReactionsState>((set) => ({
  byMessage: {},
  add: (messageId, emoji, isMe) =>
    set((s) => {
      const msg = { ...(s.byMessage[messageId] ?? {}) };
      const cur = msg[emoji] ?? { count: 0, mine: false };
      msg[emoji] = { count: cur.count + 1, mine: cur.mine || isMe };
      return { byMessage: { ...s.byMessage, [messageId]: msg } };
    }),
  remove: (messageId, emoji, isMe) =>
    set((s) => {
      const msg = { ...(s.byMessage[messageId] ?? {}) };
      const cur = msg[emoji];
      if (!cur) return s;
      const count = cur.count - 1;
      if (count <= 0) {
        delete msg[emoji];
      } else {
        msg[emoji] = { count, mine: isMe ? false : cur.mine };
      }
      return { byMessage: { ...s.byMessage, [messageId]: msg } };
    }),
}));
