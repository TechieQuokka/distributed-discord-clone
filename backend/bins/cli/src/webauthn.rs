//! WebAuthn/Passkeys 데모 (개념: webauthn). D19. SoftPasskey로 register→**암호 없는 login**을 헤드리스 실행.
//! 브라우저/실물 인증기 없이 ceremony를 in-process로 구동해 라이브 검증한다.

use auth::webauthn::{CreationChallengeResponse, RequestChallengeResponse, Url};
use serde_json::{Value, json};
use webauthn_authenticator_rs::WebauthnAuthenticator;
use webauthn_authenticator_rs::softpasskey::SoftPasskey;

pub async fn demo(base: &str, username: &str, password: &str) -> Result<(), String> {
    let cl = reqwest::Client::new();
    // 1) 일반 가입(PoW 자동 풀이) → access 토큰.
    let auth = crate::rest::register(base, username, &format!("{username}@e.com"), password).await?;
    println!("1. registered + token (user_id={})", auth.user_id);

    let origin = Url::parse(base).map_err(|e| format!("origin parse: {e}"))?;
    let mut wa = WebauthnAuthenticator::new(SoftPasskey::new(true));

    // 2) 등록 ceremony: start → SoftPasskey 자격증명 생성 → finish.
    let v = post(&cl, base, "/auth/webauthn/register/start", Some(&auth.access_token), &json!({})).await?;
    let cid = v["ceremony_id"].as_str().ok_or("no ceremony_id")?.to_string();
    let ccr: CreationChallengeResponse =
        serde_json::from_value(v["options"].clone()).map_err(|e| format!("ccr decode: {e}"))?;
    let rpkc = wa.do_registration(origin.clone(), ccr).map_err(|e| format!("soft register: {e:?}"))?;
    post_204(
        &cl,
        base,
        "/auth/webauthn/register/finish",
        Some(&auth.access_token),
        &json!({ "ceremony_id": cid, "credential": rpkc }),
    )
    .await?;
    println!("2. passkey registered");

    // 3) 암호 없는 로그인: start → SoftPasskey 서명 → finish → 토큰.
    let v = post(&cl, base, "/auth/webauthn/login/start", None, &json!({ "username": username })).await?;
    let cid = v["ceremony_id"].as_str().ok_or("no ceremony_id")?.to_string();
    let rcr: RequestChallengeResponse =
        serde_json::from_value(v["options"].clone()).map_err(|e| format!("rcr decode: {e}"))?;
    let pkc = wa.do_authentication(origin, rcr).map_err(|e| format!("soft auth: {e:?}"))?;
    let v = post(&cl, base, "/auth/webauthn/login/finish", None, &json!({ "ceremony_id": cid, "credential": pkc })).await?;
    let tok = v["access_token"].as_str().ok_or("no access_token in login finish")?;
    println!("3. passwordless login OK — access token issued (len {})", tok.len());
    Ok(())
}

async fn post(
    cl: &reqwest::Client,
    base: &str,
    path: &str,
    token: Option<&str>,
    body: &Value,
) -> Result<Value, String> {
    let mut r = cl.post(format!("{base}{path}")).json(body);
    if let Some(t) = token {
        r = r.bearer_auth(t);
    }
    let res = r.send().await.map_err(|e| e.to_string())?;
    let st = res.status();
    let txt = res.text().await.unwrap_or_default();
    if !st.is_success() {
        return Err(format!("{path} {st}: {txt}"));
    }
    Ok(serde_json::from_str(&txt).unwrap_or(Value::Null))
}

async fn post_204(
    cl: &reqwest::Client,
    base: &str,
    path: &str,
    token: Option<&str>,
    body: &Value,
) -> Result<(), String> {
    let mut r = cl.post(format!("{base}{path}")).json(body);
    if let Some(t) = token {
        r = r.bearer_auth(t);
    }
    let res = r.send().await.map_err(|e| e.to_string())?;
    let st = res.status();
    if !st.is_success() {
        return Err(format!("{path} {st}: {}", res.text().await.unwrap_or_default()));
    }
    Ok(())
}
