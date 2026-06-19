// 메시지 첨부 표시. store에 없으면 한 번 lazy 조회(송신자/수신자/히스토리 모두 보이게).
// 이미지는 인증 fetch로 blob URL을 만들어 <img>, 그 외엔 다운로드 버튼.
import { useEffect, useState } from "react";
import { fetchAttachmentObjectUrl, isImageFilename, listAttachments, type AttachmentView } from "../api/attachments.ts";
import { useAttachments } from "../store/attachments.ts";
import { isOptimisticId } from "../lib/messageCache.ts";
import type { Snowflake } from "../lib/ids.ts";

// 메시지별 1회만 조회(빈 결과도 기억). 채널 열 때 N 요청이지만 로컬 study 범위에서 허용.
const fetched = new Set<Snowflake>();

export function MessageAttachments({
  messageId,
  channelId,
}: {
  messageId: Snowflake;
  channelId: Snowflake;
}) {
  const stored = useAttachments((s) => s.byMessage[messageId]);
  const setList = useAttachments((s) => s.set);

  useEffect(() => {
    // 옵티미스틱(전송 중) 메시지는 임시 id라 서버 조회 불가 — 확정 후 실제 id로 다시 마운트된다.
    if (isOptimisticId(messageId) || stored || fetched.has(messageId)) return;
    fetched.add(messageId);
    listAttachments(channelId, messageId)
      .then((list) => {
        if (list.length > 0) setList(messageId, list);
      })
      .catch(() => {
        /* 권한/네트워크 실패 무시. */
      });
  }, [messageId, channelId, stored, setList]);

  if (!stored || stored.length === 0) return null;
  return (
    <div className="mt-1 flex flex-col gap-2">
      {stored.map((a) => (
        <AttachmentItem key={a.id} att={a} />
      ))}
    </div>
  );
}

function AttachmentItem({ att }: { att: AttachmentView }) {
  const image = isImageFilename(att.filename);
  const [url, setUrl] = useState<string | null>(null);

  useEffect(() => {
    let revoked: string | null = null;
    if (image) {
      fetchAttachmentObjectUrl(att.id)
        .then((u) => {
          revoked = u;
          setUrl(u);
        })
        .catch(() => {});
    }
    return () => {
      if (revoked) URL.revokeObjectURL(revoked);
    };
  }, [att.id, image]);

  async function download() {
    const u = await fetchAttachmentObjectUrl(att.id);
    const a = document.createElement("a");
    a.href = u;
    a.download = att.filename;
    a.click();
    URL.revokeObjectURL(u);
  }

  if (image) {
    return url ? (
      <img
        src={url}
        alt={att.filename}
        className="max-h-80 max-w-md cursor-pointer rounded"
        onClick={download}
      />
    ) : (
      <div className="text-xs text-text-muted">🖼 {att.filename} 불러오는 중…</div>
    );
  }

  return (
    <button
      onClick={download}
      className="flex w-fit items-center gap-2 rounded bg-bg-sidebar px-3 py-2 text-left text-sm hover:bg-bg-hover"
    >
      <span className="text-xl">📄</span>
      <span>
        <span className="block text-text-bright">{att.filename}</span>
        <span className="text-xs text-text-muted">{Math.ceil(att.size_bytes / 1024)} KB · 다운로드</span>
      </span>
    </button>
  );
}
