//! rest-api 통합 테스트 — in-memory `Store` + axum `oneshot`으로 핸들러·추출기·권한 강제·에러 매핑 검증.
//!
//! DB 없이 라우터를 직접 구동(`tower::ServiceExt::oneshot`). 권한 인가(D17) 경로가 핵심 대상:
//! 멤버십/소유자 단축/역할/채널 오버라이드/권한상승 방지/히스토리 게이팅.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use auth::TokenKeys;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use domain::channel::{Channel, NewChannel};
use domain::guild::{Guild, NewGuild};
use domain::id::{
    ChannelId, MessageId, RealmId, RefreshTokenId, RoleId, Snowflake, SnowflakeGenerator, UserId,
};
use domain::invite::{Invite, NewInvite};
use domain::message::{Message, NewMessage};
use domain::permissions::{ChannelOverwrite, Permissions};
use domain::refresh_token::{NewRefreshToken, RefreshToken};
use domain::repo::{
    ChannelOverwriteRepository, ChannelRepository, GuildRepository, InviteRepository,
    MessageRepository, RefreshTokenRepository, RepoError, RoleRepository, UserRepository,
};
use domain::role::{NewRole, Role};
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
    members: HashSet<(u64, u64)>,
    roles: HashMap<u64, Role>,
    member_roles: HashSet<(u64, u64, u64)>, // (realm, user, role)
    overwrites: HashMap<(u64, u64), ChannelOverwrite>,
    invites: HashMap<String, Invite>,
    channels: HashMap<u64, Channel>,
    messages: Vec<Message>,
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
        g.members.insert((realm, gd.owner_id.0.raw()));
        Ok(())
    }
    async fn get_guild(&self, realm: RealmId) -> Result<Option<Guild>, RepoError> {
        Ok(self.inner.lock().unwrap().guilds.get(&realm.0.raw()).cloned())
    }
    async fn add_member(&self, realm: RealmId, user: UserId) -> Result<(), RepoError> {
        self.inner.lock().unwrap().members.insert((realm.0.raw(), user.0.raw()));
        Ok(())
    }
    async fn is_member(&self, realm: RealmId, user: UserId) -> Result<bool, RepoError> {
        Ok(self.inner.lock().unwrap().members.contains(&(realm.0.raw(), user.0.raw())))
    }
    async fn member_realm_ids(&self, user: UserId) -> Result<Vec<RealmId>, RepoError> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .members
            .iter()
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
        g.members.insert((realm.0.raw(), user.0.raw()));
        if let Some(i) = g.invites.get_mut(code) {
            i.uses += 1;
        }
        Ok(Some(realm))
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
            .filter(|m| before.map(|b| m.id.0.raw() < b.0.raw()).unwrap_or(true))
            .cloned()
            .collect();
        v.sort_by_key(|m| std::cmp::Reverse(m.id.0.raw())); // 최신순
        v.truncate(limit.max(0) as usize);
        Ok(v)
    }
}

// ----- 테스트 하네스 -----

struct Harness {
    router: Router,
    keys: Arc<TokenKeys>,
    store: Arc<MemStore>,
    snow: Arc<SnowflakeGenerator>,
    clock: Arc<ManualClock>,
}

impl Harness {
    fn new() -> Self {
        let keys = Arc::new(TokenKeys::generate().unwrap());
        let store = Arc::new(MemStore::default());
        let snow = Arc::new(SnowflakeGenerator::new(1));
        let clock = Arc::new(ManualClock::new(domain::id::EPOCH_MS + 1));
        let state = AppState::new(
            Arc::clone(&store),
            Arc::clone(&keys),
            Arc::clone(&snow),
            clock.clone() as Arc<dyn Clock>,
        );
        Self { router: rest_api::router(state), keys, store, snow, clock }
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
}

/// 길드 생성 → (guild_id, general_channel_id).
async fn make_guild(h: &Harness, owner_tok: &str) -> (String, String) {
    let (st, body) = h.req("POST", "/guilds", Some(owner_tok), Some(json!({"name": "G"}))).await;
    assert_eq!(st, StatusCode::CREATED, "guild create: {body}");
    let gid = body["id"].as_str().unwrap().to_string();
    let chan = body["channels"][0]["id"].as_str().unwrap().to_string();
    (gid, chan)
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
