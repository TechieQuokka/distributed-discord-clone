// 첨부 대기열 — 메시지 전송 시 nonce로 파일을 보관했다가, 메시지 id가 확정되면 그 메시지에
// 업로드한다(사후 첨부 D37). 송신자만 동작.
// id 확정 경로는 둘: ① WS MESSAGE_CREATE(빠름, 기본) ② REST 히스토리 재조회(백업, Option A).
// 전송(POST /messages)은 응답 본문이 없어(D24) id를 WS로만 받는데, WS가 재연결로 그 프레임을
// 놓치면 업로드가 영영 안 됐다 — 그래서 REST 백업으로 신뢰성을 보장한다.
import { uploadAttachment } from "../api/attachments.ts";
import { listMessages } from "../api/messages.ts";
import { useAttachments } from "../store/attachments.ts";
import type { Snowflake } from "./ids.ts";

interface Pending {
  channelId: Snowflake;
  file: File;
}
const pending = new Map<string, Pending>();

export function queueAttachment(nonce: string, channelId: Snowflake, file: File): void {
  pending.set(nonce, { channelId, file });
}

/**
 * 그 nonce의 파일이 대기 중이면 messageId에 업로드하고 store에 적재.
 * `get`+`delete`는 동기라 WS·REST 두 경로가 동시에 들어와도 단 한 번만 업로드된다(원자적 선점).
 */
export async function flushAttachmentFor(nonce: string | null, messageId: Snowflake): Promise<void> {
  if (!nonce) return;
  const p = pending.get(nonce);
  if (!p) return;
  pending.delete(nonce);
  try {
    const att = await uploadAttachment(p.channelId, messageId, p.file);
    useAttachments.getState().add(messageId, att);
  } catch {
    /* 업로드 실패는 조용히 — 메시지 자체는 이미 전송됨. */
  }
}

/**
 * REST 백업 flush(Option A) — WS의 MESSAGE_CREATE 프레임을 놓쳐도 업로드를 보장한다.
 * 전송 직후 REST 히스토리에서 내가 보낸 동일 content의 최신 메시지를 찾아 그 id로 flush한다
 * (히스토리엔 nonce가 안 실려 content로 매칭 — 전송 직후라 최신순 맨 앞에 있다).
 * WS가 먼저 처리했으면 pending이 비어 즉시 종료. persist 반영 지연 대비 짧게 재시도.
 */
export async function flushAttachmentViaRest(
  nonce: string,
  channelId: Snowflake,
  myId: Snowflake,
  content: string,
): Promise<void> {
  for (let attempt = 0; attempt < 6 && pending.has(nonce); attempt++) {
    try {
      const recent = await listMessages(channelId, { limit: 10 });
      const msg = recent.find((m) => m.author_id === myId && m.content === content);
      if (msg) {
        await flushAttachmentFor(nonce, msg.id);
        return;
      }
    } catch {
      /* 일시 실패 — 재시도(또는 그 사이 WS가 처리). */
    }
    await new Promise((r) => setTimeout(r, 350));
  }
}
