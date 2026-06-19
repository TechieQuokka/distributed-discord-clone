// 메시지별 첨부 (zustand). 업로드 결과·조회 결과를 보관. backend가 MESSAGE_CREATE에
// 첨부를 안 실어주고 사후 첨부(D37)라, 클라가 메시지→첨부를 모아 표시한다.
import { create } from "zustand";
import type { AttachmentView } from "../api/attachments.ts";
import type { Snowflake } from "../lib/ids.ts";

interface AttachmentsState {
  byMessage: Record<Snowflake, AttachmentView[]>;
  set: (messageId: Snowflake, list: AttachmentView[]) => void;
  add: (messageId: Snowflake, att: AttachmentView) => void;
}

export const useAttachments = create<AttachmentsState>((set) => ({
  byMessage: {},
  set: (messageId, list) => set((s) => ({ byMessage: { ...s.byMessage, [messageId]: list } })),
  add: (messageId, att) =>
    set((s) => {
      const cur = s.byMessage[messageId] ?? [];
      if (cur.some((a) => a.id === att.id)) return s;
      return { byMessage: { ...s.byMessage, [messageId]: [...cur, att] } };
    }),
}));
