//! 헤드리스 종단 시나리오 (개념: scenario). D1/Q9 테스트 하네스.
//!
//! 가입 → 길드/채널 생성 → WS 연결·자동구독(READY) → REST로 메시지 전송 →
//! gateway `MESSAGE_CREATE` 수신까지 자동 검증. PASS/FAIL + exit code.

use std::process::ExitCode;
use std::time::Duration;

use serde_json::Value;

use crate::gateway_client::{connect_and_identify, next_frame};
use crate::rest;

pub async fn run(base: &str, password: &str) -> ExitCode {
    match run_inner(base, password).await {
        Ok(()) => {
            println!("\n✅ SCENARIO PASSED");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("\n❌ SCENARIO FAILED: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn run_inner(base: &str, password: &str) -> Result<(), String> {
    let uname = format!("scn_{}", now_ms());
    let email = format!("{uname}@example.com");

    // 1) 가입.
    let auth = rest::register(base, &uname, &email, password).await?;
    println!("1. registered: user_id={}", auth.user_id);

    // 2) 길드 + 기본 채널.
    let guild = rest::create_guild(base, &auth.access_token, "Scenario Guild").await?;
    let channel = guild.channels.first().ok_or("guild has no default channel")?;
    println!("2. guild={} channel={}", guild.id, channel.id);

    // 3) WS 연결 + IDENTIFY → READY (자동 구독 완료 보장).
    let (mut ws, ready) = connect_and_identify(base, &auth.access_token).await?;
    let ready_realms = ready.get("realms").and_then(Value::as_array).map(|a| a.len()).unwrap_or(0);
    println!("3. gateway READY (구독 realm {ready_realms}개)");
    if ready_realms == 0 {
        return Err("READY에 구독 realm 없음 — 자동구독(D13) 실패".into());
    }

    // 4) REST로 메시지 전송 (nonce로 멱등/대조).
    let nonce = format!("nonce-{}", now_ms());
    let content = "hello from scenario";
    rest::send_message(base, &auth.access_token, &channel.id, content, Some(nonce.clone()), None).await?;
    println!("4. sent message (nonce={nonce})");

    // 5) gateway에서 MESSAGE_CREATE 수신 검증 (5초 타임아웃).
    let got = tokio::time::timeout(Duration::from_secs(5), wait_message_create(&mut ws, content))
        .await
        .map_err(|_| "timeout: MESSAGE_CREATE 미수신 (팬아웃 경로 실패)".to_string())??;
    println!("5. received MESSAGE_CREATE: id={} content={:?}", got.0, got.1);

    Ok(())
}

/// MESSAGE_CREATE 이벤트를 기다려 (id, content) 반환.
async fn wait_message_create(ws: &mut crate::gateway_client::Ws, expect: &str) -> Result<(String, String), String> {
    loop {
        let frame = next_frame(ws).await?.ok_or("connection closed before MESSAGE_CREATE")?;
        if frame.get("t").and_then(Value::as_str) == Some("MESSAGE_CREATE") {
            let d = frame.get("d").cloned().unwrap_or(Value::Null);
            let content = d.get("content").and_then(Value::as_str).unwrap_or_default().to_string();
            let id = d.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
            if content == expect {
                return Ok((id, content));
            }
        }
    }
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
}
