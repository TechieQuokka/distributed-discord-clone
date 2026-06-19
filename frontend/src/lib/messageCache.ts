// 메시지 캐시 헬퍼 — 옵티미스틱 렌더링 + 확정(reconcile).
// 전송(POST /messages)은 응답 본문이 없어(D24) 메시지가 WS MESSAGE_CREATE로만 화면에 떴다.
// → WS가 그 프레임을 놓치면 "내가 쓴 글이 안 보이는" 증상. 그래서 보내는 즉시 옵티미스틱으로
//   그리고(① 즉시 표시), WS 메아리(②) 또는 REST 재조회(③)로 실제 메시지로 확정한다.
import type { QueryClient } from "@tanstack/react-query";
import { qk } from "../api/queryKeys.ts";
import { listMessages } from "../api/messages.ts";
import type { MessageView } from "../api/types.ts";
import type { Snowflake } from "./ids.ts";

/** 옵티미스틱 임시 id. 실제 Snowflake(숫자)와 안 겹치게 접두사. */
export function optimisticId(nonce: string): Snowflake {
  return `opt-${nonce}`;
}

/** 옵티미스틱 id 판별(정렬·첨부 조회에서 실제 id와 구분). */
export function isOptimisticId(id: Snowflake): boolean {
  return !/^\d+$/.test(id);
}

/** 전송 즉시 화면에 표시할 옵티미스틱 메시지를 캐시 앞(최신)에 넣는다. */
export function addOptimisticMessage(qc: QueryClient, channelId: Snowflake, msg: MessageView): void {
  qc.setQueryData<MessageView[]>(qk.messages(channelId), (old) => [msg, ...(old ?? [])]);
}

/** 전송 실패 시 옵티미스틱 제거. */
export function removeOptimisticMessage(qc: QueryClient, channelId: Snowflake, nonce: string): void {
  qc.setQueryData<MessageView[]>(qk.messages(channelId), (old) =>
    (old ?? []).filter((m) => !(m.pending && m.nonce === nonce)),
  );
}

/**
 * 실제 메시지를 캐시에 반영. 같은 nonce의 옵티미스틱이 있으면 그 자리에서 교체(전송 확정),
 * 없으면 id 중복 제거 후 앞에 추가. WS·REST 두 확정 경로가 동시에 와도 한 번만 반영된다.
 */
export function upsertRealMessage(
  qc: QueryClient,
  channelId: Snowflake,
  msg: MessageView,
  nonce: string | null,
): void {
  qc.setQueryData<MessageView[]>(qk.messages(channelId), (old) => {
    const list = old ?? [];
    if (nonce) {
      const idx = list.findIndex((m) => m.pending && m.nonce === nonce);
      if (idx >= 0) {
        const next = [...list];
        next[idx] = msg; // 옵티미스틱 → 실제(자리·순서 유지).
        return next;
      }
    }
    if (list.some((m) => m.id === msg.id)) return list; // 이미 있음(중복).
    return [msg, ...list];
  });
}

/**
 * REST 백업 확정(③) — WS MESSAGE_CREATE를 놓쳐도 옵티미스틱을 실제 메시지로 확정한다.
 * 히스토리엔 nonce가 안 실려 author+content로 매칭(전송 직후라 최신순 맨 앞).
 * WS(②)가 먼저 확정했으면 옵티미스틱이 사라져 즉시 종료. persist 반영 지연 대비 짧게 재시도.
 */
export async function reconcileMessageViaRest(
  qc: QueryClient,
  channelId: Snowflake,
  nonce: string,
  myId: Snowflake,
  content: string,
): Promise<void> {
  for (let attempt = 0; attempt < 5; attempt++) {
    const list = qc.getQueryData<MessageView[]>(qk.messages(channelId)) ?? [];
    if (!list.some((m) => m.pending && m.nonce === nonce)) return; // 이미 확정됨.
    try {
      const recent = await listMessages(channelId, { limit: 10 });
      const real = recent.find((m) => m.author_id === myId && m.content === content);
      if (real) {
        upsertRealMessage(qc, channelId, real, nonce);
        return;
      }
    } catch {
      /* 일시 실패 — 재시도(또는 그 사이 WS가 확정). */
    }
    await new Promise((r) => setTimeout(r, 350));
  }
}
