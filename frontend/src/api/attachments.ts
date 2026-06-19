// 첨부 API (CLI `rest.rs` 미러). 멀티파트 업로드 + 인증 GET(이미지 표시용 blob).
import { API_BASE, ApiError, getAccessToken } from "./http.ts";
import type { Snowflake } from "../lib/ids.ts";

export interface AttachmentView {
  id: Snowflake;
  filename: string;
  size_bytes: number;
  url: string;
}

/** 메시지에 파일 첨부(작성자 본인, 8 MiB, D37). 사후 첨부 — 메시지가 먼저 존재해야 함. */
export async function uploadAttachment(
  channelId: Snowflake,
  messageId: Snowflake,
  file: File,
): Promise<AttachmentView> {
  const form = new FormData();
  form.append("file", file, file.name);
  const headers: Record<string, string> = {};
  const tok = getAccessToken();
  if (tok) headers["authorization"] = `Bearer ${tok}`;
  const res = await fetch(`${API_BASE}/channels/${channelId}/messages/${messageId}/attachments`, {
    method: "POST",
    headers,
    body: form,
  });
  if (!res.ok) throw new ApiError(res.status, await res.text().catch(() => ""));
  return (await res.json()) as AttachmentView;
}

export async function listAttachments(
  channelId: Snowflake,
  messageId: Snowflake,
): Promise<AttachmentView[]> {
  const headers: Record<string, string> = {};
  const tok = getAccessToken();
  if (tok) headers["authorization"] = `Bearer ${tok}`;
  const res = await fetch(`${API_BASE}/channels/${channelId}/messages/${messageId}/attachments`, { headers });
  if (!res.ok) throw new ApiError(res.status, await res.text().catch(() => ""));
  return (await res.json()) as AttachmentView[];
}

/** 첨부 바이트를 인증 fetch로 받아 object URL 생성(이미지 <img>·다운로드용). 호출측이 revoke. */
export async function fetchAttachmentObjectUrl(attachmentId: Snowflake): Promise<string> {
  const headers: Record<string, string> = {};
  const tok = getAccessToken();
  if (tok) headers["authorization"] = `Bearer ${tok}`;
  const res = await fetch(`${API_BASE}/attachments/${attachmentId}`, { headers });
  if (!res.ok) throw new ApiError(res.status, await res.text().catch(() => ""));
  const blob = await res.blob();
  return URL.createObjectURL(blob);
}

export function isImageFilename(name: string): boolean {
  return /\.(png|jpe?g|gif|webp|bmp|svg|avif)$/i.test(name);
}
