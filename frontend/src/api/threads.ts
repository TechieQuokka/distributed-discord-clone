// 스레드 API (CLI `rest.rs` 미러, D44). 스레드 = 부모와 같은 Realm의 채널(kind=thread).
import { api } from "./http.ts";
import type { Snowflake } from "../lib/ids.ts";

export interface ThreadView {
  id: Snowflake;
  parent_id: Snowflake;
  name: string | null;
  owner_id: Snowflake | null;
  archived: boolean;
  message_count: number;
}

export function listThreads(channelId: Snowflake): Promise<ThreadView[]> {
  return api.get<ThreadView[]>(`/channels/${channelId}/threads`);
}

export function createThread(channelId: Snowflake, name: string, autoArchive?: number): Promise<ThreadView> {
  return api.post<ThreadView>(`/channels/${channelId}/threads`, { json: { name, auto_archive: autoArchive } });
}

export function archiveThread(threadId: Snowflake, archived: boolean): Promise<ThreadView> {
  return api.patch<ThreadView>(`/channels/${threadId}/thread`, { json: { archived } });
}
