// 메시지 API (CLI `rest.rs` 미러). 전송은 응답 본문이 없고 gateway MESSAGE_CREATE로 도착(D24).
import { api } from "./http.ts";
import type { Snowflake } from "../lib/ids.ts";
import type { MessageView, ReadStateView } from "./types.ts";

/** 채널 히스토리(최신순). before/after/around 커서 + limit (D38). */
export function listMessages(
  channelId: Snowflake,
  opts: { before?: Snowflake; after?: Snowflake; around?: Snowflake; limit?: number } = {},
): Promise<MessageView[]> {
  return api.get<MessageView[]>(`/channels/${channelId}/messages`, { query: opts });
}

/**
 * 메시지 전송. persist-then-fanout(D24)이라 **응답 본문 없음** — 본인 세션의 WS
 * MESSAGE_CREATE로 되돌아온다. nonce는 멱등 dedup(D34)용 + 옵티미스틱 매칭용.
 */
export function sendMessage(
  channelId: Snowflake,
  content: string,
  opts: { nonce?: string; referenceMessageId?: Snowflake } = {},
): Promise<void> {
  return api.post<void>(`/channels/${channelId}/messages`, {
    json: {
      content,
      nonce: opts.nonce ?? null,
      reference_message_id: opts.referenceMessageId,
    },
  });
}

export function editMessage(channelId: Snowflake, messageId: Snowflake, content: string): Promise<MessageView> {
  return api.patch<MessageView>(`/channels/${channelId}/messages/${messageId}`, { json: { content } });
}

export function deleteMessage(channelId: Snowflake, messageId: Snowflake): Promise<void> {
  return api.del<void>(`/channels/${channelId}/messages/${messageId}`);
}

export function addReaction(channelId: Snowflake, messageId: Snowflake, emoji: string): Promise<void> {
  const e = encodeURIComponent(emoji);
  return api.put<void>(`/channels/${channelId}/messages/${messageId}/reactions/${e}/@me`);
}

export function removeReaction(channelId: Snowflake, messageId: Snowflake, emoji: string): Promise<void> {
  const e = encodeURIComponent(emoji);
  return api.del<void>(`/channels/${channelId}/messages/${messageId}/reactions/${e}/@me`);
}

/** 채널을 메시지까지 읽음 처리(ack) → MESSAGE_ACK (D41). */
export function ackMessage(channelId: Snowflake, messageId: Snowflake): Promise<ReadStateView> {
  return api.post<ReadStateView>(`/channels/${channelId}/messages/${messageId}/ack`);
}

/** 길드 전문검색 (Q10, FTS). VIEW_CHANNEL 채널만. */
export function searchMessages(realmId: Snowflake, content: string, limit?: number): Promise<MessageView[]> {
  return api.get<MessageView[]>(`/guilds/${realmId}/messages/search`, { query: { content, limit } });
}
