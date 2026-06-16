//! 리포지토리 port (개념: repo). domain이 trait로 선언, storage가 구현(adapter) (D22).

use core::future::Future;

use crate::attachment::{Attachment, NewAttachment};
use crate::audit::{AuditEntry, NewAuditEntry};
use crate::channel::{Channel, NewChannel};
use crate::crdt::{CrdtEntry, LwwMap};
use crate::dm::{DmChannel, NewDm, NewGroupDm, RealmInfo};
use crate::event::{RealmEventKind, RealmEventRecord};
use crate::guild::{Guild, NewGuild};
use crate::id::{
    AttachmentId, ChannelId, MessageId, RealmId, RefreshTokenId, RoleId, UserId, WebhookId,
};
use crate::invite::{Invite, NewInvite};
use crate::member::Member;
use crate::message::{Message, NewMessage};
use crate::permissions::ChannelOverwrite;
use crate::read_state::ReadState;
use crate::refresh_token::{NewRefreshToken, RefreshToken};
use crate::relationship::{RelationKind, Relationship};
use crate::role::{NewRole, Role};
use crate::thread::{NewThread, Thread};
use crate::user::{NewUser, User};
use crate::webauthn::{NewWebAuthnCredential, WebAuthnCredential};
use crate::webhook::{NewWebhook, Webhook};

#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error("conflict (unique violation)")]
    Conflict,
    #[error("backend error: {0}")]
    Backend(String),
}

/// 유저 저장소 port.
pub trait UserRepository: Send + Sync {
    fn create_user(&self, user: &NewUser) -> impl Future<Output = Result<(), RepoError>> + Send;
    fn find_by_username(
        &self,
        username: &str,
    ) -> impl Future<Output = Result<Option<User>, RepoError>> + Send;
    fn find_by_id(
        &self,
        id: UserId,
    ) -> impl Future<Output = Result<Option<User>, RepoError>> + Send;
    /// TOTP secret 설정/해제 (D19). `None`=비활성화. 민감값이라 `User` 엔티티엔 안 싣고 전용 경로.
    fn set_totp_secret(
        &self,
        id: UserId,
        secret: Option<&[u8]>,
    ) -> impl Future<Output = Result<(), RepoError>> + Send;
    /// TOTP secret 조회 (없으면 MFA 미설정).
    fn totp_secret(
        &self,
        id: UserId,
    ) -> impl Future<Output = Result<Option<Vec<u8>>, RepoError>> + Send;
}

/// Refresh 토큰 저장소 port (D14). 회전 + 재사용 탐지.
pub trait RefreshTokenRepository: Send + Sync {
    fn create_refresh_token(
        &self,
        token: &NewRefreshToken,
    ) -> impl Future<Output = Result<(), RepoError>> + Send;

    /// 폐기·만료되지 않은(활성) 토큰을 해시로 조회. `now_unix`로 만료 판정.
    fn find_active(
        &self,
        token_hash: &[u8],
        now_unix: i64,
    ) -> impl Future<Output = Result<Option<RefreshToken>, RepoError>> + Send;

    /// 상태 무관하게 해시로 조회 (재사용 탐지용 — 이미 폐기된 토큰 제시 감지).
    fn find_by_hash(
        &self,
        token_hash: &[u8],
    ) -> impl Future<Output = Result<Option<RefreshToken>, RepoError>> + Send;

    /// 토큰 1개 폐기(revoked_at 표시). 멱등.
    fn revoke(
        &self,
        id: RefreshTokenId,
        now_unix: i64,
    ) -> impl Future<Output = Result<(), RepoError>> + Send;

    /// 유저의 모든 활성 토큰 폐기 (재사용 탐지 시 체인 전체 무효화, D14).
    fn revoke_all_for_user(
        &self,
        user_id: UserId,
        now_unix: i64,
    ) -> impl Future<Output = Result<(), RepoError>> + Send;
}

/// 길드/멤버십 저장소 port (DB-D1). 길드 = realm+guild+member(+@everyone 역할) 한 트랜잭션.
pub trait GuildRepository: Send + Sync {
    fn create_guild(&self, guild: &NewGuild) -> impl Future<Output = Result<(), RepoError>> + Send;

    /// 권한 검사용: 길드(owner_id 포함) 조회. realm이 길드가 아니면 None.
    fn get_guild(
        &self,
        realm_id: RealmId,
    ) -> impl Future<Output = Result<Option<Guild>, RepoError>> + Send;

    fn add_member(
        &self,
        realm_id: RealmId,
        user_id: UserId,
    ) -> impl Future<Output = Result<(), RepoError>> + Send;

    /// 권한 검사용: 유저가 Realm 멤버인가.
    fn is_member(
        &self,
        realm_id: RealmId,
        user_id: UserId,
    ) -> impl Future<Output = Result<bool, RepoError>> + Send;

    /// 멤버 단건 조회 (nick/joined/역할 포함). 멤버 아니면 None.
    fn get_member(
        &self,
        realm_id: RealmId,
        user_id: UserId,
    ) -> impl Future<Output = Result<Option<Member>, RepoError>> + Send;

    /// Realm 멤버 목록 (joined_at 오름차순).
    fn list_members(
        &self,
        realm_id: RealmId,
    ) -> impl Future<Output = Result<Vec<Member>, RepoError>> + Send;

    /// 멤버 nick 수정. 멤버 존재 시 `Ok(true)`, 없으면 `Ok(false)`. `None` = nick 제거.
    fn update_member_nick(
        &self,
        realm_id: RealmId,
        user_id: UserId,
        nick: Option<&str>,
    ) -> impl Future<Output = Result<bool, RepoError>> + Send;

    /// 멤버 제거(추방/탈퇴). 존재했으면 `Ok(true)`. member_roles는 FK CASCADE로 정리.
    fn remove_member(
        &self,
        realm_id: RealmId,
        user_id: UserId,
    ) -> impl Future<Output = Result<bool, RepoError>> + Send;

    /// 자동 구독용(D13): 유저가 속한 Realm 목록.
    fn member_realm_ids(
        &self,
        user_id: UserId,
    ) -> impl Future<Output = Result<Vec<RealmId>, RepoError>> + Send;
}

/// 초대 저장소 port (Phase 3, 스키마 `invites`). 길드 합류 토큰.
pub trait InviteRepository: Send + Sync {
    fn create_invite(&self, inv: &NewInvite) -> impl Future<Output = Result<(), RepoError>> + Send;

    fn find_invite(
        &self,
        code: &str,
    ) -> impl Future<Output = Result<Option<Invite>, RepoError>> + Send;

    /// 트랜잭션 redeem: 유효하면 멤버 추가(멱등) + uses 증가 후 `realm_id` 반환.
    /// 무효(미존재/만료/소진)면 `Ok(None)` — 호출측이 404로 매핑.
    fn redeem_invite(
        &self,
        code: &str,
        user: UserId,
        now_unix: i64,
    ) -> impl Future<Output = Result<Option<RealmId>, RepoError>> + Send;
}

/// 역할 저장소 port (D17). @everyone(id==realm_id) + 멤버 역할 할당.
pub trait RoleRepository: Send + Sync {
    fn create_role(&self, role: &NewRole) -> impl Future<Output = Result<(), RepoError>> + Send;

    fn list_roles(
        &self,
        realm_id: RealmId,
    ) -> impl Future<Output = Result<Vec<Role>, RepoError>> + Send;

    /// 멤버에게 역할 부여 (멱등). 멤버·역할이 존재해야 함.
    fn assign_role(
        &self,
        realm_id: RealmId,
        user_id: UserId,
        role_id: RoleId,
    ) -> impl Future<Output = Result<(), RepoError>> + Send;

    /// `@everyone`(id==realm_id) 역할의 권한 비트. 없으면 None.
    fn everyone_permissions(
        &self,
        realm_id: RealmId,
    ) -> impl Future<Output = Result<Option<u64>, RepoError>> + Send;

    /// 멤버에게 할당된 (비-@everyone) 역할들의 권한 비트 목록.
    fn member_role_permissions(
        &self,
        realm_id: RealmId,
        user_id: UserId,
    ) -> impl Future<Output = Result<Vec<u64>, RepoError>> + Send;

    /// 멤버의 (비-@everyone) 역할 (role_id, permissions) 목록 — 채널 오버라이드 매칭용.
    fn member_roles_with_ids(
        &self,
        realm_id: RealmId,
        user_id: UserId,
    ) -> impl Future<Output = Result<Vec<(u64, u64)>, RepoError>> + Send;
}

/// 채널 권한 오버라이드 저장소 port (D17, 스키마 `channel_overwrites`).
pub trait ChannelOverwriteRepository: Send + Sync {
    /// 오버라이드 upsert (allow/deny 갱신). allow·deny 모두 0이면 삭제로 취급해도 됨(구현 재량).
    fn set_overwrite(
        &self,
        ow: &ChannelOverwrite,
    ) -> impl Future<Output = Result<(), RepoError>> + Send;

    fn list_overwrites(
        &self,
        channel_id: ChannelId,
    ) -> impl Future<Output = Result<Vec<ChannelOverwrite>, RepoError>> + Send;
}

/// DM/그룹DM 저장소 port (Phase 3, D8/DB-D2). Realm 통일 추상 — DM도 realm+channel(+members).
///
/// 1:1 DM은 `dm_pairs(user_lo,user_hi)`로 중복 방지(find-or-create). 그룹DM은 자체 id + owner.
/// 멤버 추가/제거는 [`GuildRepository::add_member`]/[`GuildRepository::remove_member`]를 재사용한다
/// (members 테이블은 Realm 종류 무관 공용).
pub trait DmRepository: Send + Sync {
    /// 두 유저의 1:1 DM 조회(페어 정규화는 내부). 없으면 None.
    fn find_dm(
        &self,
        a: UserId,
        b: UserId,
    ) -> impl Future<Output = Result<Option<DmChannel>, RepoError>> + Send;

    /// 1:1 DM 생성: realm(dm) + channel(dm) + members 2 + dm_pairs 한 트랜잭션.
    /// 동시 생성 레이스로 페어가 이미 있으면 `Conflict` — 호출측이 [`find_dm`](Self::find_dm)로 재조회.
    fn create_dm(&self, dm: &NewDm) -> impl Future<Output = Result<(), RepoError>> + Send;

    /// 그룹DM 생성: realm(group_dm, owner) + channel(dm) + members N 한 트랜잭션.
    fn create_group_dm(
        &self,
        dm: &NewGroupDm,
    ) -> impl Future<Output = Result<(), RepoError>> + Send;

    /// Realm 메타(kind/owner) 조회 — 그룹DM 관리·권한 분기용. 없으면 None.
    fn get_realm(
        &self,
        realm_id: RealmId,
    ) -> impl Future<Output = Result<Option<RealmInfo>, RepoError>> + Send;
}

/// 친구·차단 저장소 port (Phase 3, 스키마 `relationships`). Discord식 방향성 행(02-schema §6).
///
/// 상태 전이(친구 요청/수락, 차단, 제거)는 두 방향 행을 함께 바꾸므로 **트랜잭션**으로 구현한다.
/// 권한/차단 판정은 호출측(라우트)이 [`get`](Self::get_relationship)/[`is_blocked_between`]로 먼저 확인.
pub trait RelationshipRepository: Send + Sync {
    /// 내 관계 목록(내가 user_id인 행들).
    fn list_relationships(
        &self,
        user: UserId,
    ) -> impl Future<Output = Result<Vec<Relationship>, RepoError>> + Send;

    /// 특정 상대에 대한 내 관계 한 건(방향: user→target). 없으면 None.
    fn get_relationship(
        &self,
        user: UserId,
        target: UserId,
    ) -> impl Future<Output = Result<Option<RelationKind>, RepoError>> + Send;

    /// a↔b 중 한쪽이라도 상대를 차단했는가 (1:1 DM 게이팅용, permissions.md §5).
    fn is_blocked_between(
        &self,
        a: UserId,
        b: UserId,
    ) -> impl Future<Output = Result<bool, RepoError>> + Send;

    /// 친구 요청 또는 수락(트랜잭션). 호출 전 차단 없음을 라우트가 보장.
    /// - 내 행이 `PendingIn`(상대가 먼저 요청) → 양쪽 `Friend`로 → `Friend` 반환.
    /// - 관계 없음 → 내 행 `PendingOut`/상대 행 `PendingIn` → `PendingOut` 반환.
    /// - 이미 `Friend`/`PendingOut` → 그대로 멱등 반환.
    fn friend_request_or_accept(
        &self,
        me: UserId,
        target: UserId,
    ) -> impl Future<Output = Result<RelationKind, RepoError>> + Send;

    /// 차단(트랜잭션): 내 행을 `Blocked`로 upsert + 상대 행 제거(친구/대기 해제).
    fn block(
        &self,
        me: UserId,
        target: UserId,
    ) -> impl Future<Output = Result<(), RepoError>> + Send;

    /// 관계 제거(트랜잭션). 제거된 내 행의 이전 종류를 반환(없었으면 None).
    /// 친구/대기면 양쪽 행 제거, 차단이면 내 행만 제거(상대는 영향 없음).
    fn remove_relationship(
        &self,
        me: UserId,
        target: UserId,
    ) -> impl Future<Output = Result<Option<RelationKind>, RepoError>> + Send;
}

/// 채널 저장소 port.
pub trait ChannelRepository: Send + Sync {
    fn create_channel(
        &self,
        channel: &NewChannel,
    ) -> impl Future<Output = Result<(), RepoError>> + Send;

    fn get(
        &self,
        id: ChannelId,
    ) -> impl Future<Output = Result<Option<Channel>, RepoError>> + Send;

    fn list_by_realm(
        &self,
        realm_id: RealmId,
    ) -> impl Future<Output = Result<Vec<Channel>, RepoError>> + Send;
}

/// 메시지 저장소 port (D24 persist / D34 nonce / D38 페이지네이션).
pub trait MessageRepository: Send + Sync {
    /// persist. nonce 중복이면 `Ok(false)`(삽입 안 됨, 멱등) — ON CONFLICT DO NOTHING.
    fn create_message(
        &self,
        msg: &NewMessage,
    ) -> impl Future<Output = Result<bool, RepoError>> + Send;

    /// 채널 히스토리 — `before`(Snowflake 커서) 이전 최신순 `limit`개 (D38).
    fn list_by_channel(
        &self,
        channel_id: ChannelId,
        before: Option<MessageId>,
        limit: i64,
    ) -> impl Future<Output = Result<Vec<Message>, RepoError>> + Send;

    /// 단건 조회 (살아있는 메시지만 — `deleted_at IS NULL`). 권한 판정·이벤트 조립용.
    fn get_message(
        &self,
        id: MessageId,
    ) -> impl Future<Output = Result<Option<Message>, RepoError>> + Send;

    /// 작성자 본인 편집 (D39). 작성자 일치 + 미삭제면 content 갱신 + `edited_at=now` → `Ok(true)`.
    fn edit_message(
        &self,
        id: MessageId,
        author: UserId,
        content: &str,
    ) -> impl Future<Output = Result<bool, RepoError>> + Send;

    /// 소프트 삭제 (D39, `deleted_at` 표시). 권한 검사는 호출측. 살아있던 메시지면 `Ok(true)`.
    fn soft_delete_message(
        &self,
        id: MessageId,
    ) -> impl Future<Output = Result<bool, RepoError>> + Send;

    /// 멘션 적재 (D39, `message_mentions`). 존재하는 유저만(어댑터가 보장), 멱등. 빈 목록은 no-op.
    fn add_mentions(
        &self,
        message_id: MessageId,
        users: &[UserId],
    ) -> impl Future<Output = Result<(), RepoError>> + Send;

    /// 전문검색 (Q10, Postgres FTS). `realm` 안에서 `channels`(호출측이 VIEW_CHANNEL로 필터한
    /// 허용 채널)에 한해 `query`(websearch 구문)에 매칭되는 살아있는 메시지를 최신순 `limit`개.
    /// `channels`가 비면 결과 없음. 권한 판정은 호출측(라우트)이 수행.
    fn search_messages(
        &self,
        realm: RealmId,
        channels: &[ChannelId],
        query: &str,
        limit: i64,
    ) -> impl Future<Output = Result<Vec<Message>, RepoError>> + Send;
}

/// 리액션 저장소 port (Phase 3, D39, 스키마 `reactions` V7). 유니코드 emoji.
pub trait ReactionRepository: Send + Sync {
    /// 본인 리액션 추가 (멱등). 새로 추가면 `Ok(true)`, 이미 있으면 `Ok(false)`.
    fn add_reaction(
        &self,
        message_id: MessageId,
        user: UserId,
        emoji: &str,
    ) -> impl Future<Output = Result<bool, RepoError>> + Send;

    /// 본인 리액션 제거. 있던 것을 지웠으면 `Ok(true)`.
    fn remove_reaction(
        &self,
        message_id: MessageId,
        user: UserId,
        emoji: &str,
    ) -> impl Future<Output = Result<bool, RepoError>> + Send;
}

/// 읽음 상태 저장소 port (Phase 3, 스키마 `read_states`). 채널별 last_read + 안 읽은 멘션 수.
pub trait ReadStateRepository: Send + Sync {
    /// 채널을 `message`까지 읽음 처리(upsert). `mention_count`는 그 이후 멘션 수로 재계산 → 결과 반환.
    fn ack(
        &self,
        user: UserId,
        channel: ChannelId,
        message: MessageId,
    ) -> impl Future<Output = Result<ReadState, RepoError>> + Send;

    /// 멘션 발생 시 대상들의 `mention_count` +1 (존재 유저만, upsert; last_read 없으면 NULL로 생성).
    /// 빈 목록은 no-op. 새 메시지는 항상 최신이라 last_read 이후 → 단순 증가가 정확.
    fn bump_mentions(
        &self,
        channel: ChannelId,
        users: &[UserId],
    ) -> impl Future<Output = Result<(), RepoError>> + Send;

    /// 유저의 모든 읽음 상태 (READY 스냅샷용).
    fn list_read_states(
        &self,
        user: UserId,
    ) -> impl Future<Output = Result<Vec<ReadState>, RepoError>> + Send;
}

/// 스레드 저장소 port (Phase 4, 스키마 `thread_meta` V13). 스레드 = channels(kind=thread)+thread_meta.
///
/// 채널 생성/조회는 [`ChannelRepository`]를 재사용하고, 여기선 thread_meta 보강 + 부모별 목록만.
pub trait ThreadRepository: Send + Sync {
    /// 스레드 생성(트랜잭션): channels(kind='thread', parent_id) + thread_meta(owner) 한 번에.
    fn create_thread(&self, t: &NewThread) -> impl Future<Output = Result<(), RepoError>> + Send;

    /// 스레드 단건 조회 (channels+thread_meta 조인, message_count 집계). 스레드 아니면 None.
    fn get_thread(
        &self,
        channel: ChannelId,
    ) -> impl Future<Output = Result<Option<Thread>, RepoError>> + Send;

    /// 부모 채널의 스레드 목록 (최신순).
    fn list_threads(
        &self,
        parent: ChannelId,
    ) -> impl Future<Output = Result<Vec<Thread>, RepoError>> + Send;

    /// 스레드 아카이브/해제. 스레드였으면 `Ok(true)`.
    fn set_thread_archived(
        &self,
        channel: ChannelId,
        archived: bool,
    ) -> impl Future<Output = Result<bool, RepoError>> + Send;
}

/// 첨부 메타 저장소 port (Phase 4, D37, 스키마 `attachments` V14). 바이트는 [`crate::blob::BlobStore`].
pub trait AttachmentRepository: Send + Sync {
    fn add_attachment(&self, a: &NewAttachment) -> impl Future<Output = Result<(), RepoError>> + Send;

    /// 메시지의 첨부 목록.
    fn list_attachments(
        &self,
        message_id: MessageId,
    ) -> impl Future<Output = Result<Vec<Attachment>, RepoError>> + Send;

    /// 첨부 단건 조회 (다운로드 시 메시지→채널 권한 판정용). 없으면 None.
    fn get_attachment(
        &self,
        id: AttachmentId,
    ) -> impl Future<Output = Result<Option<Attachment>, RepoError>> + Send;
}

/// 웹훅 저장소 port (Phase 4, 스키마 `webhooks` V15). 토큰으로 채널에 메시지 게시.
pub trait WebhookRepository: Send + Sync {
    fn create_webhook(&self, w: &NewWebhook) -> impl Future<Output = Result<(), RepoError>> + Send;

    /// 채널의 웹훅 목록.
    fn list_webhooks(
        &self,
        channel_id: ChannelId,
    ) -> impl Future<Output = Result<Vec<Webhook>, RepoError>> + Send;

    /// 단건 조회 (실행 시 token_hash 검증용). 없으면 None.
    fn get_webhook(
        &self,
        id: WebhookId,
    ) -> impl Future<Output = Result<Option<Webhook>, RepoError>> + Send;

    /// 웹훅 삭제. 있었으면 `Ok(true)`.
    fn delete_webhook(&self, id: WebhookId) -> impl Future<Output = Result<bool, RepoError>> + Send;
}

/// 감사 로그 저장소 port (Phase 4, 스키마 `audit_log_entries` V16).
pub trait AuditRepository: Send + Sync {
    /// 감사 항목 기록. 실패해도 주 동작은 진행(호출측이 warn) — 기록은 best-effort.
    fn log_audit(&self, e: &NewAuditEntry) -> impl Future<Output = Result<(), RepoError>> + Send;

    /// 길드 감사 로그 (최신순, `before` Snowflake 커서 + limit).
    fn list_audit(
        &self,
        realm: RealmId,
        before: Option<u64>,
        limit: i64,
    ) -> impl Future<Output = Result<Vec<AuditEntry>, RepoError>> + Send;
}

/// WebAuthn 자격증명 저장소 port (Phase 5, D19, 스키마 `webauthn_credentials` V18).
/// crypto/ceremony는 auth(webauthn-rs, P6); 여기선 자격증명(opaque passkey_json) 보관만.
pub trait WebAuthnRepository: Send + Sync {
    /// 등록 ceremony finish 후 자격증명 저장. credential_id 중복이면 `Conflict`.
    fn add_credential(
        &self,
        c: &NewWebAuthnCredential,
    ) -> impl Future<Output = Result<(), RepoError>> + Send;

    /// 유저의 등록 자격증명 목록 (인증 ceremony 로드 + 등록 exclude).
    fn list_credentials(
        &self,
        user: UserId,
    ) -> impl Future<Output = Result<Vec<WebAuthnCredential>, RepoError>> + Send;

    /// counter 증가 등으로 갱신된 passkey 재저장 (credential_id로 매칭). 없으면 no-op.
    fn update_credential(
        &self,
        credential_id: &[u8],
        passkey_json: &str,
    ) -> impl Future<Output = Result<(), RepoError>> + Send;
}

/// 이벤트 소싱 로그 port (Phase 5, D23/D48, 스키마 `realm_events` V20). append-only 도메인 사실 로그.
///
/// per-realm 단조 seq를 어댑터가 부여한다. **단일 직렬 소비자**(dispatch 드라이버, D24)가 append
/// 하므로 seq 경합 없음(nonce 멱등 D34와 같은 앱레벨 직렬화 논리). 재생은 `RealmProjection`이 fold.
pub trait EventLogRepository: Send + Sync {
    /// (realm, kind) 추가 → 부여된 per-realm seq 반환(1부터 단조).
    fn append_event(
        &self,
        realm: RealmId,
        kind: &RealmEventKind,
    ) -> impl Future<Output = Result<u64, RepoError>> + Send;

    /// `after_seq` 이후의 이벤트를 seq 오름차순으로 (rehydrate/증분 재생, D23/D35).
    fn replay_events(
        &self,
        realm: RealmId,
        after_seq: u64,
    ) -> impl Future<Output = Result<Vec<RealmEventRecord>, RepoError>> + Send;
}

/// CRDT 오프라인 동기화 저장소 port (Phase 5, D49, 스키마 `user_crdt_entries` V21).
///
/// 유저 동기화 문서 = 키별 LWW(`LwwMap`). 병합 권위는 domain(`LwwMap`)이고, 어댑터는 LWW 가드
/// upsert((ts,node) 큰 것 채택)로 영속. 여러 기기가 오프라인 편집 후 push해도 충돌 없이 수렴.
pub trait CrdtRepository: Send + Sync {
    /// 유저 동기화 문서 전체 로드 (LwwMap 재구성, 툼스톤 포함).
    fn load_user_doc(
        &self,
        user: UserId,
    ) -> impl Future<Output = Result<LwwMap, RepoError>> + Send;

    /// 들어온 엔트리를 LWW로 병합(upsert) 후 병합된 문서를 반환. 멱등(같은 엔트리 재전송 무해).
    fn merge_user_doc(
        &self,
        user: UserId,
        entries: &[CrdtEntry],
    ) -> impl Future<Output = Result<LwwMap, RepoError>> + Send;
}

/// 모든 저장소 port를 한 타입이 구현 — 조합 루트에서 제네릭 1개로 주입 (제네릭 폭발 방지).
pub trait Store:
    UserRepository
    + RefreshTokenRepository
    + GuildRepository
    + RoleRepository
    + ChannelOverwriteRepository
    + InviteRepository
    + DmRepository
    + RelationshipRepository
    + ChannelRepository
    + MessageRepository
    + ReactionRepository
    + ReadStateRepository
    + ThreadRepository
    + AttachmentRepository
    + WebhookRepository
    + AuditRepository
    + WebAuthnRepository
    + EventLogRepository
    + CrdtRepository
{
}

impl<T> Store for T where
    T: UserRepository
        + RefreshTokenRepository
        + GuildRepository
        + RoleRepository
        + ChannelOverwriteRepository
        + InviteRepository
        + DmRepository
        + RelationshipRepository
        + ChannelRepository
        + MessageRepository
        + ReactionRepository
        + ReadStateRepository
        + ThreadRepository
        + AttachmentRepository
        + WebhookRepository
        + AuditRepository
        + WebAuthnRepository
        + EventLogRepository
        + CrdtRepository
{
}
