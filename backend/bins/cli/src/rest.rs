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

#[derive(Serialize)]
struct RegisterBody<'a> {
    username: &'a str,
    email: &'a str,
    password: &'a str,
}

pub async fn register(base: &str, username: &str, email: &str, password: &str) -> Result<AuthResponse, String> {
    let res = client()
        .post(format!("{base}/auth/register"))
        .json(&RegisterBody { username, email, password })
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

pub async fn login(base: &str, username: &str, password: &str) -> Result<AuthResponse, String> {
    let res = client()
        .post(format!("{base}/auth/login"))
        .json(&LoginBody { username, password })
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

pub async fn join_invite(base: &str, token: &str, code: &str) -> Result<JoinView, String> {
    let res = client()
        .post(format!("{base}/invites/{code}"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ok_json(res).await
}

#[derive(Serialize)]
struct SendBody<'a> {
    content: &'a str,
    nonce: Option<String>,
}

pub async fn send_message(
    base: &str,
    token: &str,
    channel: &str,
    content: &str,
    nonce: Option<String>,
) -> Result<(), String> {
    let res = client()
        .post(format!("{base}/channels/{channel}/messages"))
        .bearer_auth(token)
        .json(&SendBody { content, nonce })
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
