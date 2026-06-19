// 표시명 해상도 — 길드 nick > 학습된 username(directory) > 짧은 id. "나"는 본인.
import { useMemo } from "react";
import { useMembers } from "./queries.ts";
import { useDirectory } from "../store/directory.ts";
import { useAuth } from "../store/auth.ts";
import { shortId } from "../lib/display.ts";
import type { Snowflake } from "../lib/ids.ts";

export function useNameResolver(realmId: Snowflake | null) {
  const { data: members } = useMembers(realmId);
  const names = useDirectory((s) => s.names);
  const myId = useAuth((s) => s.userId);
  const myName = useAuth((s) => s.username);

  return useMemo(() => {
    const nick = new Map<Snowflake, string>();
    for (const m of members ?? []) if (m.nick) nick.set(m.user_id, m.nick);
    return (id: Snowflake): string => {
      if (id === myId) return myName ?? "나";
      return nick.get(id) ?? names[id] ?? shortId(id);
    };
  }, [members, names, myId, myName]);
}
