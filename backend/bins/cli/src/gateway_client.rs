//! Gateway(WS) 클라이언트 헬퍼 (개념: gateway_client). connect→IDENTIFY→READY→이벤트 수신.

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

pub type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// http(s) base → ws(s) gateway URL.
fn ws_url(base: &str) -> String {
    let b = base.trim_end_matches('/');
    let b = b.strip_prefix("http://").map(|r| format!("ws://{r}")).unwrap_or_else(|| {
        b.strip_prefix("https://").map(|r| format!("wss://{r}")).unwrap_or_else(|| b.to_string())
    });
    format!("{b}/gateway")
}

/// 연결 → HELLO 수신 → IDENTIFY 전송 → READY 수신. (stream, READY d) 반환.
pub async fn connect_and_identify(base: &str, token: &str) -> Result<(Ws, Value), String> {
    let (mut ws, _) = connect_async(ws_url(base)).await.map_err(|e| format!("connect: {e}"))?;

    // HELLO (op 10) 대기.
    let hello = next_frame(&mut ws).await?.ok_or("connection closed before HELLO")?;
    if hello.get("op").and_then(Value::as_u64) != Some(10) {
        return Err(format!("expected HELLO, got: {hello}"));
    }

    // IDENTIFY (op 2).
    let identify = json!({ "op": 2, "d": { "token": token } });
    ws.send(Message::Text(identify.to_string().into())).await.map_err(|e| format!("send identify: {e}"))?;

    // READY (op 0, t=READY) 또는 INVALID_SESSION(op 9).
    loop {
        let frame = next_frame(&mut ws).await?.ok_or("connection closed before READY")?;
        match frame.get("op").and_then(Value::as_u64) {
            Some(0) if frame.get("t").and_then(Value::as_str) == Some("READY") => {
                return Ok((ws, frame.get("d").cloned().unwrap_or(Value::Null)));
            }
            Some(9) => return Err("INVALID_SESSION (auth failed)".into()),
            _ => {} // 다른 프레임 무시.
        }
    }
}

/// 다음 텍스트 프레임을 JSON으로. 비텍스트/핑은 건너뜀. 종료 시 None.
pub async fn next_frame(ws: &mut Ws) -> Result<Option<Value>, String> {
    while let Some(msg) = ws.next().await {
        match msg.map_err(|e| format!("ws error: {e}"))? {
            Message::Text(t) => {
                return serde_json::from_str::<Value>(t.as_str())
                    .map(Some)
                    .map_err(|e| format!("bad json: {e}"));
            }
            Message::Close(_) => return Ok(None),
            _ => continue,
        }
    }
    Ok(None)
}

/// 연결 후 이벤트를 출력. `seconds` 지정 시 그 시간 후 종료.
pub async fn listen(base: &str, token: &str, seconds: Option<u64>) -> Result<(), String> {
    let (mut ws, ready) = connect_and_identify(base, token).await?;
    println!("✅ READY: {ready}");
    println!("📡 listening for events...");

    let deadline = seconds.map(|s| tokio::time::Instant::now() + std::time::Duration::from_secs(s));
    let mut hb = tokio::time::interval(std::time::Duration::from_secs(30));

    loop {
        let recv = next_frame(&mut ws);
        tokio::select! {
            frame = recv => {
                match frame? {
                    Some(f) => print_event(&f),
                    None => { println!("(connection closed)"); break; }
                }
            }
            _ = hb.tick() => {
                let _ = ws.send(Message::Text(json!({ "op": 1 }).to_string().into())).await;
            }
            _ = sleep_until(deadline), if deadline.is_some() => {
                println!("(listen window elapsed)");
                break;
            }
        }
    }
    Ok(())
}

async fn sleep_until(deadline: Option<tokio::time::Instant>) {
    match deadline {
        Some(d) => tokio::time::sleep_until(d).await,
        None => std::future::pending::<()>().await,
    }
}

fn print_event(f: &Value) {
    match f.get("t").and_then(Value::as_str) {
        Some(t) => println!("📨 {t} (s={}): {}", f.get("s").and_then(Value::as_u64).unwrap_or(0), f.get("d").cloned().unwrap_or(Value::Null)),
        None => {} // 제어 프레임(ACK 등)은 조용히.
    }
}
