// 음성 상태 (zustand) — VOICE_STATE_UPDATE(op4 제어 평면, D47)로 채널별 참가자 집계.
// 미디어(WebRTC)는 D21 범위 밖 — 입장/이동/퇴장·self mute/deaf 시그널만.
import { create } from "zustand";
import type { Snowflake } from "../lib/ids.ts";

export interface VoiceMember {
  user_id: Snowflake;
  self_mute: boolean;
  self_deaf: boolean;
}

interface VoiceState {
  // channelId → userId → state
  byChannel: Record<Snowflake, Record<Snowflake, VoiceMember>>;
  // 유저가 현재 있는 음성 채널(이동/퇴장 처리용).
  userChannel: Record<Snowflake, Snowflake>;
  apply: (p: {
    channel_id: Snowflake | null;
    user_id: Snowflake;
    self_mute: boolean;
    self_deaf: boolean;
  }) => void;
}

export const useVoice = create<VoiceState>((set) => ({
  byChannel: {},
  userChannel: {},
  apply: (p) =>
    set((s) => {
      const byChannel = { ...s.byChannel };
      const userChannel = { ...s.userChannel };

      // 기존 채널에서 제거(이동/퇴장).
      const prev = userChannel[p.user_id];
      if (prev && byChannel[prev]) {
        const room = { ...byChannel[prev] };
        delete room[p.user_id];
        byChannel[prev] = room;
      }

      if (p.channel_id === null) {
        delete userChannel[p.user_id];
      } else {
        const room = { ...(byChannel[p.channel_id] ?? {}) };
        room[p.user_id] = {
          user_id: p.user_id,
          self_mute: p.self_mute,
          self_deaf: p.self_deaf,
        };
        byChannel[p.channel_id] = room;
        userChannel[p.user_id] = p.channel_id;
      }
      return { byChannel, userChannel };
    }),
}));
