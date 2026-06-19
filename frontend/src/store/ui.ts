// UI 선택 상태 (zustand) — 현재 선택된 Realm/채널. 여러 패널이 공유.
import { create } from "zustand";
import type { Snowflake } from "../lib/ids.ts";

interface UiState {
  selectedRealmId: Snowflake | null;
  selectedChannelId: Snowflake | null;
  selectRealm: (realmId: Snowflake | null) => void;
  selectChannel: (channelId: Snowflake | null) => void;
}

export const useUi = create<UiState>((set) => ({
  selectedRealmId: null,
  selectedChannelId: null,
  // realm 전환 시 채널 선택은 초기화(채널 로드 후 첫 채널 자동선택은 컴포넌트가).
  selectRealm: (realmId) => set({ selectedRealmId: realmId, selectedChannelId: null }),
  selectChannel: (channelId) => set({ selectedChannelId: channelId }),
}));
