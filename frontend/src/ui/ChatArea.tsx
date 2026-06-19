// 채팅 영역 — 히스토리(REST)+실시간(WS) 메시지, 작성/편집/삭제/답장, 라이브 리액션, 이전 더보기, 첨부.
import { useEffect, useMemo, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useMessages } from "../hooks/queries.ts";
import { useNameResolver } from "../hooks/names.ts";
import { addReaction, deleteMessage, editMessage, listMessages, removeReaction, sendMessage } from "../api/messages.ts";
import { qk } from "../api/queryKeys.ts";
import { useAuth } from "../store/auth.ts";
import { useReactions } from "../store/reactions.ts";
import { flushAttachmentViaRest, queueAttachment } from "../lib/attachmentQueue.ts";
import {
  addOptimisticMessage,
  optimisticId,
  reconcileMessageViaRest,
  removeOptimisticMessage,
} from "../lib/messageCache.ts";
import { MessageAttachments } from "./MessageAttachments.tsx";
import { SearchModal } from "./SearchModal.tsx";
import { ThreadsPanel } from "./ThreadsPanel.tsx";
import { compareSnowflake, type Snowflake } from "../lib/ids.ts";
import type { MessageView } from "../api/types.ts";

const QUICK_EMOJIS = ["👍", "❤️", "😂", "🎉", "😮", "😢", "🔥"];

export function ChatArea({
  channelId,
  channelName,
  realmId,
}: {
  channelId: Snowflake;
  channelName: string;
  realmId: Snowflake | null;
}) {
  const qc = useQueryClient();
  const myId = useAuth((s) => s.userId);
  const { data: messages, isLoading } = useMessages(channelId);
  const nameOf = useNameResolver(realmId);
  const [reply, setReply] = useState<MessageView | null>(null);
  const [loadingOlder, setLoadingOlder] = useState(false);
  const [exhausted, setExhausted] = useState(false);
  const [search, setSearch] = useState(false);
  const [threads, setThreads] = useState(false);
  const bottomRef = useRef<HTMLDivElement>(null);

  const ordered = useMemo(
    () =>
      [...(messages ?? [])].sort((a, b) => {
        // 옵티미스틱(pending)은 임시 id라 BigInt 비교 불가 → 항상 맨 끝(최신)에.
        if (a.pending || b.pending) {
          if (a.pending && b.pending) return 0;
          return a.pending ? 1 : -1;
        }
        return compareSnowflake(a.id, b.id);
      }),
    [messages],
  );

  // 답장 인용 미리보기용 — id→메시지(같은 채널 캐시 내).
  const byId = useMemo(() => new Map(ordered.map((m) => [m.id, m])), [ordered]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [ordered.length]);

  // 채널 전환 시 페이지네이션 상태 리셋.
  useEffect(() => {
    setExhausted(false);
    setReply(null);
  }, [channelId]);

  async function loadOlder() {
    if (ordered.length === 0) return;
    setLoadingOlder(true);
    try {
      const oldest = ordered[0].id;
      const older = await listMessages(channelId, { before: oldest, limit: 50 });
      if (older.length === 0) {
        setExhausted(true);
        return;
      }
      qc.setQueryData<MessageView[]>(qk.messages(channelId), (cur) => {
        const seen = new Set((cur ?? []).map((m) => m.id));
        const add = older.filter((m) => !seen.has(m.id));
        return [...(cur ?? []), ...add]; // cache는 최신순 → 오래된 건 뒤에.
      });
      if (older.length < 50) setExhausted(true);
    } finally {
      setLoadingOlder(false);
    }
  }

  return (
    <div className="flex min-w-0 flex-1 flex-col bg-bg-app">
      <header className="flex h-12 items-center gap-2 border-b border-black/20 px-4 shadow-sm">
        <span className="text-xl text-text-muted">{realmId ? "#" : "@"}</span>
        <span className="font-semibold text-text-bright">{channelName}</span>
        {realmId && (
          <div className="ml-auto flex items-center gap-1">
            <button
              onClick={() => setThreads(true)}
              title="스레드"
              className="rounded px-2 py-1 text-text-muted hover:bg-bg-hover hover:text-text-bright"
            >
              🧵
            </button>
            <button
              onClick={() => setSearch(true)}
              title="메시지 검색"
              className="rounded px-2 py-1 text-text-muted hover:bg-bg-hover hover:text-text-bright"
            >
              🔍
            </button>
          </div>
        )}
      </header>
      {search && realmId && <SearchModal realmId={realmId} onClose={() => setSearch(false)} />}
      {threads && <ThreadsPanel channelId={channelId} onClose={() => setThreads(false)} />}

      <div className="flex-1 overflow-y-auto px-4 py-4">
        {isLoading && <p className="text-text-muted">불러오는 중…</p>}
        {!isLoading && ordered.length > 0 && (
          <div className="mb-2 text-center">
            {exhausted ? (
              <span className="text-xs text-text-muted">— 채널의 시작 —</span>
            ) : (
              <button
                onClick={loadOlder}
                disabled={loadingOlder}
                className="text-xs text-accent hover:underline disabled:opacity-50"
              >
                {loadingOlder ? "불러오는 중…" : "이전 메시지 더 보기"}
              </button>
            )}
          </div>
        )}
        {!isLoading && ordered.length === 0 && (
          <p className="text-text-muted">#{channelName} 채널의 시작입니다. 첫 메시지를 보내보세요.</p>
        )}
        {ordered.map((m) => {
          const refMsg = m.reference_message_id ? byId.get(m.reference_message_id) : undefined;
          return (
            <MessageRow
              key={m.id}
              msg={m}
              authorName={nameOf(m.author_id)}
              mine={m.author_id === myId}
              channelId={channelId}
              onReply={() => setReply(m)}
              replyTo={refMsg}
              replyToName={refMsg ? nameOf(refMsg.author_id) : undefined}
            />
          );
        })}
        <div ref={bottomRef} />
      </div>

      <Composer
        channelId={channelId}
        channelName={channelName}
        reply={reply}
        replyName={reply ? nameOf(reply.author_id) : null}
        clearReply={() => setReply(null)}
      />
    </div>
  );
}

function MessageRow({
  msg,
  authorName,
  mine,
  channelId,
  onReply,
  replyTo,
  replyToName,
}: {
  msg: MessageView;
  authorName: string;
  mine: boolean;
  channelId: Snowflake;
  onReply: () => void;
  replyTo?: MessageView;
  replyToName?: string;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(msg.content);
  const [picker, setPicker] = useState(false);
  const tallies = useReactions((s) => s.byMessage[msg.id]);

  async function saveEdit() {
    if (draft.trim() && draft !== msg.content) await editMessage(channelId, msg.id, draft.trim());
    setEditing(false);
  }

  function toggleReaction(emoji: string, mineAlready: boolean) {
    if (mineAlready) removeReaction(channelId, msg.id, emoji);
    else addReaction(channelId, msg.id, emoji);
  }

  return (
    <div
      className={`group relative -mx-2 flex gap-3 rounded px-2 py-1 hover:bg-black/10 ${
        msg.pending ? "opacity-60" : ""
      }`}
    >
      <div className="mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-bg-input text-xs font-bold text-text-bright">
        {authorName.slice(0, 2).toUpperCase()}
      </div>
      <div className="min-w-0 flex-1">
        {msg.reference_message_id && (
          <div className="mb-0.5 flex items-center gap-1 text-xs text-text-muted">
            <span>↩</span>
            <span className="font-medium text-text-bright">{replyToName ?? "원본"}</span>
            <span className="truncate opacity-80">{replyTo ? replyTo.content : "메시지"}</span>
          </div>
        )}
        <span className="text-sm font-medium text-text-bright">{authorName}</span>
        {msg.pending && <span className="ml-1.5 text-xs text-text-muted">전송 중…</span>}
        {editing ? (
          <div className="mt-1 flex gap-2">
            <input
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") saveEdit();
                if (e.key === "Escape") setEditing(false);
              }}
              className="flex-1 rounded bg-bg-input px-2 py-1 text-text-bright outline-none"
              autoFocus
            />
            <button onClick={saveEdit} className="text-xs text-accent">저장</button>
            <button onClick={() => setEditing(false)} className="text-xs text-text-muted">취소</button>
          </div>
        ) : (
          <p className="whitespace-pre-wrap break-words text-text-normal">{msg.content}</p>
        )}

        <MessageAttachments messageId={msg.id} channelId={channelId} />

        {tallies && Object.keys(tallies).length > 0 && (
          <div className="mt-1 flex flex-wrap gap-1">
            {Object.entries(tallies).map(([emoji, t]) => (
              <button
                key={emoji}
                onClick={() => toggleReaction(emoji, t.mine)}
                className={`flex items-center gap-1 rounded border px-1.5 py-0.5 text-xs ${
                  t.mine ? "border-accent bg-accent/20" : "border-transparent bg-bg-input"
                }`}
              >
                <span>{emoji}</span>
                <span className="text-text-muted">{t.count}</span>
              </button>
            ))}
          </div>
        )}
      </div>

      {!msg.pending && (
      <div className="absolute right-2 -top-3 hidden items-center gap-1 rounded bg-bg-sidebar px-1 py-0.5 shadow group-hover:flex">
        <div className="relative">
          <button onClick={() => setPicker((v) => !v)} title="리액션" className="px-1 hover:scale-110">
            😀
          </button>
          {picker && (
            <div className="absolute right-0 top-7 z-10 flex gap-1 rounded bg-bg-sidebar p-1 shadow-lg">
              {QUICK_EMOJIS.map((e) => (
                <button
                  key={e}
                  onClick={() => {
                    addReaction(channelId, msg.id, e);
                    setPicker(false);
                  }}
                  className="px-1 text-lg hover:scale-125"
                >
                  {e}
                </button>
              ))}
            </div>
          )}
        </div>
        <button onClick={onReply} title="답장" className="px-1 text-text-muted hover:text-text-bright">↩</button>
        {mine && (
          <>
            <button
              onClick={() => {
                setDraft(msg.content);
                setEditing(true);
              }}
              title="편집"
              className="px-1 text-text-muted hover:text-text-bright"
            >
              ✎
            </button>
            <button
              onClick={() => deleteMessage(channelId, msg.id)}
              title="삭제"
              className="px-1 text-text-muted hover:text-dnd"
            >
              🗑
            </button>
          </>
        )}
      </div>
      )}
    </div>
  );
}

function Composer({
  channelId,
  channelName,
  reply,
  replyName,
  clearReply,
}: {
  channelId: Snowflake;
  channelName: string;
  reply: MessageView | null;
  replyName: string | null;
  clearReply: () => void;
}) {
  const qc = useQueryClient();
  const [text, setText] = useState("");
  const [file, setFile] = useState<File | null>(null);
  const fileRef = useRef<HTMLInputElement>(null);

  async function send() {
    const content = text.trim();
    if (!content && !file) return;
    setText("");
    const nonce = crypto.randomUUID();
    // 첨부가 있으면 nonce를 키로 대기열에 넣고, 메시지 id 확정 시 업로드(D37).
    const attached = file;
    if (attached) {
      queueAttachment(nonce, channelId, attached);
      setFile(null);
    }
    const body = content || attached?.name || "(첨부)";
    const myId = useAuth.getState().userId;
    const replyId = reply?.id;
    clearReply();
    // ① 옵티미스틱: 보내는 즉시 화면에 표시(WS 메아리를 기다리지 않음 → 간헐 미표시 방지).
    if (myId) {
      addOptimisticMessage(qc, channelId, {
        id: optimisticId(nonce),
        channel_id: channelId,
        author_id: myId,
        content: body,
        reference_message_id: replyId ?? null,
        nonce,
        pending: true,
      });
    }
    try {
      await sendMessage(channelId, body, { nonce, referenceMessageId: replyId });
    } catch {
      removeOptimisticMessage(qc, channelId, nonce); // 전송 실패 → 옵티미스틱 제거.
      return;
    }
    // 확정 경로: ② WS MESSAGE_CREATE(RealtimeProvider) ③ REST 백업(WS 놓침 대비).
    if (myId) void reconcileMessageViaRest(qc, channelId, nonce, myId, body);
    // 첨부도 같은 원리로 REST 백업 업로드(Option A).
    if (attached && myId) void flushAttachmentViaRest(nonce, channelId, myId, body);
  }

  return (
    <div className="px-4 pb-5">
      {reply && (
        <div className="flex items-center justify-between rounded-t bg-bg-sidebar px-3 py-1 text-xs text-text-muted">
          <span><b className="text-text-normal">{replyName}</b> 님에게 답장</span>
          <button onClick={clearReply} className="hover:text-text-bright">✕</button>
        </div>
      )}
      {file && (
        <div className="flex items-center justify-between rounded-t bg-bg-sidebar px-3 py-1 text-xs text-text-normal">
          <span>📎 {file.name} ({Math.ceil(file.size / 1024)} KB)</span>
          <button onClick={() => setFile(null)} className="text-text-muted hover:text-dnd">✕</button>
        </div>
      )}
      <div className="flex items-center gap-2 rounded-lg bg-bg-input px-4">
        <button
          onClick={() => fileRef.current?.click()}
          title="파일 첨부"
          className="text-xl text-text-muted hover:text-text-bright"
        >
          ＋
        </button>
        <input
          ref={fileRef}
          type="file"
          className="hidden"
          onChange={(e) => setFile(e.target.files?.[0] ?? null)}
        />
        <input
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              send();
            }
          }}
          placeholder={`#${channelName}에 메시지 보내기`}
          className="flex-1 bg-transparent py-3 text-text-normal outline-none placeholder:text-text-muted"
        />
      </div>
    </div>
  );
}
