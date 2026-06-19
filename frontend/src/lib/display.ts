// 표시 헬퍼 — 이니셜, presence 색, 짧은 id.
import type { PresenceStatus } from "../api/types.ts";
import type { Snowflake } from "./ids.ts";

export function initials(name: string | null | undefined, fallback = "?"): string {
  if (!name) return fallback;
  const parts = name.trim().split(/\s+/);
  if (parts.length === 1) return parts[0].slice(0, 2).toUpperCase();
  return (parts[0][0] + parts[1][0]).toUpperCase();
}

export function presenceColor(status: PresenceStatus | undefined): string {
  switch (status) {
    case "online":
      return "var(--color-online)";
    case "idle":
      return "var(--color-idle)";
    case "dnd":
      return "var(--color-dnd)";
    default:
      return "var(--color-text-muted)";
  }
}

/** id 뒷자리 — 유저명을 모를 때 표시용. */
export function shortId(id: Snowflake): string {
  return id.length > 6 ? `…${id.slice(-6)}` : id;
}
