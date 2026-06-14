//! `cli` — 독립 클라이언트 + 헤드리스 테스트 하네스 (서버 내부 무의존). D1/Q9.
//!
//! REST: register/login/refresh/create-guild/send. WS: listen(gateway 이벤트 구독).
//! scenario: 가입→길드→WS구독→전송→MESSAGE_CREATE 수신까지 종단 자동 검증.

mod gateway_client;
mod rest;
mod scenario;

use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(name = "discord-cli", about = "분산 Discord 클론 CLI 클라이언트")]
pub struct Cli {
    /// API base URL (env DISCORD_API 우선).
    #[arg(long, global = true)]
    url: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Register(RegisterArgs),
    Login(LoginArgs),
    Refresh(RefreshArgs),
    /// 길드 생성 (기본 'general' 채널 포함).
    CreateGuild(CreateGuildArgs),
    /// 채널에 메시지 전송 (REST → gateway로 팬아웃).
    Send(SendArgs),
    /// Gateway(WS) 연결 → 이벤트 구독·출력.
    Listen(ListenArgs),
    /// 헤드리스 종단 시나리오 자동 검증 (D1).
    Scenario(ScenarioArgs),
}

#[derive(Args)]
struct RegisterArgs {
    #[arg(long)]
    username: String,
    #[arg(long)]
    email: String,
    #[arg(long)]
    password: String,
}
#[derive(Args)]
struct LoginArgs {
    #[arg(long)]
    username: String,
    #[arg(long)]
    password: String,
}
#[derive(Args)]
struct RefreshArgs {
    #[arg(long)]
    token: String,
}
#[derive(Args)]
struct CreateGuildArgs {
    #[arg(long)]
    token: String,
    #[arg(long)]
    name: String,
}
#[derive(Args)]
struct SendArgs {
    #[arg(long)]
    token: String,
    #[arg(long)]
    channel: String,
    #[arg(long)]
    content: String,
    #[arg(long)]
    nonce: Option<String>,
}
#[derive(Args)]
struct ListenArgs {
    #[arg(long)]
    token: String,
    /// 이 시간(초)만 수신 후 종료 (미지정 시 무한).
    #[arg(long)]
    seconds: Option<u64>,
}
#[derive(Args)]
struct ScenarioArgs {
    /// 테스트 계정 비밀번호.
    #[arg(long, default_value = "scenario_pw_123")]
    password: String,
}

fn base_url(cli: &Cli) -> String {
    cli.url
        .clone()
        .or_else(|| std::env::var("DISCORD_API").ok())
        .unwrap_or_else(|| "http://127.0.0.1:8080".into())
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let base = base_url(&cli);

    let result = match &cli.command {
        Command::Register(a) => rest::register(&base, &a.username, &a.email, &a.password).await.map(print_auth),
        Command::Login(a) => rest::login(&base, &a.username, &a.password).await.map(print_auth),
        Command::Refresh(a) => rest::refresh(&base, &a.token).await.map(print_auth),
        Command::CreateGuild(a) => rest::create_guild(&base, &a.token, &a.name).await.map(|g| {
            println!("✅ guild created");
            println!("  id       = {}", g.id);
            println!("  name     = {}", g.name);
            for c in &g.channels {
                println!("  channel  = {} ({})", c.id, c.name.clone().unwrap_or_default());
            }
        }),
        Command::Send(a) => rest::send_message(&base, &a.token, &a.channel, &a.content, a.nonce.clone())
            .await
            .map(|_| println!("✅ queued (will arrive via gateway MESSAGE_CREATE)")),
        Command::Listen(a) => gateway_client::listen(&base, &a.token, a.seconds).await,
        Command::Scenario(a) => return scenario::run(&base, &a.password).await,
    };

    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("❌ {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn print_auth(a: rest::AuthResponse) {
    println!("✅ ok");
    println!("  user_id      = {}", a.user_id);
    println!("  access_token = {}", a.access_token);
    println!("  refresh_token= {}", a.refresh_token);
}
