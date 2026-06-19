// React Query 키 단일 출처 — 캐시 패치/무효화가 동일 키를 쓰도록.
import type { Snowflake } from "../lib/ids.ts";

export const qk = {
  realms: () => ["realms"] as const,
  channels: (realmId: Snowflake) => ["channels", realmId] as const,
  messages: (channelId: Snowflake) => ["messages", channelId] as const,
  members: (realmId: Snowflake) => ["members", realmId] as const,
  relationships: () => ["relationships"] as const,
  roles: (realmId: Snowflake) => ["roles", realmId] as const,
};
