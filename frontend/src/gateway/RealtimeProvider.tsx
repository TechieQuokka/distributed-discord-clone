// 실시간 브리지: 인증되면 GatewayClient를 띄우고 DISPATCH를 React Query 캐시 + 세션 store로 흘린다.
// (D30: REST=React Query / WS=네이티브 클라이언트. 둘의 접합부.)
import { useEffect } from "react";
import type { ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { GatewayClient } from "./connection.ts";
import { useAuth } from "../store/auth.ts";
import { useSession } from "../store/session.ts";
import { useReactions } from "../store/reactions.ts";
import { useVoice } from "../store/voice.ts";
import { learnUser } from "../store/directory.ts";
import { flushAttachmentFor } from "../lib/attachmentQueue.ts";
import { upsertRealMessage } from "../lib/messageCache.ts";
import { qk } from "../api/queryKeys.ts";
import type {
  MessageCreatePayload,
  MessageDeletePayload,
  MessageUpdatePayload,
  MessageView,
  PresenceUpdatePayload,
  ReactionPayload,
  ReadStateView,
  ReadyPayload,
  RelationshipAddPayload,
} from "../api/types.ts";
import type { Snowflake } from "../lib/ids.ts";

// 모듈 단일 인스턴스 — StrictMode 이중 마운트에도 소켓 1개만 유지.
let client: GatewayClient | null = null;

export function RealtimeProvider({ children }: { children: ReactNode }) {
  const qc = useQueryClient();
  const token = useAuth((s) => s.accessToken);
  const isAuthed = useAuth((s) => s.isAuthed);
  const setUsername = useAuth((s) => s.setUsername);

  useEffect(() => {
    if (!isAuthed || !token) return;

    const session = useSession.getState();

    const gateway = new GatewayClient(token, {
      onStateChange: (st) => useSession.getState().setGatewayState(st),
      onReady: (d: ReadyPayload) => {
        useSession.getState().applyReady(d);
        if (d.user.username) {
          setUsername(d.user.username);
          learnUser(d.user.id, d.user.username);
        }
        // 가입한 realm 목록이 바뀌었을 수 있으니 무효화(채널은 진입 시 로드).
        qc.invalidateQueries({ queryKey: qk.realms() });
      },
      onResumed: () => {
        // 재개 — 놓친 프레임은 버퍼 재생으로 이미 onDispatch에 흘렀다.
      },
      onDispatch: (t, d, _s) => {
        switch (t) {
          case "MESSAGE_CREATE": {
            const p = d as MessageCreatePayload;
            // nonce가 같은 옵티미스틱이 있으면 교체(전송 확정), 없으면 추가(남이 보낸 메시지).
            upsertRealMessage(
              qc,
              p.channel_id,
              {
                id: p.id,
                channel_id: p.channel_id,
                author_id: p.author.id,
                content: p.content,
                reference_message_id: p.reference_message_id,
              },
              p.nonce,
            );
            useSession.getState().bumpLastMessage(p.channel_id, p.id);
            // 내가 첨부를 단 메시지면(nonce 매칭) 이제 message id가 생겼으니 업로드.
            void flushAttachmentFor(p.nonce, p.id);
            break;
          }
          case "MESSAGE_UPDATE": {
            const p = d as MessageUpdatePayload;
            qc.setQueryData<MessageView[]>(qk.messages(p.channel_id), (old) =>
              (old ?? []).map((m) => (m.id === p.id ? { ...m, content: p.content } : m)),
            );
            break;
          }
          case "MESSAGE_DELETE": {
            const p = d as MessageDeletePayload;
            qc.setQueryData<MessageView[]>(qk.messages(p.channel_id), (old) =>
              (old ?? []).filter((m) => m.id !== p.id),
            );
            break;
          }
          case "MESSAGE_ACK": {
            const p = d as ReadStateView;
            useSession.getState().setReadState(p);
            break;
          }
          case "MESSAGE_REACTION_ADD": {
            const p = d as ReactionPayload;
            const myId = useAuth.getState().userId;
            useReactions.getState().add(p.message_id, p.emoji, p.user_id === myId);
            break;
          }
          case "MESSAGE_REACTION_REMOVE": {
            const p = d as ReactionPayload;
            const myId = useAuth.getState().userId;
            useReactions.getState().remove(p.message_id, p.emoji, p.user_id === myId);
            break;
          }
          case "PRESENCE_UPDATE": {
            const p = d as PresenceUpdatePayload;
            useSession.getState().setPresence(p.user.id, p.status);
            break;
          }
          case "RELATIONSHIP_ADD": {
            const p = d as RelationshipAddPayload;
            learnUser(p.user.id, p.user.username);
            useSession.getState().setRelationship(p.user.id, p.kind);
            qc.invalidateQueries({ queryKey: qk.relationships() });
            break;
          }
          case "RELATIONSHIP_REMOVE": {
            const p = d as { user: { id: Snowflake } };
            useSession.getState().removeRelationship(p.user.id);
            qc.invalidateQueries({ queryKey: qk.relationships() });
            break;
          }
          case "GUILD_MEMBER_ADD":
          case "GUILD_MEMBER_UPDATE":
          case "GUILD_MEMBER_REMOVE": {
            const md = d as { realm_id?: Snowflake; user?: { id: Snowflake; username?: string | null } };
            if (md.user) learnUser(md.user.id, md.user.username);
            if (md.realm_id) qc.invalidateQueries({ queryKey: qk.members(md.realm_id) });
            break;
          }
          case "CHANNEL_RECIPIENT_ADD": {
            const rd = d as { realm_id?: Snowflake; user?: { id: Snowflake; username?: string | null } };
            if (rd.user) learnUser(rd.user.id, rd.user.username);
            qc.invalidateQueries({ queryKey: qk.realms() });
            break;
          }
          case "CHANNEL_RECIPIENT_REMOVE": {
            qc.invalidateQueries({ queryKey: qk.realms() });
            break;
          }
          case "VOICE_STATE_UPDATE": {
            const p = d as {
              channel_id: Snowflake | null;
              user_id: Snowflake;
              self_mute?: boolean;
              self_deaf?: boolean;
            };
            useVoice.getState().apply({
              channel_id: p.channel_id,
              user_id: p.user_id,
              self_mute: p.self_mute ?? false,
              self_deaf: p.self_deaf ?? false,
            });
            break;
          }
          case "CHANNEL_CREATE":
          case "CHANNEL_UPDATE":
          case "CHANNEL_DELETE":
          case "THREAD_CREATE":
          case "THREAD_UPDATE": {
            const realmId = (d as { realm_id?: Snowflake }).realm_id;
            if (realmId) qc.invalidateQueries({ queryKey: qk.channels(realmId) });
            break;
          }
          default:
            break;
        }
      },
    });

    client = gateway;
    session.setGatewayState("connecting");
    gateway.connect();

    return () => {
      gateway.close();
      client = null;
      useSession.getState().reset();
    };
  }, [isAuthed, token, qc, setUsername]);

  return children;
}

/** UI에서 op3 presence·op4 voice를 보내기 위한 접근자. */
export function getGatewayClient(): GatewayClient | null {
  return client;
}
