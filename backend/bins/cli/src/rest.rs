//! REST 호출 헬퍼 (개념: rest). reqwest로 서버 HTTP API 호출.

use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct AuthResponse {
    pub user_id: String,
    pub access_token: String,
    pub refresh_token: String,
}

#[derive(Deserialize)]
pub struct ChannelView {
    pub id: String,
    pub name: Option<String>,
    #[allow(dead_code)]
    pub kind: String,
}

#[derive(Deserialize)]
pub struct GuildView {
    pub id: String,
    pub name: String,
    pub channels: Vec<ChannelView>,
}

#[derive(Deserialize)]
pub struct InviteView {
    pub code: String,
    pub realm_id: String,
    pub max_uses: i32,
    pub expires_at: Option<i64>,
}

#[derive(Deserialize)]
pub struct JoinView {
    pub realm_id: String,
    pub channels: Vec<ChannelEntry>,
}

#[derive(Deserialize)]
pub struct ChannelEntry {
    pub id: String,
    pub name: Option<String>,
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

/// 실패 응답 본문을 에러 메시지로.
async fn ok_json<T: for<'de> Deserialize<'de>>(res: reqwest::Response) -> Result<T, String> {
    let status = res.status();
    if status.is_success() {
        res.json::<T>().await.map_err(|e| format!("decode: {e}"))
    } else {
        let body = res.text().await.unwrap_or_default();
        Err(format!("{status}: {body}"))
    }
}

/// 본문 없는(204) 응답 — 성공 여부만.
async fn ok_or_err(res: reqwest::Response) -> Result<(), String> {
    let status = res.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(format!("{status}: {}", res.text().await.unwrap_or_default()))
    }
}

#[derive(Serialize)]
struct RegisterBody<'a> {
    username: &'a str,
    email: &'a str,
    password: &'a str,
    pow_challenge: String,
    pow_nonce: String,
}

#[derive(Deserialize)]
struct PowChallenge {
    challenge: String,
    difficulty: u8,
}

/// 가입: ① PoW 챌린지 받기 → ② 풀기(auth crate와 동일 알고리즘) → ③ 해를 실어 등록 (D18).
pub async fn register(base: &str, username: &str, email: &str, password: &str) -> Result<AuthResponse, String> {
    let ch: PowChallenge = ok_json(
        client()
            .get(format!("{base}/auth/pow-challenge"))
            .send()
            .await
            .map_err(|e| e.to_string())?,
    )
    .await?;
    let pow_nonce = auth::pow::solve(&ch.challenge, ch.difficulty);
    let res = client()
        .post(format!("{base}/auth/register"))
        .json(&RegisterBody { username, email, password, pow_challenge: ch.challenge, pow_nonce })
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_json(res).await
}

#[derive(Serialize)]
struct LoginBody<'a> {
    username: &'a str,
    password: &'a str,
}

/// 로그인 결과: 토큰 발급 또는 MFA 2단계 필요 (D19).
pub enum LoginOutcome {
    Tokens(AuthResponse),
    MfaRequired,
}

pub async fn login(base: &str, username: &str, password: &str) -> Result<LoginOutcome, String> {
    let res = client()
        .post(format!("{base}/auth/login"))
        .json(&LoginBody { username, password })
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = res.status();
    if !status.is_success() {
        return Err(format!("{status}: {}", res.text().await.unwrap_or_default()));
    }
    let v: serde_json::Value = res.json().await.map_err(|e| format!("decode: {e}"))?;
    if v.get("mfa_required").and_then(serde_json::Value::as_bool) == Some(true) {
        Ok(LoginOutcome::MfaRequired)
    } else {
        serde_json::from_value(v).map(LoginOutcome::Tokens).map_err(|e| format!("decode: {e}"))
    }
}

#[derive(Deserialize)]
pub struct MfaEnableView {
    pub secret: String,
    pub otpauth_uri: String,
}

#[derive(Serialize)]
struct MfaVerifyBody<'a> {
    secret: &'a str,
    code: &'a str,
}

#[derive(Serialize)]
struct MfaLoginBody<'a> {
    username: &'a str,
    password: &'a str,
    code: &'a str,
}

/// MFA enable: secret(hex) + otpauth URI 수령 (아직 미저장).
pub async fn mfa_enable(base: &str, token: &str) -> Result<MfaEnableView, String> {
    let res = client()
        .post(format!("{base}/auth/mfa/totp/enable"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_json(res).await
}

/// MFA verify: secret+code 확인 → 저장(활성화).
pub async fn mfa_verify(base: &str, token: &str, secret: &str, code: &str) -> Result<(), String> {
    let res = client()
        .post(format!("{base}/auth/mfa/totp/verify"))
        .bearer_auth(token)
        .json(&MfaVerifyBody { secret, code })
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_or_err(res).await
}

/// MFA 로그인 2단계: 비번 + TOTP 코드 → 토큰.
pub async fn mfa_login(base: &str, username: &str, password: &str, code: &str) -> Result<AuthResponse, String> {
    let res = client()
        .post(format!("{base}/auth/mfa/totp"))
        .json(&MfaLoginBody { username, password, code })
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_json(res).await
}

#[derive(Serialize)]
struct RefreshBody<'a> {
    refresh_token: &'a str,
}

pub async fn refresh(base: &str, token: &str) -> Result<AuthResponse, String> {
    let res = client()
        .post(format!("{base}/auth/refresh"))
        .json(&RefreshBody { refresh_token: token })
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_json(res).await
}

#[derive(Serialize)]
struct NameBody<'a> {
    name: &'a str,
}

pub async fn create_guild(base: &str, token: &str, name: &str) -> Result<GuildView, String> {
    let res = client()
        .post(format!("{base}/guilds"))
        .bearer_auth(token)
        .json(&NameBody { name })
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_json(res).await
}

#[derive(Deserialize)]
pub struct RoleView {
    pub id: String,
    pub name: String,
    pub permissions: String,
    #[allow(dead_code)]
    pub position: i32,
}

#[derive(Serialize)]
struct CreateChannelBody<'a> {
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<&'a str>,
}

/// 채널 생성 (kind 지정 가능: text/voice/category/announcement/forum).
pub async fn create_channel_kind(base: &str, token: &str, guild: &str, name: &str, kind: Option<&str>) -> Result<ChannelView, String> {
    let res = client()
        .post(format!("{base}/guilds/{guild}/channels"))
        .bearer_auth(token)
        .json(&CreateChannelBody { name, kind })
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_json(res).await
}

#[derive(Deserialize)]
pub struct ThreadView {
    pub id: String,
    pub parent_id: String,
    pub name: Option<String>,
    pub owner_id: Option<String>,
    pub archived: bool,
    pub message_count: i64,
}

#[derive(Serialize)]
struct CreateThreadBody<'a> {
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    auto_archive: Option<i32>,
}

/// 부모 채널 아래 스레드 생성.
pub async fn create_thread(base: &str, token: &str, channel: &str, name: &str, auto_archive: Option<i32>) -> Result<ThreadView, String> {
    let res = client()
        .post(format!("{base}/channels/{channel}/threads"))
        .bearer_auth(token)
        .json(&CreateThreadBody { name, auto_archive })
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_json(res).await
}

/// 부모 채널의 스레드 목록.
pub async fn list_threads(base: &str, token: &str, channel: &str) -> Result<Vec<ThreadView>, String> {
    let res = client()
        .get(format!("{base}/channels/{channel}/threads"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_json(res).await
}

#[derive(Serialize)]
struct ArchiveBody {
    archived: bool,
}

/// 스레드 아카이브/해제 (소유자 또는 MANAGE_THREADS).
pub async fn archive_thread(base: &str, token: &str, thread: &str, archived: bool) -> Result<ThreadView, String> {
    let res = client()
        .patch(format!("{base}/channels/{thread}/thread"))
        .bearer_auth(token)
        .json(&ArchiveBody { archived })
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_json(res).await
}

#[derive(Serialize)]
struct CreateRoleBody<'a> {
    name: &'a str,
    permissions: u64,
}

pub async fn create_role(base: &str, token: &str, guild: &str, name: &str, permissions: u64) -> Result<RoleView, String> {
    let res = client()
        .post(format!("{base}/guilds/{guild}/roles"))
        .bearer_auth(token)
        .json(&CreateRoleBody { name, permissions })
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_json(res).await
}

pub async fn assign_role(base: &str, token: &str, guild: &str, user: &str, role: &str) -> Result<(), String> {
    let res = client()
        .put(format!("{base}/guilds/{guild}/members/{user}/roles/{role}"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = res.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(format!("{status}: {}", res.text().await.unwrap_or_default()))
    }
}

#[derive(Serialize)]
struct SetOverwriteBody<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    allow: u64,
    deny: u64,
}

pub async fn set_channel_perm(
    base: &str,
    token: &str,
    channel: &str,
    target: &str,
    kind: &str,
    allow: u64,
    deny: u64,
) -> Result<(), String> {
    let res = client()
        .put(format!("{base}/channels/{channel}/permissions/{target}"))
        .bearer_auth(token)
        .json(&SetOverwriteBody { kind, allow, deny })
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = res.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(format!("{status}: {}", res.text().await.unwrap_or_default()))
    }
}

#[derive(Serialize)]
struct CreateInviteBody {
    max_uses: i32,
    max_age: i64,
}

pub async fn create_invite(
    base: &str,
    token: &str,
    realm_id: &str,
    max_uses: i32,
    max_age: i64,
) -> Result<InviteView, String> {
    let res = client()
        .post(format!("{base}/guilds/{realm_id}/invites"))
        .bearer_auth(token)
        .json(&CreateInviteBody { max_uses, max_age })
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_json(res).await
}

#[derive(Deserialize)]
pub struct MemberView {
    pub user_id: String,
    pub nick: Option<String>,
    pub joined_at: i64,
    pub roles: Vec<String>,
}

pub async fn list_members(base: &str, token: &str, guild: &str) -> Result<Vec<MemberView>, String> {
    let res = client()
        .get(format!("{base}/guilds/{guild}/members"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_json(res).await
}

#[derive(Serialize)]
struct SetNickBody<'a> {
    nick: Option<&'a str>,
}

pub async fn set_nick(
    base: &str,
    token: &str,
    guild: &str,
    user: &str,
    nick: Option<&str>,
) -> Result<MemberView, String> {
    let res = client()
        .patch(format!("{base}/guilds/{guild}/members/{user}"))
        .bearer_auth(token)
        .json(&SetNickBody { nick })
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_json(res).await
}

/// 추방(타인) 또는 탈퇴(user="@me"). 204 No Content.
pub async fn remove_member(base: &str, token: &str, guild: &str, user: &str) -> Result<(), String> {
    let res = client()
        .delete(format!("{base}/guilds/{guild}/members/{user}"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = res.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(format!("{status}: {}", res.text().await.unwrap_or_default()))
    }
}

pub async fn join_invite(base: &str, token: &str, code: &str) -> Result<JoinView, String> {
    let res = client()
        .post(format!("{base}/invites/{code}"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_json(res).await
}

#[derive(Deserialize)]
pub struct MessageView {
    pub id: String,
    #[allow(dead_code)]
    pub channel_id: String,
    #[allow(dead_code)]
    pub author_id: String,
    pub content: String,
}

#[derive(Serialize)]
struct EditBody<'a> {
    content: &'a str,
}

pub async fn edit_message(base: &str, token: &str, channel: &str, message: &str, content: &str) -> Result<MessageView, String> {
    let res = client()
        .patch(format!("{base}/channels/{channel}/messages/{message}"))
        .bearer_auth(token)
        .json(&EditBody { content })
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_json(res).await
}

pub async fn delete_message(base: &str, token: &str, channel: &str, message: &str) -> Result<(), String> {
    let res = client()
        .delete(format!("{base}/channels/{channel}/messages/{message}"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_or_err(res).await
}

pub async fn add_reaction(base: &str, token: &str, channel: &str, message: &str, emoji: &str) -> Result<(), String> {
    let e = urlencoding::encode(emoji);
    let res = client()
        .put(format!("{base}/channels/{channel}/messages/{message}/reactions/{e}/@me"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_or_err(res).await
}

pub async fn remove_reaction(base: &str, token: &str, channel: &str, message: &str, emoji: &str) -> Result<(), String> {
    let e = urlencoding::encode(emoji);
    let res = client()
        .delete(format!("{base}/channels/{channel}/messages/{message}/reactions/{e}/@me"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_or_err(res).await
}

#[derive(Deserialize)]
pub struct DmChannelView {
    pub id: String,
    pub realm_id: String,
    pub kind: String,
    pub recipients: Vec<String>,
}

#[derive(Serialize)]
struct OpenDmBody<'a> {
    recipient_id: &'a str,
}

/// 1:1 DM 열기 (find-or-create). 기존 있으면 같은 채널 반환.
pub async fn open_dm(base: &str, token: &str, recipient_id: &str) -> Result<DmChannelView, String> {
    let res = client()
        .post(format!("{base}/users/@me/channels"))
        .bearer_auth(token)
        .json(&OpenDmBody { recipient_id })
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_json(res).await
}

#[derive(Serialize)]
struct OpenGroupBody {
    recipient_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

/// 그룹DM 생성 (호출자 = 소유자).
pub async fn create_group_dm(
    base: &str,
    token: &str,
    recipient_ids: Vec<String>,
    name: Option<String>,
) -> Result<DmChannelView, String> {
    let res = client()
        .post(format!("{base}/users/@me/channels"))
        .bearer_auth(token)
        .json(&OpenGroupBody { recipient_ids, name })
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_json(res).await
}

/// 그룹DM 참가자 추가(소유자). 204.
pub async fn add_recipient(base: &str, token: &str, channel: &str, user: &str) -> Result<(), String> {
    let res = client()
        .put(format!("{base}/channels/{channel}/recipients/{user}"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_or_err(res).await
}

/// 그룹DM 참가자 제거(소유자) 또는 본인 탈퇴(user="@me"는 미지원 — 본인 id 사용). 204.
pub async fn remove_recipient(base: &str, token: &str, channel: &str, user: &str) -> Result<(), String> {
    let res = client()
        .delete(format!("{base}/channels/{channel}/recipients/{user}"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_or_err(res).await
}

#[derive(Deserialize)]
pub struct RelationshipView {
    pub user_id: String,
    pub kind: String,
}

#[derive(Serialize)]
struct PutRelationshipBody<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
}

/// 친구 요청/수락(kind="friend") 또는 차단(kind="block").
pub async fn put_relationship(base: &str, token: &str, user: &str, kind: &str) -> Result<RelationshipView, String> {
    let res = client()
        .put(format!("{base}/users/@me/relationships/{user}"))
        .bearer_auth(token)
        .json(&PutRelationshipBody { kind })
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_json(res).await
}

/// 친구 삭제/요청 취소·거절/차단 해제. 204.
pub async fn delete_relationship(base: &str, token: &str, user: &str) -> Result<(), String> {
    let res = client()
        .delete(format!("{base}/users/@me/relationships/{user}"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_or_err(res).await
}

pub async fn list_relationships(base: &str, token: &str) -> Result<Vec<RelationshipView>, String> {
    let res = client()
        .get(format!("{base}/users/@me/relationships"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_json(res).await
}

#[derive(Deserialize)]
pub struct ReadStateView {
    pub channel_id: String,
    pub last_read_message_id: Option<String>,
    pub mention_count: i32,
}

/// 채널을 메시지까지 읽음 처리(ack).
pub async fn ack(base: &str, token: &str, channel: &str, message: &str) -> Result<ReadStateView, String> {
    let res = client()
        .post(format!("{base}/channels/{channel}/messages/{message}/ack"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_json(res).await
}

pub async fn list_read_states(base: &str, token: &str) -> Result<Vec<ReadStateView>, String> {
    let res = client()
        .get(format!("{base}/users/@me/read-states"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_json(res).await
}

/// 길드 전문검색 (Q10, FTS). VIEW_CHANNEL 있는 채널만 결과에 포함.
pub async fn search_messages(
    base: &str,
    token: &str,
    guild: &str,
    content: &str,
    limit: Option<i64>,
) -> Result<Vec<MessageView>, String> {
    let mut url = format!("{base}/guilds/{guild}/messages/search?content={}", urlencoding::encode(content));
    if let Some(l) = limit {
        url.push_str(&format!("&limit={l}"));
    }
    let res = client().get(url).bearer_auth(token).send().await.map_err(|e| e.to_string())?;
    ok_json(res).await
}

#[derive(Deserialize)]
pub struct AttachmentView {
    pub id: String,
    pub filename: String,
    pub size_bytes: i64,
    pub url: String,
}

/// 메시지에 파일 첨부 (멀티파트 업로드, 작성자 본인).
pub async fn upload_attachment(base: &str, token: &str, channel: &str, message: &str, path: &str) -> Result<AttachmentView, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {path}: {e}"))?;
    let filename = std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .to_owned();
    let part = reqwest::multipart::Part::bytes(bytes).file_name(filename);
    let form = reqwest::multipart::Form::new().part("file", part);
    let res = client()
        .post(format!("{base}/channels/{channel}/messages/{message}/attachments"))
        .bearer_auth(token)
        .multipart(form)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_json(res).await
}

/// 메시지의 첨부 목록.
pub async fn list_attachments(base: &str, token: &str, channel: &str, message: &str) -> Result<Vec<AttachmentView>, String> {
    let res = client()
        .get(format!("{base}/channels/{channel}/messages/{message}/attachments"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_json(res).await
}

/// 첨부 다운로드 → 로컬 파일로 저장.
pub async fn download_attachment(base: &str, token: &str, attachment: &str, out: &str) -> Result<usize, String> {
    let res = client()
        .get(format!("{base}/attachments/{attachment}"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = res.status();
    if !status.is_success() {
        return Err(format!("{status}: {}", res.text().await.unwrap_or_default()));
    }
    let bytes = res.bytes().await.map_err(|e| e.to_string())?;
    std::fs::write(out, &bytes).map_err(|e| format!("write {out}: {e}"))?;
    Ok(bytes.len())
}

#[derive(Deserialize)]
pub struct AuditEntryView {
    pub id: String,
    pub actor_id: Option<String>,
    pub action_type: i16,
    pub target_id: Option<String>,
}

/// 길드 감사 로그 조회 (VIEW_AUDIT_LOG).
pub async fn list_audit(base: &str, token: &str, guild: &str, limit: Option<i64>) -> Result<Vec<AuditEntryView>, String> {
    let mut url = format!("{base}/guilds/{guild}/audit-logs");
    if let Some(l) = limit {
        url.push_str(&format!("?limit={l}"));
    }
    let res = client().get(url).bearer_auth(token).send().await.map_err(|e| e.to_string())?;
    ok_json(res).await
}

#[derive(Deserialize)]
pub struct WebhookView {
    pub id: String,
    pub name: String,
    pub token: Option<String>,
}

/// 웹훅 생성 (MANAGE_WEBHOOKS) → 토큰 1회 반환.
pub async fn create_webhook(base: &str, token: &str, channel: &str, name: &str) -> Result<WebhookView, String> {
    let res = client()
        .post(format!("{base}/channels/{channel}/webhooks"))
        .bearer_auth(token)
        .json(&NameBody { name })
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_json(res).await
}

/// 채널 웹훅 목록.
pub async fn list_webhooks(base: &str, token: &str, channel: &str) -> Result<Vec<WebhookView>, String> {
    let res = client()
        .get(format!("{base}/channels/{channel}/webhooks"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_json(res).await
}

/// 웹훅 삭제 (MANAGE_WEBHOOKS).
pub async fn delete_webhook(base: &str, token: &str, webhook: &str) -> Result<(), String> {
    let res = client()
        .delete(format!("{base}/webhooks/{webhook}"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_or_err(res).await
}

#[derive(Serialize)]
struct ExecuteWebhookBody<'a> {
    content: &'a str,
}

/// 웹훅 실행 (Bearer 없음 — URL 토큰). 채널에 메시지 게시.
pub async fn execute_webhook(base: &str, webhook: &str, wh_token: &str, content: &str) -> Result<(), String> {
    let res = client()
        .post(format!("{base}/webhooks/{webhook}/{wh_token}"))
        .json(&ExecuteWebhookBody { content })
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_or_err(res).await
}

#[derive(Serialize)]
struct SendBody<'a> {
    content: &'a str,
    nonce: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reference_message_id: Option<String>,
}

pub async fn send_message(
    base: &str,
    token: &str,
    channel: &str,
    content: &str,
    nonce: Option<String>,
    reference_message_id: Option<String>,
) -> Result<(), String> {
    let res = client()
        .post(format!("{base}/channels/{channel}/messages"))
        .bearer_auth(token)
        .json(&SendBody { content, nonce, reference_message_id })
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = res.status();
    if status.is_success() {
        Ok(())
    } else {
        Err(format!("{status}: {}", res.text().await.unwrap_or_default()))
    }
}
