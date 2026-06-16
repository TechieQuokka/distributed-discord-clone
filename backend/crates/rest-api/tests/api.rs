//! rest-api 통합 테스트 — in-memory `Store` + axum `oneshot`으로 핸들러·추출기·권한 강제·에러 매핑 검증.
//!
//! DB 없이 라우터를 직접 구동(`tower::ServiceExt::oneshot`). 권한 인가(D17) 경로가 핵심 대상:
//! 멤버십/소유자 단축/역할/채널 오버라이드/권한상승 방지/히스토리 게이팅.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use auth::{PowKeys, TokenKeys};
use axum::Router;
use rest_api::ratelimit::{RateLimiter, RateRule};
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use domain::channel::{Channel, NewChannel};
use domain::dm::{DmChannel, NewDm, NewGroupDm};
use domain::guild::{Guild, NewGuild};
use domain::id::{
    AttachmentId, ChannelId, MessageId, RealmId, RefreshTokenId, RoleId, Snowflake,
    SnowflakeGenerator, UserId, WebhookId,
};
use domain::invite::{Invite, NewInvite};
use domain::member::Member;
use domain::message::{Message, NewMessage};
use domain::permissions::{ChannelOverwrite, Permissions};
use domain::read_state::ReadState;
use domain::refresh_token::{NewRefreshToken, RefreshToken};
use domain::relationship::{RelationKind, Relationship};
use domain::attachment::{Attachment, NewAttachment};
use domain::audit::{AuditEntry, NewAuditEntry};
use domain::blob::{BlobError, BlobStore};
use domain::event::{RealmEventKind, RealmEventRecord};
use domain::repo::{
    AttachmentRepository, AuditRepository, ChannelOverwriteRepository, ChannelRepository,
    DmRepository, EventLogRepository, GuildRepository, InviteRepository, MessageRepository,
    ReactionRepository, ReadStateRepository, RefreshTokenRepository, RelationshipRepository,
    RepoError, RoleRepository, ThreadRepository, UserRepository, WebAuthnRepository,
    WebhookRepository,
};
use domain::role::{NewRole, Role};
use domain::thread::{NewThread, Thread};
use domain::webauthn::{NewWebAuthnCredential, WebAuthnCredential};
use domain::webhook::{NewWebhook, Webhook};
use domain::user::{NewUser, User};
use node::clock::{Clock, ManualClock};
use rest_api::AppState;
use serde_json::{Value, json};
use tower::ServiceExt;

// ----- in-memory Store -----

#[derive(Default)]
struct Inner {
    users: HashMap<u64, User>,
    refresh: Vec<(RefreshToken, Vec<u8>, i64, bool)>, // (token, hash, expires_unix, revoked)
    guilds: HashMap<u64, Guild>,
    members: HashMap<(u64, u64), (Option<String>, i64)>, // (realm,user) → (nick, joined_at)
    roles: HashMap<u64, Role>,
    member_roles: HashSet<(u64, u64, u64)>, // (realm, user, role)
    overwrites: HashMap<(u64, u64), ChannelOverwrite>,
    invites: HashMap<String, Invite>,
    dm_pairs: HashMap<(u64, u64), u64>, // (user_lo, user_hi) → realm_id
    realm_meta: HashMap<u64, (domain::dm::RealmKind, Option<u64>, Option<String>)>, // realm → (kind, owner, name)
    relationships: HashMap<(u64, u64), RelationKind>, // (user, target) → kind (방향성 행)
    channels: HashMap<u64, Channel>,
    messages: Vec<Message>,
    deleted_messages: HashSet<u64>,             // 소프트 삭제된 message id
    reactions: HashSet<(u64, u64, String)>,     // (message, user, emoji)
    mentions: HashSet<(u64, u64)>,              // (message, user)
    read_states: HashMap<(u64, u64), (Option<u64>, i32)>, // (user, channel) → (last_read, mention_count)
    totp: HashMap<u64, Vec<u8>>,                 // user → TOTP secret (D19)
    threads: HashMap<u64, (u64, Option<u64>, bool, i32)>, // channel → (parent, owner, archived, auto_archive)
    attachments: Vec<Attachment>,
    webhooks: Vec<Webhook>,
    audit: Vec<AuditEntry>,
    webauthn: Vec<(u64, Vec<u8>, String)>, // (user_id, credential_id, passkey_json) (D19)
    events: Vec<(u64, u64, RealmEventKind)>, // (realm_id, seq, kind) — 이벤트 소싱 로그 (D48)
    crdt: HashMap<(u64, String), (Option<String>, u64, u64)>, // (user,key) → (value, ts, node) — CRDT 동기화 (D49)
}

#[derive(Default, Clone)]
struct MemStore {
    inner: Arc<Mutex<Inner>>,
}

impl MemStore {
    fn seed_user(&self, id: u64, name: &str) {
        self.inner.lock().unwrap().users.insert(
            id,
            User {
                id: UserId(Snowflake::from_raw(id)),
                username: name.into(),
                global_name: None,
                email: format!("{name}@e.com"),
                password_hash: "h".into(),
                is_bot: false,
            },
        );
    }
}

impl UserRepository for MemStore {
    async fn create_user(&self, u: &NewUser) -> Result<(), RepoError> {
        let mut g = self.inner.lock().unwrap();
        if g.users.values().any(|x| x.username == u.username) {
            return Err(RepoError::Conflict);
        }
        g.users.insert(
            u.id.0.raw(),
            User {
                id: u.id,
                username: u.username.clone(),
                global_name: None,
                email: u.email.clone(),
                password_hash: u.password_hash.clone(),
                is_bot: false,
            },
        );
        Ok(())
    }
    async fn find_by_username(&self, name: &str) -> Result<Option<User>, RepoError> {
        Ok(self.inner.lock().unwrap().users.values().find(|u| u.username == name).cloned())
    }
    async fn find_by_id(&self, id: UserId) -> Result<Option<User>, RepoError> {
        Ok(self.inner.lock().unwrap().users.get(&id.0.raw()).cloned())
    }
    async fn set_totp_secret(&self, id: UserId, secret: Option<&[u8]>) -> Result<(), RepoError> {
        let mut g = self.inner.lock().unwrap();
        match secret {
            Some(s) => {
                g.totp.insert(id.0.raw(), s.to_vec());
            }
            None => {
                g.totp.remove(&id.0.raw());
            }
        }
        Ok(())
    }
    async fn totp_secret(&self, id: UserId) -> Result<Option<Vec<u8>>, RepoError> {
        Ok(self.inner.lock().unwrap().totp.get(&id.0.raw()).cloned())
    }
}

impl RefreshTokenRepository for MemStore {
    async fn create_refresh_token(&self, t: &NewRefreshToken) -> Result<(), RepoError> {
        self.inner.lock().unwrap().refresh.push((
            RefreshToken { id: t.id, user_id: t.user_id },
            t.token_hash.clone(),
            t.expires_at_unix,
            false,
        ));
        Ok(())
    }
    async fn find_active(&self, hash: &[u8], now: i64) -> Result<Option<RefreshToken>, RepoError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .refresh
            .iter()
            .find(|(_, h, exp, rev)| h == hash && !rev && *exp > now)
            .map(|(t, ..)| t.clone()))
    }
    async fn find_by_hash(&self, hash: &[u8]) -> Result<Option<RefreshToken>, RepoError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .refresh
            .iter()
            .find(|(_, h, ..)| h == hash)
            .map(|(t, ..)| t.clone()))
    }
    async fn revoke(&self, id: RefreshTokenId, _now: i64) -> Result<(), RepoError> {
        let mut g = self.inner.lock().unwrap();
        for (t, _, _, rev) in g.refresh.iter_mut() {
            if t.id == id {
                *rev = true;
            }
        }
        Ok(())
    }
    async fn revoke_all_for_user(&self, user: UserId, _now: i64) -> Result<(), RepoError> {
        let mut g = self.inner.lock().unwrap();
        for (t, _, _, rev) in g.refresh.iter_mut() {
            if t.user_id == user {
                *rev = true;
            }
        }
        Ok(())
    }
}

impl GuildRepository for MemStore {
    async fn create_guild(&self, gd: &NewGuild) -> Result<(), RepoError> {
        let mut g = self.inner.lock().unwrap();
        let realm = gd.realm_id.0.raw();
        if g.guilds.contains_key(&realm) {
            return Err(RepoError::Conflict);
        }
        g.guilds.insert(
            realm,
            Guild { realm_id: gd.realm_id, name: gd.name.clone(), owner_id: gd.owner_id },
        );
        // @everyone 역할 (id == realm).
        g.roles.insert(
            realm,
            Role {
                id: RoleId(Snowflake::from_raw(realm)),
                realm_id: gd.realm_id,
                name: "@everyone".into(),
                permissions: Permissions::default_everyone(),
                position: 0,
            },
        );
        g.members.insert((realm, gd.owner_id.0.raw()), (None, 0));
        g.realm_meta.insert(realm, (domain::dm::RealmKind::Guild, Some(gd.owner_id.0.raw()), Some(gd.name.clone())));
        Ok(())
    }
    async fn get_guild(&self, realm: RealmId) -> Result<Option<Guild>, RepoError> {
        Ok(self.inner.lock().unwrap().guilds.get(&realm.0.raw()).cloned())
    }
    async fn add_member(&self, realm: RealmId, user: UserId) -> Result<(), RepoError> {
        let mut g = self.inner.lock().unwrap();
        let joined = g.members.len() as i64; // 단조 증가 stand-in (joined 순서).
        g.members.entry((realm.0.raw(), user.0.raw())).or_insert((None, joined));
        Ok(())
    }
    async fn is_member(&self, realm: RealmId, user: UserId) -> Result<bool, RepoError> {
        Ok(self.inner.lock().unwrap().members.contains_key(&(realm.0.raw(), user.0.raw())))
    }
    async fn get_member(&self, realm: RealmId, user: UserId) -> Result<Option<Member>, RepoError> {
        let g = self.inner.lock().unwrap();
        Ok(g.members.get(&(realm.0.raw(), user.0.raw())).map(|(nick, joined)| Member {
            realm_id: realm,
            user_id: user,
            nick: nick.clone(),
            joined_at: *joined,
            roles: g
                .member_roles
                .iter()
                .filter(|(r, u, role)| *r == realm.0.raw() && *u == user.0.raw() && *role != realm.0.raw())
                .map(|(_, _, role)| RoleId(Snowflake::from_raw(*role)))
                .collect(),
        }))
    }
    async fn list_members(&self, realm: RealmId) -> Result<Vec<Member>, RepoError> {
        let g = self.inner.lock().unwrap();
        let mut out: Vec<Member> = g
            .members
            .iter()
            .filter(|((r, _), _)| *r == realm.0.raw())
            .map(|((_, u), (nick, joined))| Member {
                realm_id: realm,
                user_id: UserId(Snowflake::from_raw(*u)),
                nick: nick.clone(),
                joined_at: *joined,
                roles: g
                    .member_roles
                    .iter()
                    .filter(|(rr, uu, role)| *rr == realm.0.raw() && *uu == *u && *role != realm.0.raw())
                    .map(|(_, _, role)| RoleId(Snowflake::from_raw(*role)))
                    .collect(),
            })
            .collect();
        out.sort_by_key(|m| m.joined_at);
        Ok(out)
    }
    async fn update_member_nick(
        &self,
        realm: RealmId,
        user: UserId,
        nick: Option<&str>,
    ) -> Result<bool, RepoError> {
        let mut g = self.inner.lock().unwrap();
        match g.members.get_mut(&(realm.0.raw(), user.0.raw())) {
            Some(entry) => {
                entry.0 = nick.map(|s| s.to_owned());
                Ok(true)
            }
            None => Ok(false),
        }
    }
    async fn remove_member(&self, realm: RealmId, user: UserId) -> Result<bool, RepoError> {
        let mut g = self.inner.lock().unwrap();
        let existed = g.members.remove(&(realm.0.raw(), user.0.raw())).is_some();
        g.member_roles.retain(|(r, u, _)| !(*r == realm.0.raw() && *u == user.0.raw()));
        Ok(existed)
    }
    async fn member_realm_ids(&self, user: UserId) -> Result<Vec<RealmId>, RepoError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .members
            .keys()
            .filter(|(_, u)| *u == user.0.raw())
            .map(|(r, _)| RealmId(Snowflake::from_raw(*r)))
            .collect())
    }
}

impl RoleRepository for MemStore {
    async fn create_role(&self, r: &NewRole) -> Result<(), RepoError> {
        let mut g = self.inner.lock().unwrap();
        if g.roles.contains_key(&r.id.0.raw()) {
            return Err(RepoError::Conflict);
        }
        g.roles.insert(
            r.id.0.raw(),
            Role { id: r.id, realm_id: r.realm_id, name: r.name.clone(), permissions: r.permissions, position: 0 },
        );
        Ok(())
    }
    async fn list_roles(&self, realm: RealmId) -> Result<Vec<Role>, RepoError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .roles
            .values()
            .filter(|r| r.realm_id == realm)
            .cloned()
            .collect())
    }
    async fn assign_role(&self, realm: RealmId, user: UserId, role: RoleId) -> Result<(), RepoError> {
        self.inner.lock().unwrap().member_roles.insert((realm.0.raw(), user.0.raw(), role.0.raw()));
        Ok(())
    }
    async fn everyone_permissions(&self, realm: RealmId) -> Result<Option<u64>, RepoError> {
        Ok(self.inner.lock().unwrap().roles.get(&realm.0.raw()).map(|r| r.permissions.bits()))
    }
    async fn member_role_permissions(&self, realm: RealmId, user: UserId) -> Result<Vec<u64>, RepoError> {
        let g = self.inner.lock().unwrap();
        Ok(g.member_roles
            .iter()
            .filter(|(r, u, role)| *r == realm.0.raw() && *u == user.0.raw() && *role != realm.0.raw())
            .filter_map(|(_, _, role)| g.roles.get(role).map(|x| x.permissions.bits()))
            .collect())
    }
    async fn member_roles_with_ids(&self, realm: RealmId, user: UserId) -> Result<Vec<(u64, u64)>, RepoError> {
        let g = self.inner.lock().unwrap();
        Ok(g.member_roles
            .iter()
            .filter(|(r, u, role)| *r == realm.0.raw() && *u == user.0.raw() && *role != realm.0.raw())
            .filter_map(|(_, _, role)| g.roles.get(role).map(|x| (*role, x.permissions.bits())))
            .collect())
    }
}

impl ChannelOverwriteRepository for MemStore {
    async fn set_overwrite(&self, ow: &ChannelOverwrite) -> Result<(), RepoError> {
        self.inner
            .lock()
            .unwrap()
            .overwrites
            .insert((ow.channel_id.0.raw(), ow.target_id), ow.clone());
        Ok(())
    }
    async fn list_overwrites(&self, channel: ChannelId) -> Result<Vec<ChannelOverwrite>, RepoError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .overwrites
            .values()
            .filter(|o| o.channel_id == channel)
            .cloned()
            .collect())
    }
}

impl InviteRepository for MemStore {
    async fn create_invite(&self, inv: &NewInvite) -> Result<(), RepoError> {
        let mut g = self.inner.lock().unwrap();
        if g.invites.contains_key(&inv.code) {
            return Err(RepoError::Conflict);
        }
        g.invites.insert(
            inv.code.clone(),
            Invite {
                code: inv.code.clone(),
                realm_id: inv.realm_id,
                channel_id: inv.channel_id,
                inviter_id: inv.inviter_id,
                uses: 0,
                max_uses: inv.max_uses,
                expires_at: inv.expires_at,
            },
        );
        Ok(())
    }
    async fn find_invite(&self, code: &str) -> Result<Option<Invite>, RepoError> {
        Ok(self.inner.lock().unwrap().invites.get(code).cloned())
    }
    async fn redeem_invite(&self, code: &str, user: UserId, now: i64) -> Result<Option<RealmId>, RepoError> {
        let mut g = self.inner.lock().unwrap();
        let Some(inv) = g.invites.get(code).cloned() else { return Ok(None) };
        if !inv.is_valid(now) {
            return Ok(None);
        }
        let realm = inv.realm_id;
        let joined = g.members.len() as i64;
        g.members.entry((realm.0.raw(), user.0.raw())).or_insert((None, joined));
        if let Some(i) = g.invites.get_mut(code) {
            i.uses += 1;
        }
        Ok(Some(realm))
    }
}

impl DmRepository for MemStore {
    async fn find_dm(&self, a: UserId, b: UserId) -> Result<Option<DmChannel>, RepoError> {
        let (lo, hi) = domain::dm::order_pair(a, b);
        let g = self.inner.lock().unwrap();
        let Some(&realm) = g.dm_pairs.get(&(lo.0.raw(), hi.0.raw())) else { return Ok(None) };
        let chan = g.channels.values().find(|c| c.realm_id.0.raw() == realm).map(|c| c.id);
        Ok(chan.map(|channel_id| DmChannel {
            realm_id: RealmId(Snowflake::from_raw(realm)),
            channel_id,
            kind: domain::dm::RealmKind::Dm,
        }))
    }
    async fn create_dm(&self, dm: &NewDm) -> Result<(), RepoError> {
        let (lo, hi) = domain::dm::order_pair(dm.user_a, dm.user_b);
        let mut g = self.inner.lock().unwrap();
        if g.dm_pairs.contains_key(&(lo.0.raw(), hi.0.raw())) {
            return Err(RepoError::Conflict);
        }
        let realm = dm.realm_id.0.raw();
        g.realm_meta.insert(realm, (domain::dm::RealmKind::Dm, None, None));
        g.channels.insert(
            dm.channel_id.0.raw(),
            Channel { id: dm.channel_id, realm_id: dm.realm_id, kind: domain::channel::ChannelKind::Dm, name: None, position: 0 },
        );
        let joined = g.members.len() as i64;
        g.members.insert((realm, dm.user_a.0.raw()), (None, joined));
        g.members.insert((realm, dm.user_b.0.raw()), (None, joined + 1));
        g.dm_pairs.insert((lo.0.raw(), hi.0.raw()), realm);
        Ok(())
    }
    async fn create_group_dm(&self, dm: &NewGroupDm) -> Result<(), RepoError> {
        let mut g = self.inner.lock().unwrap();
        let realm = dm.realm_id.0.raw();
        g.realm_meta.insert(realm, (domain::dm::RealmKind::GroupDm, Some(dm.owner.0.raw()), dm.name.clone()));
        g.channels.insert(
            dm.channel_id.0.raw(),
            Channel { id: dm.channel_id, realm_id: dm.realm_id, kind: domain::channel::ChannelKind::Dm, name: dm.name.clone(), position: 0 },
        );
        for (i, m) in dm.members.iter().enumerate() {
            let joined = g.members.len() as i64 + i as i64;
            g.members.entry((realm, m.0.raw())).or_insert((None, joined));
        }
        Ok(())
    }
    async fn get_realm(&self, realm_id: RealmId) -> Result<Option<domain::dm::RealmInfo>, RepoError> {
        Ok(self.inner.lock().unwrap().realm_meta.get(&realm_id.0.raw()).map(|(kind, owner, name)| {
            domain::dm::RealmInfo {
                id: realm_id,
                kind: *kind,
                owner_id: owner.map(|o| UserId(Snowflake::from_raw(o))),
                name: name.clone(),
            }
        }))
    }
}

impl RelationshipRepository for MemStore {
    async fn list_relationships(&self, user: UserId) -> Result<Vec<Relationship>, RepoError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .relationships
            .iter()
            .filter(|((u, _), _)| *u == user.0.raw())
            .map(|((_, t), kind)| Relationship {
                user_id: user,
                target_id: UserId(Snowflake::from_raw(*t)),
                kind: *kind,
            })
            .collect())
    }
    async fn get_relationship(&self, user: UserId, target: UserId) -> Result<Option<RelationKind>, RepoError> {
        Ok(self.inner.lock().unwrap().relationships.get(&(user.0.raw(), target.0.raw())).copied())
    }
    async fn is_blocked_between(&self, a: UserId, b: UserId) -> Result<bool, RepoError> {
        let g = self.inner.lock().unwrap();
        Ok(g.relationships.get(&(a.0.raw(), b.0.raw())) == Some(&RelationKind::Blocked)
            || g.relationships.get(&(b.0.raw(), a.0.raw())) == Some(&RelationKind::Blocked))
    }
    async fn friend_request_or_accept(&self, me: UserId, target: UserId) -> Result<RelationKind, RepoError> {
        let mut g = self.inner.lock().unwrap();
        let (m, t) = (me.0.raw(), target.0.raw());
        let result = match g.relationships.get(&(m, t)).copied() {
            Some(RelationKind::Friend) => RelationKind::Friend,
            Some(RelationKind::PendingOut) => RelationKind::PendingOut,
            Some(RelationKind::PendingIn) => {
                g.relationships.insert((m, t), RelationKind::Friend);
                g.relationships.insert((t, m), RelationKind::Friend);
                RelationKind::Friend
            }
            Some(RelationKind::Blocked) => RelationKind::Blocked,
            None => {
                g.relationships.insert((m, t), RelationKind::PendingOut);
                g.relationships.insert((t, m), RelationKind::PendingIn);
                RelationKind::PendingOut
            }
        };
        Ok(result)
    }
    async fn block(&self, me: UserId, target: UserId) -> Result<(), RepoError> {
        let mut g = self.inner.lock().unwrap();
        g.relationships.insert((me.0.raw(), target.0.raw()), RelationKind::Blocked);
        g.relationships.remove(&(target.0.raw(), me.0.raw()));
        Ok(())
    }
    async fn remove_relationship(&self, me: UserId, target: UserId) -> Result<Option<RelationKind>, RepoError> {
        let mut g = self.inner.lock().unwrap();
        let (m, t) = (me.0.raw(), target.0.raw());
        let mine = g.relationships.remove(&(m, t));
        if let Some(k) = mine
            && k != RelationKind::Blocked
        {
            g.relationships.remove(&(t, m));
        }
        Ok(mine)
    }
}

impl ChannelRepository for MemStore {
    async fn create_channel(&self, c: &NewChannel) -> Result<(), RepoError> {
        self.inner.lock().unwrap().channels.insert(
            c.id.0.raw(),
            Channel { id: c.id, realm_id: c.realm_id, kind: c.kind, name: Some(c.name.clone()), position: 0 },
        );
        Ok(())
    }
    async fn get(&self, id: ChannelId) -> Result<Option<Channel>, RepoError> {
        Ok(self.inner.lock().unwrap().channels.get(&id.0.raw()).cloned())
    }
    async fn list_by_realm(&self, realm: RealmId) -> Result<Vec<Channel>, RepoError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .channels
            .values()
            .filter(|c| c.realm_id == realm)
            .cloned()
            .collect())
    }
}

impl MessageRepository for MemStore {
    async fn create_message(&self, m: &NewMessage) -> Result<bool, RepoError> {
        let mut g = self.inner.lock().unwrap();
        if let Some(n) = &m.nonce
            && g.messages.iter().any(|x| x.channel_id == m.channel_id && x.nonce.as_deref() == Some(n)) {
                return Ok(false);
            }
        g.messages.push(Message {
            id: m.id,
            channel_id: m.channel_id,
            realm_id: m.realm_id,
            author_id: m.author_id,
            content: m.content.clone(),
            nonce: m.nonce.clone(),
            reference_message_id: m.reference_message_id,
        });
        Ok(true)
    }
    async fn list_by_channel(
        &self,
        channel: ChannelId,
        before: Option<MessageId>,
        limit: i64,
    ) -> Result<Vec<Message>, RepoError> {
        let g = self.inner.lock().unwrap();
        let mut v: Vec<Message> = g
            .messages
            .iter()
            .filter(|m| m.channel_id == channel)
            .filter(|m| !g.deleted_messages.contains(&m.id.0.raw()))
            .filter(|m| before.map(|b| m.id.0.raw() < b.0.raw()).unwrap_or(true))
            .cloned()
            .collect();
        v.sort_by_key(|m| std::cmp::Reverse(m.id.0.raw())); // 최신순
        v.truncate(limit.max(0) as usize);
        Ok(v)
    }
    async fn get_message(&self, id: MessageId) -> Result<Option<Message>, RepoError> {
        let g = self.inner.lock().unwrap();
        if g.deleted_messages.contains(&id.0.raw()) {
            return Ok(None);
        }
        Ok(g.messages.iter().find(|m| m.id == id).cloned())
    }
    async fn edit_message(&self, id: MessageId, author: UserId, content: &str) -> Result<bool, RepoError> {
        let mut g = self.inner.lock().unwrap();
        if g.deleted_messages.contains(&id.0.raw()) {
            return Ok(false);
        }
        match g.messages.iter_mut().find(|m| m.id == id && m.author_id == author) {
            Some(m) => {
                m.content = content.to_owned();
                Ok(true)
            }
            None => Ok(false),
        }
    }
    async fn soft_delete_message(&self, id: MessageId) -> Result<bool, RepoError> {
        let mut g = self.inner.lock().unwrap();
        if !g.messages.iter().any(|m| m.id == id) || g.deleted_messages.contains(&id.0.raw()) {
            return Ok(false);
        }
        Ok(g.deleted_messages.insert(id.0.raw()))
    }
    async fn add_mentions(&self, message_id: MessageId, users: &[UserId]) -> Result<(), RepoError> {
        let mut g = self.inner.lock().unwrap();
        for u in users {
            // 존재하는 유저만 (FK 모사).
            if g.users.contains_key(&u.0.raw()) {
                g.mentions.insert((message_id.0.raw(), u.0.raw()));
            }
        }
        Ok(())
    }
    async fn search_messages(
        &self,
        realm: RealmId,
        channels: &[ChannelId],
        query: &str,
        limit: i64,
    ) -> Result<Vec<Message>, RepoError> {
        if channels.is_empty() {
            return Ok(Vec::new());
        }
        let chans: HashSet<u64> = channels.iter().map(|c| c.0.raw()).collect();
        let q = query.to_lowercase();
        let g = self.inner.lock().unwrap();
        let mut v: Vec<Message> = g
            .messages
            .iter()
            .filter(|m| m.realm_id == realm && chans.contains(&m.channel_id.0.raw()))
            .filter(|m| !g.deleted_messages.contains(&m.id.0.raw()))
            // FTS 대용: 토큰 단위 substring 매칭(라우트·권한 필터 검증이 목적, 실제 FTS는 storage 테스트).
            .filter(|m| m.content.to_lowercase().split_whitespace().any(|w| w.contains(&q)))
            .cloned()
            .collect();
        v.sort_by_key(|m| std::cmp::Reverse(m.id.0.raw()));
        v.truncate(limit.max(0) as usize);
        Ok(v)
    }
}

impl ThreadRepository for MemStore {
    async fn create_thread(&self, t: &NewThread) -> Result<(), RepoError> {
        let mut g = self.inner.lock().unwrap();
        g.channels.insert(
            t.id.0.raw(),
            Channel {
                id: t.id,
                realm_id: t.realm_id,
                kind: domain::channel::ChannelKind::Thread,
                name: Some(t.name.clone()),
                position: 0,
            },
        );
        g.threads.insert(t.id.0.raw(), (t.parent_id.0.raw(), Some(t.owner.0.raw()), false, t.auto_archive));
        Ok(())
    }
    async fn get_thread(&self, channel: ChannelId) -> Result<Option<Thread>, RepoError> {
        let g = self.inner.lock().unwrap();
        Ok(g.threads.get(&channel.0.raw()).map(|&(parent, owner, archived, auto)| {
            let ch = g.channels.get(&channel.0.raw());
            let mc = g.messages.iter().filter(|m| m.channel_id == channel && !g.deleted_messages.contains(&m.id.0.raw())).count() as i64;
            Thread {
                id: channel,
                realm_id: ch.map(|c| c.realm_id).unwrap_or(RealmId(Snowflake::from_raw(0))),
                parent_id: ChannelId(Snowflake::from_raw(parent)),
                name: ch.and_then(|c| c.name.clone()),
                owner_id: owner.map(|o| UserId(Snowflake::from_raw(o))),
                archived,
                auto_archive: auto,
                message_count: mc,
            }
        }))
    }
    async fn list_threads(&self, parent: ChannelId) -> Result<Vec<Thread>, RepoError> {
        let ids: Vec<u64> = {
            let g = self.inner.lock().unwrap();
            let mut v: Vec<u64> = g.threads.iter().filter(|(_, t)| t.0 == parent.0.raw()).map(|(c, _)| *c).collect();
            v.sort_by_key(|c| std::cmp::Reverse(*c));
            v
        };
        let mut out = Vec::new();
        for id in ids {
            if let Some(t) = self.get_thread(ChannelId(Snowflake::from_raw(id))).await? {
                out.push(t);
            }
        }
        Ok(out)
    }
    async fn set_thread_archived(&self, channel: ChannelId, archived: bool) -> Result<bool, RepoError> {
        let mut g = self.inner.lock().unwrap();
        match g.threads.get_mut(&channel.0.raw()) {
            Some(t) => {
                t.2 = archived;
                Ok(true)
            }
            None => Ok(false),
        }
    }
}

impl ReadStateRepository for MemStore {
    async fn ack(&self, user: UserId, channel: ChannelId, message: MessageId) -> Result<ReadState, RepoError> {
        let mut g = self.inner.lock().unwrap();
        // message 이후의 살아있는 멘션 수 재계산.
        let mention_count = g
            .messages
            .iter()
            .filter(|m| m.channel_id == channel && m.id.0.raw() > message.0.raw())
            .filter(|m| !g.deleted_messages.contains(&m.id.0.raw()))
            .filter(|m| g.mentions.contains(&(m.id.0.raw(), user.0.raw())))
            .count() as i32;
        g.read_states.insert((user.0.raw(), channel.0.raw()), (Some(message.0.raw()), mention_count));
        Ok(ReadState { channel_id: channel, last_read_message_id: Some(message), mention_count })
    }
    async fn bump_mentions(&self, channel: ChannelId, users: &[UserId]) -> Result<(), RepoError> {
        let mut g = self.inner.lock().unwrap();
        for u in users {
            if !g.users.contains_key(&u.0.raw()) {
                continue; // 존재 유저만(FK 모사).
            }
            let e = g.read_states.entry((u.0.raw(), channel.0.raw())).or_insert((None, 0));
            e.1 += 1;
        }
        Ok(())
    }
    async fn list_read_states(&self, user: UserId) -> Result<Vec<ReadState>, RepoError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .read_states
            .iter()
            .filter(|((u, _), _)| *u == user.0.raw())
            .map(|((_, c), (lr, mc))| ReadState {
                channel_id: ChannelId(Snowflake::from_raw(*c)),
                last_read_message_id: lr.map(|m| MessageId(Snowflake::from_raw(m))),
                mention_count: *mc,
            })
            .collect())
    }
}

impl ReactionRepository for MemStore {
    async fn add_reaction(&self, message_id: MessageId, user: UserId, emoji: &str) -> Result<bool, RepoError> {
        Ok(self.inner.lock().unwrap().reactions.insert((message_id.0.raw(), user.0.raw(), emoji.to_owned())))
    }
    async fn remove_reaction(&self, message_id: MessageId, user: UserId, emoji: &str) -> Result<bool, RepoError> {
        Ok(self.inner.lock().unwrap().reactions.remove(&(message_id.0.raw(), user.0.raw(), emoji.to_owned())))
    }
}

impl AttachmentRepository for MemStore {
    async fn add_attachment(&self, a: &NewAttachment) -> Result<(), RepoError> {
        self.inner.lock().unwrap().attachments.push(Attachment {
            id: a.id,
            message_id: a.message_id,
            filename: a.filename.clone(),
            size_bytes: a.size_bytes,
            content_type: a.content_type.clone(),
            url: a.url.clone(),
        });
        Ok(())
    }
    async fn list_attachments(&self, message_id: MessageId) -> Result<Vec<Attachment>, RepoError> {
        Ok(self.inner.lock().unwrap().attachments.iter().filter(|a| a.message_id == message_id).cloned().collect())
    }
    async fn get_attachment(&self, id: AttachmentId) -> Result<Option<Attachment>, RepoError> {
        Ok(self.inner.lock().unwrap().attachments.iter().find(|a| a.id == id).cloned())
    }
}

impl WebhookRepository for MemStore {
    async fn create_webhook(&self, w: &NewWebhook) -> Result<(), RepoError> {
        self.inner.lock().unwrap().webhooks.push(Webhook {
            id: w.id,
            channel_id: w.channel_id,
            realm_id: w.realm_id,
            name: w.name.clone(),
            creator_id: Some(w.creator_id),
            token_hash: w.token_hash.clone(),
        });
        Ok(())
    }
    async fn list_webhooks(&self, channel_id: ChannelId) -> Result<Vec<Webhook>, RepoError> {
        Ok(self.inner.lock().unwrap().webhooks.iter().filter(|w| w.channel_id == channel_id).cloned().collect())
    }
    async fn get_webhook(&self, id: WebhookId) -> Result<Option<Webhook>, RepoError> {
        Ok(self.inner.lock().unwrap().webhooks.iter().find(|w| w.id == id).cloned())
    }
    async fn delete_webhook(&self, id: WebhookId) -> Result<bool, RepoError> {
        let mut g = self.inner.lock().unwrap();
        let before = g.webhooks.len();
        g.webhooks.retain(|w| w.id != id);
        Ok(g.webhooks.len() < before)
    }
}

impl AuditRepository for MemStore {
    async fn log_audit(&self, e: &NewAuditEntry) -> Result<(), RepoError> {
        self.inner.lock().unwrap().audit.push(AuditEntry {
            id: e.id,
            realm_id: e.realm_id,
            actor_id: Some(e.actor_id),
            action: e.action,
            target_id: e.target_id,
            changes: e.changes.clone(),
        });
        Ok(())
    }
    async fn list_audit(&self, realm: RealmId, before: Option<u64>, limit: i64) -> Result<Vec<AuditEntry>, RepoError> {
        let g = self.inner.lock().unwrap();
        let mut v: Vec<AuditEntry> = g
            .audit
            .iter()
            .filter(|e| e.realm_id == realm)
            .filter(|e| before.map(|b| e.id.raw() < b).unwrap_or(true))
            .cloned()
            .collect();
        v.sort_by_key(|e| std::cmp::Reverse(e.id.raw()));
        v.truncate(limit.max(0) as usize);
        Ok(v)
    }
}

impl WebAuthnRepository for MemStore {
    async fn add_credential(&self, c: &NewWebAuthnCredential) -> Result<(), RepoError> {
        let mut g = self.inner.lock().unwrap();
        if g.webauthn.iter().any(|(_, cid, _)| *cid == c.credential_id) {
            return Err(RepoError::Conflict);
        }
        g.webauthn.push((c.user_id.0.raw(), c.credential_id.clone(), c.passkey_json.clone()));
        Ok(())
    }
    async fn list_credentials(&self, user: UserId) -> Result<Vec<WebAuthnCredential>, RepoError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .webauthn
            .iter()
            .filter(|(u, ..)| *u == user.0.raw())
            .map(|(_, cid, pk)| WebAuthnCredential { credential_id: cid.clone(), passkey_json: pk.clone() })
            .collect())
    }
    async fn update_credential(&self, credential_id: &[u8], passkey_json: &str) -> Result<(), RepoError> {
        let mut g = self.inner.lock().unwrap();
        for (_, cid, pk) in g.webauthn.iter_mut() {
            if cid == credential_id {
                *pk = passkey_json.to_string();
            }
        }
        Ok(())
    }
}

impl domain::repo::CrdtRepository for MemStore {
    async fn load_user_doc(&self, user: UserId) -> Result<domain::crdt::LwwMap, RepoError> {
        let g = self.inner.lock().unwrap();
        let u = user.0.raw();
        let entries = g.crdt.iter().filter(|((uid, _), _)| *uid == u).map(|((_, k), (v, ts, n))| {
            domain::crdt::CrdtEntry { key: k.clone(), value: v.clone(), ts_ms: *ts, node: *n }
        });
        Ok(domain::crdt::LwwMap::from_entries(entries))
    }
    async fn merge_user_doc(
        &self,
        user: UserId,
        entries: &[domain::crdt::CrdtEntry],
    ) -> Result<domain::crdt::LwwMap, RepoError> {
        {
            let mut g = self.inner.lock().unwrap();
            let u = user.0.raw();
            for e in entries {
                let slot = g.crdt.entry((u, e.key.clone())).or_insert((None, 0, 0));
                // LWW 가드: (ts,node) 큰 것만 채택.
                if (e.ts_ms, e.node) > (slot.1, slot.2) {
                    *slot = (e.value.clone(), e.ts_ms, e.node);
                }
            }
        }
        self.load_user_doc(user).await
    }
}

impl EventLogRepository for MemStore {
    async fn append_event(&self, realm: RealmId, kind: &RealmEventKind) -> Result<u64, RepoError> {
        let mut g = self.inner.lock().unwrap();
        let r = realm.0.raw();
        let seq = g.events.iter().filter(|(rid, ..)| *rid == r).count() as u64 + 1;
        g.events.push((r, seq, kind.clone()));
        Ok(seq)
    }
    async fn replay_events(
        &self,
        realm: RealmId,
        after_seq: u64,
    ) -> Result<Vec<RealmEventRecord>, RepoError> {
        let r = realm.0.raw();
        Ok(self
            .inner
            .lock()
            .unwrap()
            .events
            .iter()
            .filter(|(rid, seq, _)| *rid == r && *seq > after_seq)
            .map(|(_, seq, kind)| RealmEventRecord { realm_id: realm, seq: *seq, kind: kind.clone() })
            .collect())
    }
}

/// 인메모리 BlobStore (테스트용) — key→bytes.
#[derive(Default, Clone)]
struct MemBlobStore {
    blobs: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl BlobStore for MemBlobStore {
    fn put(&self, key: &str, bytes: Vec<u8>) -> domain::emit::BoxFuture<'_, Result<(), BlobError>> {
        self.blobs.lock().unwrap().insert(key.to_owned(), bytes);
        Box::pin(async { Ok(()) })
    }
    fn get(&self, key: &str) -> domain::emit::BoxFuture<'_, Result<Option<Vec<u8>>, BlobError>> {
        let v = self.blobs.lock().unwrap().get(key).cloned();
        Box::pin(async move { Ok(v) })
    }
}

// ----- 테스트 하네스 -----

/// 발생한 emit(realm, t, payload)을 기록하는 테스트 emitter (D39 팬아웃 검증용).
#[derive(Default)]
struct RecordingEmitter {
    events: Mutex<Vec<(u64, String, String)>>,
}

impl domain::emit::RealmEmitter for RecordingEmitter {
    fn emit(&self, realm: RealmId, t: String, payload: String) -> domain::emit::BoxFuture<'_, ()> {
        self.events.lock().unwrap().push((realm.0.raw(), t, payload));
        Box::pin(async {})
    }
}

/// 발생한 유저 emit(users, t)을 기록하는 테스트 emitter (친구·차단 통지 검증용).
#[derive(Default)]
struct RecordingUserEmitter {
    events: Mutex<Vec<(Vec<u64>, String)>>,
}

impl domain::emit::UserEmitter for RecordingUserEmitter {
    fn emit_to_users(&self, users: &[UserId], t: String, _payload: String) -> domain::emit::BoxFuture<'_, ()> {
        self.events.lock().unwrap().push((users.iter().map(|u| u.0.raw()).collect(), t));
        Box::pin(async {})
    }
}

struct Harness {
    router: Router,
    keys: Arc<TokenKeys>,
    pow: Arc<PowKeys>,
    store: Arc<MemStore>,
    snow: Arc<SnowflakeGenerator>,
    clock: Arc<ManualClock>,
    emitter: Arc<RecordingEmitter>,
    user_emitter: Arc<RecordingUserEmitter>,
    blobs: MemBlobStore,
}

impl Harness {
    fn new() -> Self {
        // 기존 테스트는 한도에 안 걸리게 lenient 리미터.
        Self::with_ratelimit(RateLimiter::lenient())
    }

    fn with_ratelimit(ratelimit: RateLimiter) -> Self {
        let keys = Arc::new(TokenKeys::generate().unwrap());
        let pow = Arc::new(PowKeys::generate().unwrap());
        let store = Arc::new(MemStore::default());
        let snow = Arc::new(SnowflakeGenerator::new(1));
        let clock = Arc::new(ManualClock::new(domain::id::EPOCH_MS + 1));
        let emitter = Arc::new(RecordingEmitter::default());
        let user_emitter = Arc::new(RecordingUserEmitter::default());
        let blobs = MemBlobStore::default();
        // WebAuthn(D19): 테스트는 항상 localhost RP로 활성 (다른 라우트엔 무영향).
        let webauthn = Some(Arc::new(
            auth::WebauthnService::new("localhost", "http://localhost:8080").unwrap(),
        ));
        let state = AppState::new(
            Arc::clone(&store),
            Arc::clone(&keys),
            Arc::clone(&pow),
            Arc::new(ratelimit),
            Arc::clone(&snow),
            clock.clone() as Arc<dyn Clock>,
            Arc::clone(&emitter) as Arc<dyn domain::emit::RealmEmitter>,
            Arc::clone(&user_emitter) as Arc<dyn domain::emit::UserEmitter>,
            Arc::new(blobs.clone()) as Arc<dyn domain::blob::BlobStore>,
            webauthn,
        );
        Self { router: rest_api::router(state), keys, pow, store, snow, clock, emitter, user_emitter, blobs }
    }

    /// 기록된 Realm emit 이벤트 이름 목록 (검증용).
    fn emitted(&self) -> Vec<String> {
        self.emitter.events.lock().unwrap().iter().map(|(_, t, _)| t.clone()).collect()
    }

    /// 기록된 유저 emit 이벤트 이름 목록 (친구·차단 검증용).
    fn user_emitted(&self) -> Vec<String> {
        self.user_emitter.events.lock().unwrap().iter().map(|(_, t)| t.clone()).collect()
    }

    fn token(&self, uid: u64) -> String {
        self.keys.issue_access(uid).unwrap()
    }

    /// 새 Snowflake user id 발급 + 시드.
    fn user(&self, name: &str) -> u64 {
        let id = self.snow.next(self.clock.now_ms()).raw();
        self.store.seed_user(id, name);
        id
    }

    async fn req(&self, method: &str, uri: &str, token: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
        let mut b = Request::builder().method(method).uri(uri);
        if let Some(t) = token {
            b = b.header("authorization", format!("Bearer {t}"));
        }
        let req = if let Some(j) = body {
            b.header("content-type", "application/json").body(Body::from(j.to_string())).unwrap()
        } else {
            b.body(Body::empty()).unwrap()
        };
        let resp = self.router.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let val = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, val)
    }

    /// 멀티파트 파일 업로드 (한 파일 필드 "file").
    async fn upload(&self, uri: &str, token: &str, filename: &str, ct: &str, data: &[u8]) -> (StatusCode, Value) {
        let boundary = "X-TEST-BOUNDARY-9z";
        let mut body = Vec::new();
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: {ct}\r\n\r\n").as_bytes(),
        );
        body.extend_from_slice(data);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let req = Request::builder()
            .method("POST")
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", format!("multipart/form-data; boundary={boundary}"))
            .body(Body::from(body))
            .unwrap();
        let resp = self.router.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    /// 원시 바이트 GET (다운로드 검증용).
    async fn get_bytes(&self, uri: &str, token: &str) -> (StatusCode, Vec<u8>) {
        let req = Request::builder()
            .method("GET")
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = self.router.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, bytes.to_vec())
    }
}

/// 길드 생성 → (guild_id, general_channel_id).
async fn make_guild(h: &Harness, owner_tok: &str) -> (String, String) {
    let (st, body) = h.req("POST", "/guilds", Some(owner_tok), Some(json!({"name": "G"}))).await;
    assert_eq!(st, StatusCode::CREATED, "guild create: {body}");
    let gid = body["id"].as_str().unwrap().to_string();
    let chan = body["channels"][0]["id"].as_str().unwrap().to_string();
    (gid, chan)
}

// ── 가입 봇방지 PoW (D18) ──────────────────────────────────────────────

#[tokio::test]
async fn pow_challenge_endpoint_issues_token() {
    let h = Harness::new();
    let (st, body) = h.req("GET", "/auth/pow-challenge", None, None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(body["difficulty"].as_u64().unwrap() as u8, auth::pow::DEFAULT_DIFFICULTY);
    assert!(body["challenge"].as_str().unwrap().starts_with("v4.local."));
}

#[tokio::test]
async fn register_with_valid_pow_succeeds() {
    let h = Harness::new();
    // 빠른 테스트를 위해 낮은 난이도 챌린지를 직접 발급(엔드포인트와 같은 키) 후 푼다.
    let challenge = h.pow.issue_challenge(8).unwrap();
    let nonce = auth::pow::solve(&challenge, 8);
    let (st, body) = h
        .req(
            "POST",
            "/auth/register",
            None,
            Some(json!({
                "username": "alice", "email": "a@x.io", "password": "password123",
                "pow_challenge": challenge, "pow_nonce": nonce,
            })),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED, "register: {body}");
    assert!(body["access_token"].as_str().is_some());
}

#[tokio::test]
async fn register_without_valid_pow_rejected() {
    let h = Harness::new();
    let (st, _) = h
        .req(
            "POST",
            "/auth/register",
            None,
            Some(json!({
                "username": "bob", "email": "b@x.io", "password": "password123",
                "pow_challenge": "v4.local.not-a-real-token", "pow_nonce": "0",
            })),
        )
        .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "가짜 PoW는 가입 거부");
}

#[tokio::test]
async fn rate_limit_returns_429_after_capacity() {
    // auth 버킷 용량 2 (clock 고정이라 리필 없음) → 3번째 요청은 429.
    let mut rules = HashMap::new();
    rules.insert("auth", RateRule { capacity: 2.0, refill_per_sec: 1.0 });
    let rl = RateLimiter::from_rules(rules, RateRule { capacity: 1e9, refill_per_sec: 1e9 });
    let h = Harness::with_ratelimit(rl);

    let (s1, _) = h.req("GET", "/auth/pow-challenge", None, None).await;
    let (s2, _) = h.req("GET", "/auth/pow-challenge", None, None).await;
    let (s3, _) = h.req("GET", "/auth/pow-challenge", None, None).await;
    assert_eq!(s1, StatusCode::OK);
    assert_eq!(s2, StatusCode::OK);
    assert_eq!(s3, StatusCode::TOO_MANY_REQUESTS, "용량 초과 → 429 (D32)");
}

// ── TOTP MFA (D19) ─────────────────────────────────────────────────────

#[tokio::test]
async fn mfa_enable_verify_then_login_requires_totp() {
    let h = Harness::new();
    let now = h.clock.now_ms() / 1000; // 고정 clock → enable/verify/login 코드 시각 동일.

    // 1) 가입(PoW)으로 실제 password_hash + 토큰 확보.
    let challenge = h.pow.issue_challenge(8).unwrap();
    let nonce = auth::pow::solve(&challenge, 8);
    let (st, body) = h
        .req(
            "POST",
            "/auth/register",
            None,
            Some(json!({
                "username": "mfauser", "email": "m@x.io", "password": "password123",
                "pow_challenge": challenge, "pow_nonce": nonce,
            })),
        )
        .await;
    assert_eq!(st, StatusCode::CREATED, "register: {body}");
    let token = body["access_token"].as_str().unwrap().to_string();

    // 2) enable → secret(hex) + otpauth URI (아직 미저장).
    let (st, en) = h.req("POST", "/auth/mfa/totp/enable", Some(&token), None).await;
    assert_eq!(st, StatusCode::OK);
    let secret_hex = en["secret"].as_str().unwrap().to_string();
    assert!(en["otpauth_uri"].as_str().unwrap().starts_with("otpauth://totp/"));

    // 3) 인증 앱 대역으로 코드 생성 → verify(저장=활성).
    let secret = auth::totp::decode_hex(&secret_hex).unwrap();
    let code = auth::totp::generate(&secret, now).unwrap();
    let (st, _) = h
        .req("POST", "/auth/mfa/totp/verify", Some(&token), Some(json!({ "secret": secret_hex, "code": code })))
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT, "verify activates MFA");

    // 4) 이제 비번 로그인은 토큰 대신 mfa_required.
    let (st, lg) = h.req("POST", "/auth/login", None, Some(json!({ "username": "mfauser", "password": "password123" }))).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(lg["mfa_required"], json!(true), "MFA 활성 → 비번만으론 토큰 없음");
    assert!(lg["access_token"].is_null());

    // 5) 2단계: 비번 + TOTP 코드 → 토큰 발급.
    let code2 = auth::totp::generate(&secret, now).unwrap();
    let (st, tk) = h
        .req("POST", "/auth/mfa/totp", None, Some(json!({ "username": "mfauser", "password": "password123", "code": code2 })))
        .await;
    assert_eq!(st, StatusCode::OK, "2단계 통과: {tk}");
    assert!(tk["access_token"].as_str().is_some());

    // 6) 틀린 코드는 거부.
    let (st, _) = h
        .req("POST", "/auth/mfa/totp", None, Some(json!({ "username": "mfauser", "password": "password123", "code": "000000" })))
        .await;
    assert_eq!(st, StatusCode::UNAUTHORIZED, "틀린 TOTP 거부");
}

#[tokio::test]
async fn missing_token_is_unauthorized() {
    let h = Harness::new();
    let (st, _) = h.req("POST", "/guilds", None, Some(json!({"name": "G"}))).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn guild_create_yields_default_channel_and_owner_membership() {
    let h = Harness::new();
    let owner = h.user("owner");
    let tok = h.token(owner);
    let (gid, chan) = make_guild(&h, &tok).await;
    assert!(!gid.is_empty() && !chan.is_empty());
    // 소유자는 멤버 → 자기 길드 채널 목록 조회 가능(roles 라우트로 간접 확인).
    let (st, body) = h.req("GET", &format!("/guilds/{gid}/roles"), Some(&tok), None).await;
    assert_eq!(st, StatusCode::OK);
    // @everyone 역할이 존재.
    assert!(body.as_array().unwrap().iter().any(|r| r["name"] == "@everyone"));
}

#[tokio::test]
async fn channel_create_requires_manage_channels() {
    let h = Harness::new();
    let owner = h.token(h.user("owner"));
    let (gid, _) = make_guild(&h, &owner).await;

    // 비멤버 외부인 → 403.
    let outsider = h.token(h.user("outsider"));
    let (st, _) = h.req("POST", &format!("/guilds/{gid}/channels"), Some(&outsider), Some(json!({"name": "x"}))).await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // 소유자 → 201 (owner 단축).
    let (st, _) = h.req("POST", &format!("/guilds/{gid}/channels"), Some(&owner), Some(json!({"name": "x"}))).await;
    assert_eq!(st, StatusCode::CREATED);
}

#[tokio::test]
async fn invite_redeem_makes_member_who_can_then_send_via_perms() {
    let h = Harness::new();
    let owner = h.token(h.user("owner"));
    let (gid, _) = make_guild(&h, &owner).await;

    // 초대 발급.
    let (st, inv) = h.req("POST", &format!("/guilds/{gid}/invites"), Some(&owner), Some(json!({}))).await;
    assert_eq!(st, StatusCode::CREATED);
    let code = inv["code"].as_str().unwrap().to_string();

    // 외부인 → 멤버 아님(채널 생성 시도 403).
    let bob_id = h.user("bob");
    let bob = h.token(bob_id);
    let (st, _) = h.req("POST", &format!("/guilds/{gid}/channels"), Some(&bob), Some(json!({"name": "y"}))).await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // redeem → 멤버 됨.
    let (st, joined) = h.req("POST", &format!("/invites/{code}"), Some(&bob), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(joined["realm_id"], gid);

    // 미존재 코드 → 404.
    let (st, _) = h.req("POST", "/invites/NONEXIST", Some(&bob), None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn role_creation_blocks_privilege_escalation() {
    let h = Harness::new();
    let owner = h.token(h.user("owner"));
    let (gid, _) = make_guild(&h, &owner).await;

    // 외부인 → MANAGE_ROLES 없음 → 역할 생성 403.
    let mallory = h.token(h.user("mallory"));
    let (st, _) = h.req("POST", &format!("/guilds/{gid}/roles"), Some(&mallory), Some(json!({"name":"r","permissions":0}))).await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // 소유자는 ADMINISTRATOR(0x8) 역할도 생성 가능 (owner 단축).
    let admin_bits = Permissions::ADMINISTRATOR.bits();
    let (st, _) = h.req("POST", &format!("/guilds/{gid}/roles"), Some(&owner), Some(json!({"name":"admin","permissions": admin_bits}))).await;
    assert_eq!(st, StatusCode::CREATED);
}

#[tokio::test]
async fn assigning_role_grants_channel_management() {
    let h = Harness::new();
    let owner = h.token(h.user("owner"));
    let (gid, _) = make_guild(&h, &owner).await;
    let bob_id = h.user("bob");
    let bob = h.token(bob_id);
    h.store.add_member(RealmId(Snowflake::from_raw(gid.parse().unwrap())), UserId(Snowflake::from_raw(bob_id))).await.unwrap();

    // bob(@everyone만) → 채널 생성 403.
    let (st, _) = h.req("POST", &format!("/guilds/{gid}/channels"), Some(&bob), Some(json!({"name":"a"}))).await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // owner가 MANAGE_CHANNELS 역할 생성 + bob에 부여.
    let mc = Permissions::MANAGE_CHANNELS.bits();
    let (_, role) = h.req("POST", &format!("/guilds/{gid}/roles"), Some(&owner), Some(json!({"name":"mod","permissions": mc}))).await;
    let rid = role["id"].as_str().unwrap();
    let (st, _) = h.req("PUT", &format!("/guilds/{gid}/members/{bob_id}/roles/{rid}"), Some(&owner), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    // 이제 bob → 채널 생성 201.
    let (st, _) = h.req("POST", &format!("/guilds/{gid}/channels"), Some(&bob), Some(json!({"name":"a"}))).await;
    assert_eq!(st, StatusCode::CREATED);
}

/// 회귀: 히스토리 조회가 채널 권한(VIEW_CHANNEL)으로 게이팅된다 (검증 패스에서 고친 버그).
#[tokio::test]
async fn history_read_is_gated_by_channel_view_permission() {
    let h = Harness::new();
    let owner_id = h.user("owner");
    let owner = h.token(owner_id);
    let (gid, chan) = make_guild(&h, &owner).await;
    let bob_id = h.user("bob");
    let bob = h.token(bob_id);
    h.store.add_member(RealmId(Snowflake::from_raw(gid.parse().unwrap())), UserId(Snowflake::from_raw(bob_id))).await.unwrap();

    // 기본: bob 히스토리 조회 200 (@everyone VIEW+READ_HISTORY).
    let (st, _) = h.req("GET", &format!("/channels/{chan}/messages"), Some(&bob), None).await;
    assert_eq!(st, StatusCode::OK);

    // @everyone VIEW_CHANNEL deny.
    let view = Permissions::VIEW_CHANNEL.bits();
    let (st, _) = h.req(
        "PUT",
        &format!("/channels/{chan}/permissions/{gid}"),
        Some(&owner),
        Some(json!({"type":"role","allow":0,"deny": view})),
    )
    .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    // bob → 403, owner(단축) → 200.
    let (st, _) = h.req("GET", &format!("/channels/{chan}/messages"), Some(&bob), None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    let (st, _) = h.req("GET", &format!("/channels/{chan}/messages"), Some(&owner), None).await;
    assert_eq!(st, StatusCode::OK);
}

#[tokio::test]
async fn member_list_self_nick_and_leave_emit_events() {
    let h = Harness::new();
    let owner = h.token(h.user("owner"));
    let (gid, _) = make_guild(&h, &owner).await;

    // bob 합류 → GUILD_MEMBER_ADD emit.
    let (_, inv) = h.req("POST", &format!("/guilds/{gid}/invites"), Some(&owner), Some(json!({}))).await;
    let code = inv["code"].as_str().unwrap().to_string();
    let bob_id = h.user("bob");
    let bob = h.token(bob_id);
    let (st, _) = h.req("POST", &format!("/invites/{code}"), Some(&bob), None).await;
    assert_eq!(st, StatusCode::OK);
    assert!(h.emitted().contains(&"GUILD_MEMBER_ADD".to_string()), "ADD emit 누락: {:?}", h.emitted());

    // 멤버 목록 = owner + bob.
    let (st, list) = h.req("GET", &format!("/guilds/{gid}/members"), Some(&owner), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 2);

    // bob 본인 닉 변경(@me) — @everyone 기본에 CHANGE_NICKNAME 포함 → GUILD_MEMBER_UPDATE emit.
    let (st, m) = h.req("PATCH", &format!("/guilds/{gid}/members/@me"), Some(&bob), Some(json!({"nick":"Bobby"}))).await;
    assert_eq!(st, StatusCode::OK, "patch nick: {m}");
    assert_eq!(m["nick"], "Bobby");
    assert_eq!(m["user_id"], bob_id.to_string());
    assert!(h.emitted().contains(&"GUILD_MEMBER_UPDATE".to_string()));

    // 단건 조회로 nick 반영 확인.
    let (st, one) = h.req("GET", &format!("/guilds/{gid}/members/{bob_id}"), Some(&owner), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(one["nick"], "Bobby");

    // bob 탈퇴(@me) → GUILD_MEMBER_REMOVE emit.
    let (st, _) = h.req("DELETE", &format!("/guilds/{gid}/members/@me"), Some(&bob), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert!(h.emitted().contains(&"GUILD_MEMBER_REMOVE".to_string()));

    // 목록 = owner만.
    let (_, list) = h.req("GET", &format!("/guilds/{gid}/members"), Some(&owner), None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn member_kick_perms_and_owner_protected() {
    let h = Harness::new();
    let owner_id = h.user("owner");
    let owner = h.token(owner_id);
    let (gid, _) = make_guild(&h, &owner).await;

    // bob, carol 합류 (멤버지만 KICK_MEMBERS/MANAGE_NICKNAMES 없음).
    let (_, inv) = h.req("POST", &format!("/guilds/{gid}/invites"), Some(&owner), Some(json!({}))).await;
    let code = inv["code"].as_str().unwrap().to_string();
    let bob_id = h.user("bob");
    let bob = h.token(bob_id);
    let carol_id = h.user("carol");
    let carol = h.token(carol_id);
    h.req("POST", &format!("/invites/{code}"), Some(&bob), None).await;
    h.req("POST", &format!("/invites/{code}"), Some(&carol), None).await;

    // 비멤버는 멤버 목록 403.
    let outsider = h.token(h.user("outsider"));
    let (st, _) = h.req("GET", &format!("/guilds/{gid}/members"), Some(&outsider), None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // bob(KICK_MEMBERS 없음)이 carol 추방 → 403.
    let (st, _) = h.req("DELETE", &format!("/guilds/{gid}/members/{carol_id}"), Some(&bob), None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // 소유자는 추방 불가 (고아화 방지) → 400.
    let (st, _) = h.req("DELETE", &format!("/guilds/{gid}/members/{owner_id}"), Some(&owner), None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    // bob이 타인(owner) 닉 변경 시도 → MANAGE_NICKNAMES 없음 403.
    let (st, _) = h.req("PATCH", &format!("/guilds/{gid}/members/{owner_id}"), Some(&bob), Some(json!({"nick":"x"}))).await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // owner가 carol 추방 → 204.
    let (st, _) = h.req("DELETE", &format!("/guilds/{gid}/members/{carol_id}"), Some(&owner), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (_, list) = h.req("GET", &format!("/guilds/{gid}/members"), Some(&owner), None).await;
    assert_eq!(list.as_array().unwrap().len(), 2); // owner + bob
}

/// 메시지 시드 헬퍼 (rest-api엔 전송 라우트가 없어 store에 직접).
async fn seed_message(h: &Harness, chan: &str, realm: &str, author: u64, content: &str) -> u64 {
    let mid = h.snow.next(h.clock.now_ms()).raw();
    h.store
        .create_message(&NewMessage {
            id: MessageId(Snowflake::from_raw(mid)),
            channel_id: ChannelId(Snowflake::from_raw(chan.parse().unwrap())),
            realm_id: RealmId(Snowflake::from_raw(realm.parse().unwrap())),
            author_id: UserId(Snowflake::from_raw(author)),
            content: content.into(),
            nonce: None,
            reference_message_id: None,
        })
        .await
        .unwrap();
    mid
}

#[tokio::test]
async fn message_edit_and_soft_delete() {
    let h = Harness::new();
    let owner_id = h.user("owner");
    let owner = h.token(owner_id);
    let (gid, chan) = make_guild(&h, &owner).await;
    let mid = seed_message(&h, &chan, &gid, owner_id, "orig").await;

    // 비작성자 편집 → 403.
    let mallory = h.token(h.user("mallory"));
    let (st, _) = h.req("PATCH", &format!("/channels/{chan}/messages/{mid}"), Some(&mallory), Some(json!({"content":"hax"}))).await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // 작성자 편집 → 200 + MESSAGE_UPDATE.
    let (st, m) = h.req("PATCH", &format!("/channels/{chan}/messages/{mid}"), Some(&owner), Some(json!({"content":"edited"}))).await;
    assert_eq!(st, StatusCode::OK, "edit: {m}");
    assert_eq!(m["content"], "edited");
    assert!(h.emitted().contains(&"MESSAGE_UPDATE".to_string()));

    // 빈 내용 → 400.
    let (st, _) = h.req("PATCH", &format!("/channels/{chan}/messages/{mid}"), Some(&owner), Some(json!({"content":"   "}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    // bob 합류(멤버지만 MANAGE_MESSAGES 없음).
    let (_, inv) = h.req("POST", &format!("/guilds/{gid}/invites"), Some(&owner), Some(json!({}))).await;
    let code = inv["code"].as_str().unwrap().to_string();
    let bob = h.token(h.user("bob"));
    h.req("POST", &format!("/invites/{code}"), Some(&bob), None).await;

    // bob이 owner 메시지 삭제 → MANAGE_MESSAGES 없음 403.
    let (st, _) = h.req("DELETE", &format!("/channels/{chan}/messages/{mid}"), Some(&bob), None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // 작성자 삭제 → 204 + MESSAGE_DELETE + 히스토리에서 제외.
    let (st, _) = h.req("DELETE", &format!("/channels/{chan}/messages/{mid}"), Some(&owner), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert!(h.emitted().contains(&"MESSAGE_DELETE".to_string()));
    // 삭제 후 재편집 → 404.
    let (st, _) = h.req("PATCH", &format!("/channels/{chan}/messages/{mid}"), Some(&owner), Some(json!({"content":"again"}))).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn message_reactions_add_idempotent_and_remove() {
    let h = Harness::new();
    let owner_id = h.user("owner");
    let owner = h.token(owner_id);
    let (gid, chan) = make_guild(&h, &owner).await;
    let mid = seed_message(&h, &chan, &gid, owner_id, "react to me").await;

    // 리액션 추가(@me) → 204 + REACTION_ADD (default @everyone에 ADD_REACTIONS 포함).
    let (st, _) = h.req("PUT", &format!("/channels/{chan}/messages/{mid}/reactions/smile/@me"), Some(&owner), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert_eq!(h.emitted().iter().filter(|t| *t == "MESSAGE_REACTION_ADD").count(), 1);

    // 멱등: 같은 리액션 재추가 → 204지만 emit 추가 안 됨.
    let (st, _) = h.req("PUT", &format!("/channels/{chan}/messages/{mid}/reactions/smile/@me"), Some(&owner), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert_eq!(h.emitted().iter().filter(|t| *t == "MESSAGE_REACTION_ADD").count(), 1, "멱등 재추가는 재팬아웃 안 함");

    // 제거 → 204 + REACTION_REMOVE.
    let (st, _) = h.req("DELETE", &format!("/channels/{chan}/messages/{mid}/reactions/smile/@me"), Some(&owner), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert!(h.emitted().contains(&"MESSAGE_REACTION_REMOVE".to_string()));

    // 존재하지 않는 메시지에 리액션 → 404.
    let (st, _) = h.req("PUT", &format!("/channels/{chan}/messages/999999/reactions/smile/@me"), Some(&owner), None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

/// 1:1 DM 열기는 멱등(find-or-create)이며, 두 참가자는 멤버라 채널 권한 경로(default_everyone)로
/// 히스토리 조회가 통과한다 — 비참가자는 403 (Realm 통일 P4: 길드와 동일 경로 재사용).
#[tokio::test]
async fn dm_open_idempotent_and_member_gated() {
    let h = Harness::new();
    let alice_id = h.user("alice");
    let alice = h.token(alice_id);
    let bob_id = h.user("bob");
    let bob = h.token(bob_id);

    // alice가 bob과 DM 열기 → 201.
    let (st, dm) = h.req("POST", "/users/@me/channels", Some(&alice), Some(json!({"recipient_id": bob_id.to_string()}))).await;
    assert_eq!(st, StatusCode::CREATED, "open dm: {dm}");
    assert_eq!(dm["kind"], "dm");
    let chan = dm["id"].as_str().unwrap().to_string();

    // 다시 열기(반대 방향, bob→alice) → 같은 채널 재사용(200).
    let (st, dm2) = h.req("POST", "/users/@me/channels", Some(&bob), Some(json!({"recipient_id": alice_id.to_string()}))).await;
    assert_eq!(st, StatusCode::OK, "reopen dm: {dm2}");
    assert_eq!(dm2["id"], chan, "같은 두 사람의 DM은 같은 채널");

    // 자기 자신과 DM → 400.
    let (st, _) = h.req("POST", "/users/@me/channels", Some(&alice), Some(json!({"recipient_id": alice_id.to_string()}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    // 참가자는 DM 채널 히스토리 조회 200 (default_everyone VIEW+READ_HISTORY).
    let (st, _) = h.req("GET", &format!("/channels/{chan}/messages"), Some(&alice), None).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = h.req("GET", &format!("/channels/{chan}/messages"), Some(&bob), None).await;
    assert_eq!(st, StatusCode::OK);

    // 비참가자는 403 (멤버 아님).
    let carol = h.token(h.user("carol"));
    let (st, _) = h.req("GET", &format!("/channels/{chan}/messages"), Some(&carol), None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}

/// 그룹DM 생성 + 참가자 관리: 소유자만 추가/타인제거, 본인 탈퇴, 소유자 탈퇴 불가.
#[tokio::test]
async fn group_dm_recipient_management() {
    let h = Harness::new();
    let owner_id = h.user("gowner");
    let owner = h.token(owner_id);
    let bob_id = h.user("gbob");
    let bob = h.token(bob_id);
    let carol_id = h.user("gcarol");

    // 그룹DM 생성 (owner + bob).
    let (st, g) = h.req("POST", "/users/@me/channels", Some(&owner), Some(json!({"recipient_ids": [bob_id.to_string()], "name": "squad"}))).await;
    assert_eq!(st, StatusCode::CREATED, "create group: {g}");
    assert_eq!(g["kind"], "group_dm");
    let chan = g["id"].as_str().unwrap().to_string();

    // bob(비소유)이 carol 추가 시도 → 403.
    let (st, _) = h.req("PUT", &format!("/channels/{chan}/recipients/{carol_id}"), Some(&bob), None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // owner가 carol 추가 → 204 + CHANNEL_RECIPIENT_ADD.
    let (st, _) = h.req("PUT", &format!("/channels/{chan}/recipients/{carol_id}"), Some(&owner), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert!(h.emitted().contains(&"CHANNEL_RECIPIENT_ADD".to_string()));

    // 이제 carol은 멤버 → 히스토리 조회 200.
    let carol = h.token(carol_id);
    let (st, _) = h.req("GET", &format!("/channels/{chan}/messages"), Some(&carol), None).await;
    assert_eq!(st, StatusCode::OK);

    // bob 본인 탈퇴 → 204 + CHANNEL_RECIPIENT_REMOVE.
    let (st, _) = h.req("DELETE", &format!("/channels/{chan}/recipients/{bob_id}"), Some(&bob), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    assert!(h.emitted().contains(&"CHANNEL_RECIPIENT_REMOVE".to_string()));

    // 소유자 탈퇴 불가 → 400.
    let (st, _) = h.req("DELETE", &format!("/channels/{chan}/recipients/{owner_id}"), Some(&owner), None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    // 길드 채널엔 recipient 라우트 사용 불가 (group_dm 아님) → 400.
    let (gid, gchan) = make_guild(&h, &owner).await;
    let _ = gid;
    let (st, _) = h.req("PUT", &format!("/channels/{gchan}/recipients/{bob_id}"), Some(&owner), None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

/// 친구 요청 → 수락 → 제거, 차단/해제의 상태기계 + RELATIONSHIP_* 통지.
#[tokio::test]
async fn relationship_friend_and_block_lifecycle() {
    let h = Harness::new();
    let a_id = h.user("rel_a");
    let a = h.token(a_id);
    let b_id = h.user("rel_b");
    let b = h.token(b_id);

    // a → b 친구 요청 → pending_out.
    let (st, r) = h.req("PUT", &format!("/users/@me/relationships/{b_id}"), Some(&a), Some(json!({"type":"friend"}))).await;
    assert_eq!(st, StatusCode::OK, "friend req: {r}");
    assert_eq!(r["kind"], "pending_out");
    assert!(h.user_emitted().contains(&"RELATIONSHIP_ADD".to_string()));

    // b 관점: pending_in.
    let (_, list) = h.req("GET", "/users/@me/relationships", Some(&b), None).await;
    assert_eq!(list[0]["kind"], "pending_in");
    assert_eq!(list[0]["user_id"], a_id.to_string());

    // b가 수락(같은 엔드포인트) → friend 양쪽.
    let (st, r) = h.req("PUT", &format!("/users/@me/relationships/{a_id}"), Some(&b), Some(json!({"type":"friend"}))).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(r["kind"], "friend");
    let (_, la) = h.req("GET", "/users/@me/relationships", Some(&a), None).await;
    assert_eq!(la[0]["kind"], "friend");

    // 자기 자신 → 400.
    let (st, _) = h.req("PUT", &format!("/users/@me/relationships/{a_id}"), Some(&a), Some(json!({"type":"friend"}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    // a가 친구 삭제 → 양쪽 비어짐.
    let (st, _) = h.req("DELETE", &format!("/users/@me/relationships/{b_id}"), Some(&a), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (_, la) = h.req("GET", "/users/@me/relationships", Some(&a), None).await;
    assert_eq!(la.as_array().unwrap().len(), 0);
    let (_, lb) = h.req("GET", "/users/@me/relationships", Some(&b), None).await;
    assert_eq!(lb.as_array().unwrap().len(), 0);

    // 없는 관계 삭제 → 404.
    let (st, _) = h.req("DELETE", &format!("/users/@me/relationships/{b_id}"), Some(&a), None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);

    // a가 b 차단 → a행 blocked. b가 a에게 친구 요청 → 403(상대가 차단).
    let (st, _) = h.req("PUT", &format!("/users/@me/relationships/{b_id}"), Some(&a), Some(json!({"type":"block"}))).await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = h.req("PUT", &format!("/users/@me/relationships/{a_id}"), Some(&b), Some(json!({"type":"friend"}))).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    // a가 차단 상태에서 친구 요청 → 400(먼저 해제).
    let (st, _) = h.req("PUT", &format!("/users/@me/relationships/{b_id}"), Some(&a), Some(json!({"type":"friend"}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
}

/// 차단 시 1:1 DM 열기가 거부된다 (permissions.md §5 seam 닫힘).
#[tokio::test]
async fn block_prevents_dm_open() {
    let h = Harness::new();
    let a_id = h.user("blk_a");
    let a = h.token(a_id);
    let b_id = h.user("blk_b");
    let b = h.token(b_id);

    // a가 b 차단.
    let (st, _) = h.req("PUT", &format!("/users/@me/relationships/{b_id}"), Some(&a), Some(json!({"type":"block"}))).await;
    assert_eq!(st, StatusCode::OK);

    // a→b DM 열기 거부 (내가 차단).
    let (st, _) = h.req("POST", "/users/@me/channels", Some(&a), Some(json!({"recipient_id": b_id.to_string()}))).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    // b→a DM 열기도 거부 (상대가 나를 차단).
    let (st, _) = h.req("POST", "/users/@me/channels", Some(&b), Some(json!({"recipient_id": a_id.to_string()}))).await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // 해제 후엔 가능.
    let (st, _) = h.req("DELETE", &format!("/users/@me/relationships/{b_id}"), Some(&a), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, _) = h.req("POST", "/users/@me/channels", Some(&a), Some(json!({"recipient_id": b_id.to_string()}))).await;
    assert_eq!(st, StatusCode::CREATED);
}

/// 읽음 상태: ack가 last_read 갱신 + 멘션수 재계산 + MESSAGE_ACK 통지. 권한·경계 검증.
#[tokio::test]
async fn read_state_ack_and_mention_recount() {
    let h = Harness::new();
    let owner_id = h.user("rs_owner");
    let owner = h.token(owner_id);
    let (gid, chan) = make_guild(&h, &owner).await;

    // bob 합류(멤버).
    let bob_id = h.user("rs_bob");
    let bob = h.token(bob_id);
    h.store.add_member(RealmId(Snowflake::from_raw(gid.parse().unwrap())), UserId(Snowflake::from_raw(bob_id))).await.unwrap();

    // owner가 bob을 멘션하는 메시지 2개 시드 + 멘션 행(dispatch 없이 직접).
    let m0 = seed_message(&h, &chan, &gid, owner_id, "<@bob> hi 0").await;
    let m1 = seed_message(&h, &chan, &gid, owner_id, "<@bob> hi 1").await;
    let bob_uid = UserId(Snowflake::from_raw(bob_id));
    h.store.add_mentions(MessageId(Snowflake::from_raw(m0)), &[bob_uid]).await.unwrap();
    h.store.add_mentions(MessageId(Snowflake::from_raw(m1)), &[bob_uid]).await.unwrap();

    // 초기: bob 읽음 상태 없음.
    let (st, rs) = h.req("GET", "/users/@me/read-states", Some(&bob), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(rs.as_array().unwrap().len(), 0);

    // m0까지 ack → m0 이후 멘션 1개(m1) 남음 + MESSAGE_ACK.
    let (st, s) = h.req("POST", &format!("/channels/{chan}/messages/{m0}/ack"), Some(&bob), None).await;
    assert_eq!(st, StatusCode::OK, "ack: {s}");
    assert_eq!(s["last_read_message_id"], m0.to_string());
    assert_eq!(s["mention_count"], 1);
    assert!(h.user_emitted().contains(&"MESSAGE_ACK".to_string()));

    // m1까지 ack → 0.
    let (st, s) = h.req("POST", &format!("/channels/{chan}/messages/{m1}/ack"), Some(&bob), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(s["mention_count"], 0);

    // 목록에 반영.
    let (_, rs) = h.req("GET", "/users/@me/read-states", Some(&bob), None).await;
    assert_eq!(rs.as_array().unwrap().len(), 1);
    assert_eq!(rs[0]["last_read_message_id"], m1.to_string());

    // 비멤버는 ack 403.
    let outsider = h.token(h.user("rs_out"));
    let (st, _) = h.req("POST", &format!("/channels/{chan}/messages/{m1}/ack"), Some(&outsider), None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // 존재하지 않는 메시지 ack → 404.
    let (st, _) = h.req("POST", &format!("/channels/{chan}/messages/999999/ack"), Some(&bob), None).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

// ── 스레드 / 포럼 (Phase 4) ────────────────────────────────────────────

#[tokio::test]
async fn thread_create_list_archive_and_forum_channel() {
    let h = Harness::new();
    let owner_id = h.user("th_owner");
    let owner = h.token(owner_id);
    let (gid, general) = make_guild(&h, &owner).await;

    // 포럼 채널 생성 (kind=forum).
    let (st, f) = h.req("POST", &format!("/guilds/{gid}/channels"), Some(&owner), Some(json!({"name":"help","kind":"forum"}))).await;
    assert_eq!(st, StatusCode::CREATED, "forum: {f}");
    assert_eq!(f["kind"], "forum");

    // 잘못된 kind → 400.
    let (st, _) = h.req("POST", &format!("/guilds/{gid}/channels"), Some(&owner), Some(json!({"name":"x","kind":"thread"}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "thread는 채널 생성으로 불가");

    // bob 합류(멤버, default_everyone에 CREATE_PUBLIC_THREADS 포함).
    let bob_id = h.user("th_bob");
    let bob = h.token(bob_id);
    h.store.add_member(RealmId(Snowflake::from_raw(gid.parse().unwrap())), UserId(Snowflake::from_raw(bob_id))).await.unwrap();

    // bob이 general 아래 스레드 생성 → 201 + THREAD_CREATE.
    let (st, t) = h.req("POST", &format!("/channels/{general}/threads"), Some(&bob), Some(json!({"name":"discussion"}))).await;
    assert_eq!(st, StatusCode::CREATED, "thread: {t}");
    let tid = t["id"].as_str().unwrap().to_string();
    assert_eq!(t["parent_id"], general);
    assert_eq!(t["owner_id"], bob_id.to_string());
    assert!(!t["archived"].as_bool().unwrap());
    assert!(h.emitted().contains(&"THREAD_CREATE".to_string()));

    // 목록.
    let (st, list) = h.req("GET", &format!("/channels/{general}/threads"), Some(&owner), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 1);

    // 비멤버는 생성 403.
    let outsider = h.token(h.user("th_out"));
    let (st, _) = h.req("POST", &format!("/channels/{general}/threads"), Some(&outsider), Some(json!({"name":"x"}))).await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // 아카이브: 소유자(bob) → 200 + THREAD_UPDATE + archived.
    let (st, u) = h.req("PATCH", &format!("/channels/{tid}/thread"), Some(&bob), Some(json!({"archived":true}))).await;
    assert_eq!(st, StatusCode::OK, "archive: {u}");
    assert!(u["archived"].as_bool().unwrap());
    assert!(h.emitted().contains(&"THREAD_UPDATE".to_string()));

    // 제3자(멤버지만 비소유·MANAGE_THREADS 없음? — default엔 MANAGE_THREADS 없음) 아카이브 → 403.
    let carol_id = h.user("th_carol");
    let carol = h.token(carol_id);
    h.store.add_member(RealmId(Snowflake::from_raw(gid.parse().unwrap())), UserId(Snowflake::from_raw(carol_id))).await.unwrap();
    let (st, _) = h.req("PATCH", &format!("/channels/{tid}/thread"), Some(&carol), Some(json!({"archived":false}))).await;
    assert_eq!(st, StatusCode::FORBIDDEN, "비소유·MANAGE_THREADS 없음 → 거부");

    // owner(길드 소유자=권한 단축)는 아카이브 가능.
    let (st, _) = h.req("PATCH", &format!("/channels/{tid}/thread"), Some(&owner), Some(json!({"archived":false}))).await;
    assert_eq!(st, StatusCode::OK);
}

// ── 감사 로그 (Phase 4) ────────────────────────────────────────────────

#[tokio::test]
async fn audit_log_records_and_lists() {
    let h = Harness::new();
    let owner_id = h.user("al_owner");
    let owner = h.token(owner_id);
    let (gid, _chan) = make_guild(&h, &owner).await;

    // 채널 생성 → ChannelCreate(10), 역할 생성 → RoleCreate(30) 감사 기록.
    h.req("POST", &format!("/guilds/{gid}/channels"), Some(&owner), Some(json!({"name":"ops"}))).await;
    h.req("POST", &format!("/guilds/{gid}/roles"), Some(&owner), Some(json!({"name":"mod","permissions":0}))).await;

    // owner(소유자=VIEW_AUDIT_LOG 포함)는 조회 가능. 최신순(역할이 먼저).
    let (st, logs) = h.req("GET", &format!("/guilds/{gid}/audit-logs"), Some(&owner), None).await;
    assert_eq!(st, StatusCode::OK, "audit: {logs}");
    let arr = logs.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["action_type"], 30, "최신 = RoleCreate");
    assert_eq!(arr[1]["action_type"], 10, "ChannelCreate");
    assert_eq!(arr[0]["changes"]["name"], "mod");

    // 일반 멤버(VIEW_AUDIT_LOG 없음)는 403.
    let bob = h.token(h.user("al_bob"));
    h.store.add_member(RealmId(Snowflake::from_raw(gid.parse().unwrap())), UserId(Snowflake::from_raw(h.keys.verify_access(&bob).unwrap()))).await.unwrap();
    let (st, _) = h.req("GET", &format!("/guilds/{gid}/audit-logs"), Some(&bob), None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}

// ── 웹훅 (Phase 4) ─────────────────────────────────────────────────────

#[tokio::test]
async fn webhook_create_execute_and_permissions() {
    let h = Harness::new();
    let owner_id = h.user("wh_owner");
    let owner = h.token(owner_id);
    let (_gid, chan) = make_guild(&h, &owner).await;

    // owner(소유자=권한 단축, MANAGE_WEBHOOKS) 생성 → 토큰 1회 반환.
    let (st, w) = h.req("POST", &format!("/channels/{chan}/webhooks"), Some(&owner), Some(json!({"name":"ci"}))).await;
    assert_eq!(st, StatusCode::CREATED, "webhook: {w}");
    let wid = w["id"].as_str().unwrap().to_string();
    let token = w["token"].as_str().unwrap().to_string();

    // 목록엔 토큰 노출 안 됨.
    let (st, list) = h.req("GET", &format!("/channels/{chan}/webhooks"), Some(&owner), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert!(list[0]["token"].is_null());

    // 일반 멤버(MANAGE_WEBHOOKS 없음)는 생성/목록 403.
    let bob = h.token(h.user("wh_bob"));
    h.store.add_member(RealmId(Snowflake::from_raw(_gid.parse().unwrap())), UserId(Snowflake::from_raw(h.keys.verify_access(&bob).unwrap()))).await.unwrap();
    let (st, _) = h.req("GET", &format!("/channels/{chan}/webhooks"), Some(&bob), None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // 실행(Bearer 없음, URL 토큰) → 200 + MESSAGE_CREATE emit + 메시지 적재.
    let (st, ex) = h.req("POST", &format!("/webhooks/{wid}/{token}"), None, Some(json!({"content":"hello from CI"}))).await;
    assert_eq!(st, StatusCode::OK, "execute: {ex}");
    assert!(h.emitted().contains(&"MESSAGE_CREATE".to_string()));

    // 틀린 토큰 → 401.
    let (st, _) = h.req("POST", &format!("/webhooks/{wid}/deadbeef"), None, Some(json!({"content":"x"}))).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED);

    // 빈 내용 → 400.
    let (st, _) = h.req("POST", &format!("/webhooks/{wid}/{token}"), None, Some(json!({"content":"  "}))).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    // 삭제(MANAGE_WEBHOOKS) → 204, 이후 실행 404.
    let (st, _) = h.req("DELETE", &format!("/webhooks/{wid}"), Some(&owner), None).await;
    assert_eq!(st, StatusCode::NO_CONTENT);
    let (st, _) = h.req("POST", &format!("/webhooks/{wid}/{token}"), None, Some(json!({"content":"after delete"}))).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

// ── 첨부 (D37, 로컬 FS) ────────────────────────────────────────────────

#[tokio::test]
async fn attachment_upload_download_and_permissions() {
    let h = Harness::new();
    let owner_id = h.user("at_owner");
    let owner = h.token(owner_id);
    let (gid, chan) = make_guild(&h, &owner).await;
    let mid = seed_message(&h, &chan, &gid, owner_id, "with file").await;

    // owner(작성자) 업로드 → 201 + 메타.
    let (st, a) = h.upload(&format!("/channels/{chan}/messages/{mid}/attachments"), &owner, "hello.txt", "text/plain", b"hello bytes").await;
    assert_eq!(st, StatusCode::CREATED, "upload: {a}");
    let aid = a["id"].as_str().unwrap().to_string();
    assert_eq!(a["filename"], "hello.txt");
    assert_eq!(a["size_bytes"], 11);
    assert_eq!(a["url"], format!("/attachments/{aid}"));
    // 바이트가 BlobStore에 저장됨.
    assert_eq!(h.blobs.blobs.lock().unwrap().get(&aid).map(|v| v.len()), Some(11));

    // 목록 → 1.
    let (st, list) = h.req("GET", &format!("/channels/{chan}/messages/{mid}/attachments"), Some(&owner), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(list.as_array().unwrap().len(), 1);

    // 다운로드 → 바이트 일치.
    let (st, bytes) = h.get_bytes(&format!("/attachments/{aid}"), &owner).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(bytes, b"hello bytes");

    // 비작성자(멤버) 업로드 → 403.
    let bob_id = h.user("at_bob");
    let bob = h.token(bob_id);
    h.store.add_member(RealmId(Snowflake::from_raw(gid.parse().unwrap())), UserId(Snowflake::from_raw(bob_id))).await.unwrap();
    let (st, _) = h.upload(&format!("/channels/{chan}/messages/{mid}/attachments"), &bob, "x.txt", "text/plain", b"x").await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // 비멤버 다운로드 → 403.
    let outsider = h.token(h.user("at_out"));
    let (st, _) = h.get_bytes(&format!("/attachments/{aid}"), &outsider).await;
    assert_eq!(st, StatusCode::FORBIDDEN);

    // 없는 첨부 다운로드 → 404.
    let (st, _) = h.get_bytes("/attachments/999999", &owner).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

// ── 전문검색 (Q10, FTS) ────────────────────────────────────────────────

#[tokio::test]
async fn search_matches_and_channel_permission_filter() {
    let h = Harness::new();
    let owner_id = h.user("se_owner");
    let owner = h.token(owner_id);
    let (gid, general) = make_guild(&h, &owner).await;

    // 둘째 채널 'secret' 생성.
    let (st, c) = h.req("POST", &format!("/guilds/{gid}/channels"), Some(&owner), Some(json!({"name":"secret"}))).await;
    assert_eq!(st, StatusCode::CREATED, "channel: {c}");
    let secret = c["id"].as_str().unwrap().to_string();

    // 메시지 시드: general에 "fox", secret에 "fox".
    seed_message(&h, &general, &gid, owner_id, "the quick brown fox").await;
    seed_message(&h, &general, &gid, owner_id, "lazy dog sleeps").await;
    seed_message(&h, &secret, &gid, owner_id, "secret fox lives here").await;

    // owner(소유자=권한 단축) 검색 "fox" → 양 채널 2건.
    let (st, hits) = h.req("GET", &format!("/guilds/{gid}/messages/search?content=fox"), Some(&owner), None).await;
    assert_eq!(st, StatusCode::OK, "search: {hits}");
    assert_eq!(hits.as_array().unwrap().len(), 2);

    // 매칭 없음 → 빈 결과.
    let (st, none) = h.req("GET", &format!("/guilds/{gid}/messages/search?content=elephant"), Some(&owner), None).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(none.as_array().unwrap().len(), 0);

    // 빈 검색어 → 400.
    let (st, _) = h.req("GET", &format!("/guilds/{gid}/messages/search?content=%20"), Some(&owner), None).await;
    assert_eq!(st, StatusCode::BAD_REQUEST);

    // bob 합류(멤버, 비소유자).
    let bob_id = h.user("se_bob");
    let bob = h.token(bob_id);
    h.store.add_member(RealmId(Snowflake::from_raw(gid.parse().unwrap())), UserId(Snowflake::from_raw(bob_id))).await.unwrap();

    // secret 채널에 @everyone VIEW_CHANNEL deny 오버라이드 (target = @everyone = realm id).
    let (st, _) = h
        .req(
            "PUT",
            &format!("/channels/{secret}/permissions/{gid}"),
            Some(&owner),
            Some(json!({ "type": "role", "allow": 0, "deny": Permissions::VIEW_CHANNEL.bits() })),
        )
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT);

    // bob 검색 "fox" → secret은 VIEW_CHANNEL 없어 제외, general 1건만.
    let (st, bob_hits) = h.req("GET", &format!("/guilds/{gid}/messages/search?content=fox"), Some(&bob), None).await;
    assert_eq!(st, StatusCode::OK, "bob search: {bob_hits}");
    let arr = bob_hits.as_array().unwrap();
    assert_eq!(arr.len(), 1, "secret 채널은 권한 필터로 제외");
    assert_eq!(arr[0]["channel_id"], general);

    // 비멤버 → 403.
    let outsider = h.token(h.user("se_out"));
    let (st, _) = h.req("GET", &format!("/guilds/{gid}/messages/search?content=fox"), Some(&outsider), None).await;
    assert_eq!(st, StatusCode::FORBIDDEN);
}

// ── WebAuthn / Passkeys (D19) ──────────────────────────────────────────
// SoftPasskey로 register→login ceremony를 HTTP 계층 전체에 통과시켜 검증(헤드리스).
#[tokio::test]
async fn webauthn_register_then_passwordless_login() {
    use auth::webauthn::{CreationChallengeResponse, RequestChallengeResponse, Url};
    use webauthn_authenticator_rs::WebauthnAuthenticator;
    use webauthn_authenticator_rs::softpasskey::SoftPasskey;

    let h = Harness::new();
    let uid = h.user("wa_alice");
    let tok = h.token(uid);
    let origin = Url::parse("http://localhost:8080").unwrap();
    let mut authr = WebauthnAuthenticator::new(SoftPasskey::new(true));

    // 1) 등록 시작 → challenge.
    let (st, body) = h.req("POST", "/auth/webauthn/register/start", Some(&tok), Some(json!({}))).await;
    assert_eq!(st, StatusCode::OK, "register/start: {body}");
    let cid = body["ceremony_id"].as_str().unwrap().to_string();
    let ccr: CreationChallengeResponse = serde_json::from_value(body["options"].clone()).unwrap();

    // 2) SoftPasskey가 자격증명 생성 → 등록 완료.
    let rpkc = authr.do_registration(origin.clone(), ccr).expect("soft register");
    let (st, body) = h
        .req(
            "POST",
            "/auth/webauthn/register/finish",
            Some(&tok),
            Some(json!({ "ceremony_id": cid, "credential": rpkc })),
        )
        .await;
    assert_eq!(st, StatusCode::NO_CONTENT, "register/finish: {body}");

    // 3) 암호 없는 로그인 시작 → challenge.
    let (st, body) = h.req("POST", "/auth/webauthn/login/start", None, Some(json!({ "username": "wa_alice" }))).await;
    assert_eq!(st, StatusCode::OK, "login/start: {body}");
    let cid = body["ceremony_id"].as_str().unwrap().to_string();
    let rcr: RequestChallengeResponse = serde_json::from_value(body["options"].clone()).unwrap();

    // 4) SoftPasskey가 challenge에 서명 → 로그인 완료 → 토큰 발급.
    let pkc = authr.do_authentication(origin, rcr).expect("soft auth");
    let (st, body) = h
        .req(
            "POST",
            "/auth/webauthn/login/finish",
            None,
            Some(json!({ "ceremony_id": cid, "credential": pkc })),
        )
        .await;
    assert_eq!(st, StatusCode::OK, "login/finish: {body}");
    assert!(body["access_token"].as_str().is_some(), "암호 없이 access 토큰 발급");
    assert!(body["refresh_token"].as_str().is_some());
    assert_eq!(body["user_id"].as_str(), Some(uid.to_string().as_str()));
}

// 알 수 없는 ceremony_id로 finish → 400.
#[tokio::test]
async fn webauthn_unknown_ceremony_rejected() {
    let h = Harness::new();
    let (st, _) = h
        .req("POST", "/auth/webauthn/login/finish", None, Some(json!({ "ceremony_id": "999", "credential": {} })))
        .await;
    assert!(st.is_client_error(), "잘못된 ceremony/credential은 4xx 거부 (got {st})");
}

// ── CRDT 오프라인 동기화 (D49) ─────────────────────────────────────────

#[tokio::test]
async fn crdt_sync_two_devices_converge() {
    let h = Harness::new();
    let u = h.user("syncer");
    let token = h.token(u);

    // 기기1(node 1): draft="phone" @200. 기기2(node 2): 같은 키 draft="laptop" @210 (오프라인 동시 편집).
    let (s1, _) = h
        .req("POST", "/users/@me/sync", Some(&token),
            Some(json!({ "entries": [{ "key": "draft", "value": "phone", "ts": 200, "node": 1 }] })))
        .await;
    assert_eq!(s1, StatusCode::OK);
    let (s2, d2) = h
        .req("POST", "/users/@me/sync", Some(&token),
            Some(json!({ "entries": [{ "key": "draft", "value": "laptop", "ts": 210, "node": 2 }] })))
        .await;
    assert_eq!(s2, StatusCode::OK);
    // 더 늦은 쓰기(laptop)로 수렴 (LWW).
    assert_eq!(d2["live"]["draft"], json!("laptop"));

    // 이른 쓰기 재push → 무시(수렴 안정). GET도 같은 값.
    let _ = h
        .req("POST", "/users/@me/sync", Some(&token),
            Some(json!({ "entries": [{ "key": "draft", "value": "phone", "ts": 200, "node": 1 }] })))
        .await;
    let (sg, dg) = h.req("GET", "/users/@me/sync", Some(&token), None).await;
    assert_eq!(sg, StatusCode::OK);
    assert_eq!(dg["live"]["draft"], json!("laptop"), "이른 쓰기는 못 덮음");

    // 툼스톤 삭제(더 늦은 dot) → live에서 사라짐.
    let (_, dd) = h
        .req("POST", "/users/@me/sync", Some(&token),
            Some(json!({ "entries": [{ "key": "draft", "value": null, "ts": 220, "node": 2 }] })))
        .await;
    assert!(dd["live"].get("draft").is_none(), "툼스톤 삭제 후 live 비어야");
}

#[tokio::test]
async fn crdt_sync_requires_auth() {
    let h = Harness::new();
    let (st, _) = h.req("GET", "/users/@me/sync", None, None).await;
    assert_eq!(st, StatusCode::UNAUTHORIZED, "인증 없으면 401");
}
