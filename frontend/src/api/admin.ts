// 관리자 API — 감사로그(V16) + 웹훅(V15). CLI `rest.rs` 미러.
import { api } from "./http.ts";
import type { Snowflake } from "../lib/ids.ts";

export interface AuditEntryView {
  id: Snowflake;
  actor_id: Snowflake | null;
  action_type: number;
  target_id: Snowflake | null;
}

export interface WebhookView {
  id: Snowflake;
  name: string;
  token: string | null; // 생성 시 1회만.
}

export function listAudit(realmId: Snowflake, limit = 50): Promise<AuditEntryView[]> {
  return api.get<AuditEntryView[]>(`/guilds/${realmId}/audit-logs`, { query: { limit } });
}

export function listWebhooks(channelId: Snowflake): Promise<WebhookView[]> {
  return api.get<WebhookView[]>(`/channels/${channelId}/webhooks`);
}

export function createWebhook(channelId: Snowflake, name: string): Promise<WebhookView> {
  return api.post<WebhookView>(`/channels/${channelId}/webhooks`, { json: { name } });
}

export function deleteWebhook(webhookId: Snowflake): Promise<void> {
  return api.del<void>(`/webhooks/${webhookId}`);
}

// 알려진 audit action_type → 한글 라벨(domain::audit::AuditAction). 미상은 #N.
const ACTION_LABELS: Record<number, string> = {
  1: "채널 생성",
  2: "채널 수정",
  3: "채널 삭제",
  10: "역할 생성",
  11: "역할 수정",
  12: "역할 삭제",
  20: "멤버 추방",
  21: "멤버 닉 변경",
  22: "멤버 역할 변경",
  30: "웹훅 생성",
  31: "웹훅 삭제",
};
export function auditLabel(actionType: number): string {
  return ACTION_LABELS[actionType] ?? `액션 #${actionType}`;
}
