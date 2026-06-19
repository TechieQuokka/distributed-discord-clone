// DM·친구·읽음상태 API (CLI `rest.rs` 미러).
import { api } from "./http.ts";
import type { Snowflake } from "../lib/ids.ts";
import type { DmChannelView, ReadStateView, RelationshipView } from "./types.ts";

// ── DM / 그룹DM ───────────────────────────────────────────────────────────
/** 1:1 DM 열기(find-or-create). 기존 있으면 같은 채널 반환. */
export function openDm(recipientId: Snowflake): Promise<DmChannelView> {
  return api.post<DmChannelView>("/users/@me/channels", { json: { recipient_id: recipientId } });
}

export function createGroupDm(recipientIds: Snowflake[], name?: string): Promise<DmChannelView> {
  return api.post<DmChannelView>("/users/@me/channels", { json: { recipient_ids: recipientIds, name } });
}

export function addRecipient(channelId: Snowflake, userId: Snowflake): Promise<void> {
  return api.put<void>(`/channels/${channelId}/recipients/${userId}`);
}

export function removeRecipient(channelId: Snowflake, userId: Snowflake): Promise<void> {
  return api.del<void>(`/channels/${channelId}/recipients/${userId}`);
}

// ── 친구 / 차단 ───────────────────────────────────────────────────────────
export function listRelationships(): Promise<RelationshipView[]> {
  return api.get<RelationshipView[]>("/users/@me/relationships");
}

/** 친구 요청/수락(type="friend") 또는 차단(type="block"). */
export function putRelationship(userId: Snowflake, type: "friend" | "block"): Promise<RelationshipView> {
  return api.put<RelationshipView>(`/users/@me/relationships/${userId}`, { json: { type } });
}

/** 친구 삭제 / 요청 취소·거절 / 차단 해제. */
export function removeRelationship(userId: Snowflake): Promise<void> {
  return api.del<void>(`/users/@me/relationships/${userId}`);
}

// ── 읽음 상태 ──────────────────────────────────────────────────────────────
export function listReadStates(): Promise<ReadStateView[]> {
  return api.get<ReadStateView[]>("/users/@me/read-states");
}
