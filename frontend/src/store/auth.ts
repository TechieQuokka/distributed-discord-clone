// 인증 store (zustand) — 토큰/유저 세션 보관 + localStorage 영속 + http 모듈 토큰 동기화.
import { create } from "zustand";
import { setAccessToken } from "../api/http.ts";
import type { AuthResponse } from "../api/types.ts";
import type { Snowflake } from "../lib/ids.ts";

const STORAGE_KEY = "discord-clone-auth";

interface PersistedAuth {
  userId: Snowflake;
  username: string | null;
  accessToken: string;
  refreshToken: string;
}

interface AuthState {
  userId: Snowflake | null;
  username: string | null;
  accessToken: string | null;
  refreshToken: string | null;
  isAuthed: boolean;
  setSession: (res: AuthResponse, username?: string | null) => void;
  setUsername: (name: string | null) => void;
  logout: () => void;
}

function load(): PersistedAuth | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw ? (JSON.parse(raw) as PersistedAuth) : null;
  } catch {
    return null;
  }
}

function save(a: PersistedAuth | null): void {
  if (a) localStorage.setItem(STORAGE_KEY, JSON.stringify(a));
  else localStorage.removeItem(STORAGE_KEY);
}

// 초기 하이드레이션: 새로고침 후에도 로그인 유지. http 모듈에 토큰 주입.
const initial = load();
if (initial) setAccessToken(initial.accessToken);

export const useAuth = create<AuthState>((set, get) => ({
  userId: initial?.userId ?? null,
  username: initial?.username ?? null,
  accessToken: initial?.accessToken ?? null,
  refreshToken: initial?.refreshToken ?? null,
  isAuthed: initial !== null,

  setSession: (res, username) => {
    setAccessToken(res.access_token);
    const persisted: PersistedAuth = {
      userId: res.user_id,
      username: username ?? get().username,
      accessToken: res.access_token,
      refreshToken: res.refresh_token,
    };
    save(persisted);
    set({
      userId: persisted.userId,
      username: persisted.username,
      accessToken: persisted.accessToken,
      refreshToken: persisted.refreshToken,
      isAuthed: true,
    });
  },

  setUsername: (name) => {
    const cur = load();
    if (cur) save({ ...cur, username: name });
    set({ username: name });
  },

  logout: () => {
    setAccessToken(null);
    save(null);
    set({ userId: null, username: null, accessToken: null, refreshToken: null, isAuthed: false });
  },
}));
