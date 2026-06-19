// backend 계약 타입 — CLI `rest.rs`의 serde 뷰 + gateway.md 페이로드를 그대로 미러링.
// 모든 id는 Snowflake(string).
import type { Snowflake } from "../lib/ids.ts";

// ── Auth ────────────────────────────────────────────────────────────────
export interface AuthResponse {
  user_id: Snowflake;
  access_token: string;
  refresh_token: string;
}

export interface PowChallenge {
  challenge: string;
  difficulty: number;
}

/** 로그인 결과: 토큰 또는 MFA 2단계 필요(`{mfa_required:true}`). */
export type LoginResult = AuthResponse | { mfa_required: true };

export interface MfaEnableView {
  secret: string;
  otpauth_uri: string;
}

// ── Realms / Guilds / Channels ───────────────────────────────────────────
export type RealmKind = "guild" | "dm" | "group_dm";
export type ChannelKind =
  | "text"
  | "voice"
  | "category"
  | "announcement"
  | "forum"
  | "thread"
  | "dm";

/** GET /users/@me/realms (v1.51 신규). */
export interface RealmView {
  id: Snowflake;
  kind: RealmKind;
  name: string | null;
  owner_id: Snowflake | null;
}

/** GET /guilds/:id/channels (v1.51 신규) · create-guild/channel 응답의 채널. */
export interface ChannelView {
  id: Snowflake;
  name: string | null;
  kind: ChannelKind;
}

/** POST /guilds 응답. */
export interface GuildView {
  id: Snowflake;
  name: string;
  channels: ChannelView[];
}

export interface InviteView {
  code: string;
  realm_id: Snowflake;
  max_uses: number;
  expires_at: number | null;
}

/** POST /invites/:code 응답. */
export interface JoinView {
  realm_id: Snowflake;
  channels: { id: Snowflake; name: string | null }[];
}

// ── Members / Roles ──────────────────────────────────────────────────────
export interface MemberView {
  user_id: Snowflake;
  nick: string | null;
  joined_at: number;
  roles: Snowflake[];
}

export interface RoleView {
  id: Snowflake;
  name: string;
  permissions: string; // u64 as string
  position: number;
}

// ── Messages ─────────────────────────────────────────────────────────────
/** GET /channels/:id/messages 의 한 행(히스토리). 최신순. */
export interface MessageView {
  id: Snowflake;
  channel_id: Snowflake;
  author_id: Snowflake;
  content: string;
  /** 답장 대상 메시지 id. WS MESSAGE_CREATE에서 옴(히스토리 REST엔 아직 없음 → 라이브만 인용 표시). */
  reference_message_id?: Snowflake | null;
  // ── 클라 전용(서버 응답엔 없음) — 옵티미스틱 렌더링용 ──
  /** 전송 시 생성한 nonce. WS/REST 확정 시 이 키로 옵티미스틱을 실제 메시지와 매칭. */
  nonce?: string | null;
  /** 전송 중(옵티미스틱) 표시. 확정되면 사라짐. */
  pending?: boolean;
}

// ── DM ───────────────────────────────────────────────────────────────────
export interface DmChannelView {
  id: Snowflake;
  realm_id: Snowflake;
  kind: RealmKind;
  recipients: Snowflake[];
}

// ── Relationships ────────────────────────────────────────────────────────
export type RelationKind = "friend" | "pending_in" | "pending_out" | "blocked";
export interface RelationshipView {
  user_id: Snowflake;
  kind: RelationKind;
}

// ── Read states ──────────────────────────────────────────────────────────
export interface ReadStateView {
  channel_id: Snowflake;
  last_read_message_id: Snowflake | null;
  mention_count: number;
}

// ── Gateway READY 스냅샷 (gateway.md §2) ──────────────────────────────────
export type PresenceStatus = "online" | "idle" | "dnd" | "offline";

export interface ReadyPayload {
  session_id: string;
  resume_token: string;
  user: { id: Snowflake; username: string | null };
  realms: { id: Snowflake }[];
  read_states?: ReadStateView[];
  relationships?: RelationshipView[];
  presences?: { user: { id: Snowflake }; status: PresenceStatus }[];
  last_message_ids?: { channel_id: Snowflake; last_message_id: Snowflake }[];
}

// ── Gateway DISPATCH 페이로드(주요) ───────────────────────────────────────
export interface MessageCreatePayload {
  id: Snowflake;
  channel_id: Snowflake;
  author: { id: Snowflake };
  content: string;
  nonce: string | null;
  reference_message_id: Snowflake | null;
  mentions: Snowflake[];
}

export interface MessageUpdatePayload {
  id: Snowflake;
  channel_id: Snowflake;
  author: { id: Snowflake };
  content: string;
  edited: boolean;
}

export interface MessageDeletePayload {
  id: Snowflake;
  channel_id: Snowflake;
}

export interface ReactionPayload {
  message_id: Snowflake;
  channel_id: Snowflake;
  user_id: Snowflake;
  emoji: string;
}

export interface PresenceUpdatePayload {
  user: { id: Snowflake };
  status: PresenceStatus;
}

export interface RelationshipAddPayload {
  user: { id: Snowflake; username?: string | null };
  kind: RelationKind;
}
