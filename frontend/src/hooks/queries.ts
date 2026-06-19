// REST 읽기 훅 (React Query, D30). 캐시 키는 queryKeys 단일 출처.
import { useQuery } from "@tanstack/react-query";
import { qk } from "../api/queryKeys.ts";
import { listChannels, listMembers, listMyRealms } from "../api/guilds.ts";
import { listMessages } from "../api/messages.ts";
import { listRelationships } from "../api/social.ts";
import type { Snowflake } from "../lib/ids.ts";

export function useRealms() {
  return useQuery({ queryKey: qk.realms(), queryFn: listMyRealms });
}

export function useChannels(realmId: Snowflake | null) {
  return useQuery({
    queryKey: realmId ? qk.channels(realmId) : qk.channels("none"),
    queryFn: () => listChannels(realmId!),
    enabled: !!realmId,
  });
}

export function useMessages(channelId: Snowflake | null) {
  return useQuery({
    queryKey: channelId ? qk.messages(channelId) : qk.messages("none"),
    queryFn: () => listMessages(channelId!, { limit: 50 }),
    enabled: !!channelId,
  });
}

export function useMembers(realmId: Snowflake | null) {
  return useQuery({
    queryKey: realmId ? qk.members(realmId) : qk.members("none"),
    queryFn: () => listMembers(realmId!),
    enabled: !!realmId,
  });
}

export function useRelationships() {
  return useQuery({ queryKey: qk.relationships(), queryFn: listRelationships });
}
