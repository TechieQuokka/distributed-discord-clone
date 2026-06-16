# Architecture Decision Log

> Discord 클론 — 공부 + 포트폴리오용. 로컬 전용, 이론 확립.
> 철학: **"공부는 실전처럼"** — 실제로 수만 명을 붙이진 않지만, *수만 동접을 감당할 수 있는 구조*로 설계하고 로컬 시뮬레이션으로 증명한다.
>
> 최종 갱신: 2026-06-16 (**Phase 5 대거 진행, v1.45**: SWIM D45/D46 · WebAuthn D19 · **이벤트 소싱 D48**(가산형 CQRS) · **CRDT 오프라인 동기화 D49**(상태기반 CvRDT) · **Voice 시그널링 설계 D47**(미디어 제외) · 하드닝(idle/dnd op3·신규 월 파티션 사전생성). MinIO는 범위 제외(D37, 로컬 테스트 전용). 잔여=세부 하드닝·크로스노드 RESUME·액터 supervisor)

---

## 0. 목표 & 제약 (Goals & Constraints)

| 항목 | 내용 |
|---|---|
| 목적 | 공부 + 포트폴리오 (실운영 X) |
| 실행 환경 | 전부 로컬 테스트 범주 |
| 규모 목표 | **수만 동접을 감당하는 구조** (다중 노드 전제) |
| 검증 방법 | 로컬에서 수십 개 테스트 클라이언트 구동 (일부는 Claude가 직접 구동) |
| 핵심 테마 | **분산 인프라 정공법** (소셜/그래프 기능 아닌 인프라 깊이로 독창성 어필) |
| 기능 범위 | Discord 기능 *가능한 전부* 카피 (단, 과하지 않게 — 컷라인은 열린 질문) |

---

## 1. 핵심 원칙 (Principles) — 지난 실패에서 도출

> **지난 프로젝트 실패 원인: 코드가 내부적으로 꼬임 (경계 부재).**
> 아래 원칙은 그걸 구조적으로 막기 위함.

- **P1. 청사진 우선** — 코드 전에 합의된 청사진(이 문서)부터.
- **P2. 경계는 trait 뒤에 가둔다** — 알고리즘/전송 구현은 인터페이스 뒤에 숨기고, 코어 로직은 구현체를 모른다.
- **P3. stub 우선** — 어려운 구현(raw TCP 등)은 in-process stub으로 먼저 굴리고 기능부터. 실제 구현체는 나중에 교체.
- **P4. 특수 케이스를 추상화로 흡수** — 예: DM/길드/그룹DM을 따로 두지 않고 `Realm` 하나로 통일.
- **P5. 상태는 가둔다** — 공유 가변 상태 + 락 패턴 회피. 상태는 액터 안에 격리.
- **P6. 암호화는 직접 짜지 않는다** — "수제"는 전송 프로토콜·액터 런타임까지만. crypto 프리미티브(서명/해싱/TLS)는 **검증된 크레이트**(argon2, rustls, paseto/ed25519-dalek) 사용. 직접 짠 암호 = 뚫림.

---

## 2. 기술 스택 (Stack)

| 레이어 | 선택 | 비고 |
|---|---|---|
| Backend | **Rust** | |
| Async 런타임 | **tokio** | |
| Frontend | **Web (TS)** | 프레임워크 미정 (열린 질문) |
| DB | **PostgreSQL** | 이미 설치됨. ⚠️ 비번은 `.env`/환경변수로 분리할 것 |
| 메시지 브로커 | **없음** | Redis/NATS 미사용. 노드 간 직접 통신으로 대체 |

---

## 3. 확정된 결정 (Decisions)

### D1. API + CLI, 그리고 CLI의 정체
- Backend는 **REST API**와 **CLI**를 모두 지원.
- **CLI는 backend 내부가 아니라, API를 소비하는 독립 클라이언트.**
- CLI는 **테스트 하네스(부하 생성기 + 검증기)** 역할을 겸한다.
- 따라서 CLI는 대화형뿐 아니라 **비대화형(헤드리스) 모드** 필수 — 스크립트로 "로그인 → 입장 → N개 전송 → 수신 검증" 자동 실행 가능해야 함.

### D2. 엣지 프로토콜 = WebSocket (강제)
- 클라이언트 ↔ Gateway = **WebSocket**.
- 이유: 프론트가 web → **브라우저는 raw TCP를 못 연다.**
- CLI 클라이언트도 일관성을 위해 WebSocket 사용.

### D3. 노드 간 통신 = raw TCP + 자체 프로토콜
- Gateway 노드 ↔ Gateway 노드 = **raw TCP**.
- **자체 수제 바이트 레이아웃** 직렬화 (bincode/protobuf 아님, 직접 `[u8]` 깎기).
- **길이 접두사(length-prefix) 프레이밍** (4바이트 길이 + 페이로드) — TCP 스트림의 메시지 경계 문제 해결.

### D4. 토폴로지 = 풀 메시 (Full Mesh)
- 모든 노드가 서로 직접 연결. 노드 N개 → N(N-1)/2 링크.
- 로컬 노드 수(3~10개) 규모에서 가장 단순하고 충분.

### D5. 노드 발견 = 정적 config (v1) → SWIM 동적 합류 (Phase 5)
- v1: 노드 주소 목록을 설정 파일에 정적 정의.
- **확장됨(Phase 5, D45)**: gossip(SWIM)으로 **동적 합류**. config의 peers는 이제 **seed(introducer) 목록**으로 충분 — 신규 노드가 seed에 합류 요청하면 SWIM이 전 노드에 감염 전파하고 멤버가 서로를 런타임 dial한다. 정적 전체목록 경로도 fallback으로 유지(D45).

### D6. 노드 배치 = Consistent Hashing
- Realm을 노드에 배치하는 데 consistent hashing 사용.
- 노드 추가/삭제 시 재배치 최소화. **이 프로젝트 독창성의 중심축.**

### D7. 동시성 모델 = Actor (tokio + mpsc 수제)
- **각 Realm = 액터 1개.** 상태를 독점, 락 없음, 메시지로만 소통.
- 구현: `struct XxxActor` + `enum XxxMsg` + `mpsc::channel`(메일박스) + `while let Some(msg) = rx.recv().await` 루프.
- 외부 크레이트(`ractor` 등) 미사용 — 수제.
- supervisor/재시작 전략은 직접 설계 (열린 질문).

### D8. 샤딩 단위 = Realm (통일 추상)
- **Realm = "메시지가 오가는 격리된 컨테이너"** 의 1급 추상.
  - 길드 = Realm
  - DM(1:1) = Realm (※ **재조정**: id는 합성키가 아니라 정상 Snowflake. 중복 방지는 `dm_pairs(user_lo,user_hi UNIQUE)→realm_id` 조회로. [database/01-overview.md DB-D2] 참조)
  - 그룹 DM = Realm (자체 id)
- 모든 Realm은 액터 1개. 소유 노드 = `hash(realm_id)`.
- → DM 특수 케이스가 사라짐. 라우팅은 "Realm에게 보내기" 하나로 통일.
- **구현(Phase 3, DM/그룹DM)**: 통일 추상의 배당이 실제로 발현 — DM·그룹DM도 `realms`+`channels`(+`members`)라 **메시징·권한·분산 팬아웃·자동구독(D13)을 길드와 무변경 공유**(gateway/node/protocol/server 코드 추가 0). DM Realm은 @everyone 역할이 없어 권한 계산이 `default_everyone`으로 폴백 → 멤버면 전송·조회가 길드와 같은 경로로 통과(perm `effective`/`can_send`의 기존 폴백). 1:1 DM 중복 방지는 `dm_pairs`(DB-D2) find-or-create(같은 두 사람 = 같은 채널). 그룹DM은 자체 realm(kind=group_dm, owner_id) + 참가자 관리(`PUT`/`DELETE /channels/:id/recipients/:uid`, 소유자만 추가·타인제거 / 본인 탈퇴, 소유자 탈퇴 불가). 멤버 추가/제거는 `GuildRepository::{add_member,remove_member}` 재사용. 신규 포트 `DmRepository`(find_dm/create_dm/create_group_dm/get_realm), wire **V9 `dm_pairs`**. 실시간 통지는 `CHANNEL_RECIPIENT_ADD/_REMOVE`(D39 envelope·RealmEmitter 재사용). 1:1 차단(blocked) 거부는 relationships 도입 후 seam(permissions.md §5).

### D9. 2단 소유권 (Two-tier Ownership)
- **세션 소유**: WS 연결을 들고 있는 노드 = 클라이언트가 *접속한* 노드.
- **Realm 소유**: Realm 상태의 권위 노드 = `hash(realm_id)`.
- 메시지 라우팅 흐름:
  ```
  노드A의 유저가 RealmR에 메시지 전송
    → RealmR 소유 노드 = hash(R) = 노드B
    → A가 raw TCP로 B에 전달
    → B(RealmR 액터)가 검증·저장·팬아웃 대상 계산
    → 구독자 세션은 여러 노드에 흩어짐
    → B가 각 구독자가 붙은 노드로 push → 그 노드가 로컬 WS로 전달
  ```

### D10. 전송 계층 격리 (P2/P3 적용)
- 노드 간 전송은 `trait` 뒤에 둔다 (예: `trait NodeTransport`).
- v1 = in-process stub 구현체로 시작 → 기능 먼저.
- raw TCP 구현체는 나중에 끼운다.

### D11. ID = Snowflake
- 64bit: 타임스탬프 + 워커ID + 시퀀스. 시간순 정렬 가능. 레이아웃 `[timestamp 41 | worker 10 | sequence 12]`.
- **핵심 불변식: `1 worker_id = 1 sequence 카운터`.** Snowflake 전역 유일성은 전적으로 여기 의존한다. worker_id는 노드당 1개(D29)이므로 **generator도 노드당 정확히 1개**여야 한다.
- **정정(2026-06-14):** 초기 구현은 Realm 액터마다 `SnowflakeGenerator`를 따로 생성해 같은 노드의 두 Realm이 동일 worker_id·동일 ms에 동일 ID를 발급할 수 있었다(불변식 위반). → **노드당 단일 generator**를 만들어 모든 Realm 액터에 **주입**(`Arc<SnowflakeGenerator>`)한다. RealmActor는 worker_id를 직접 들지 않고 주입된 generator의 `next()`만 호출.
- generator는 동시 접근되므로 **thread-safe & lock-free**: 가변 상태 `(last_ms, sequence)`를 `AtomicU64` 하나에 packing(41+12비트)하고 CAS 루프로 갱신(`&self next()`). worker는 불변이라 atomic 밖. 핫패스(메시지 발급)에 직렬화 액터 홉을 만들지 않는다(수만 동접 목표). Twitter Snowflake 원조와 동일하게 "worker당 generator 1개 + 동시 접근".
- `Clock`(D25)과 같은 **노드 레벨 시간 프리미티브**로 취급 → 같은 주입 패턴. 도메인/Realm 상태가 아니므로 P5(상태=액터)의 예외가 정당.

### D12. 팬아웃 위치추적 = Realm-local
- 유저가 Realm에 입장/구독하면 그 사실이 Realm 소유 노드로 전파 → **Realm 액터가 `{user_id → node_id}` 구독자 표를 자체 보유**.
- 팬아웃 시 Realm 액터는 대상 노드를 이미 알고 있음 → **전역 조회 0번.**
- 전역 세션 레지스트리(모든 노드가 풀맵 보유) 방식 기각, 유저-해시 조회 방식도 기각(RTT 폭증).
- **"팬아웃 위치추적"과 "전역 presence"는 분리한다.** presence(친구 온라인 여부 등)는 Realm 무관 전역 상태 → 별도 메커니즘(gossip 등, Q2 하위 갈래로 미정).

### D13. 구독 모델 = 입장 시 자동 구독 (Discord식)
- Discord Gateway처럼, Realm(특히 길드)에 입장하면 해당 Realm 이벤트를 자동 구독에 가깝게 처리.

---

## 3-S. 보안 결정 (Security) — P6 전제 위에서

### D14. 사용자 인증 = 2-토큰 패턴
- **access token = PASETO v4.public (Ed25519 서명), 짧은 수명(~15분), 무상태.**
  - 노드는 **공개키로 검증만** → 시크릿 공유 불필요 (분산에 이상적).
  - JWT 대신 PASETO 선택: `alg` 혼동 공격 등 JWT 함정 원천 차단.
- **refresh token = opaque 랜덤(256-bit CSPRNG), Postgres 저장 → 즉시 폐기 가능.**
  - **회전(rotation) + 재사용 탐지(reuse detection)** — 탈취 감지.

### D15. 비밀번호 해싱 = Argon2id
- bcrypt/PBKDF2 아님. `argon2` 크레이트.

### D16. 전송 암호화 = TLS 1.3 (rustls)
- 클라↔Gateway = `wss://` (로컬 self-signed).
- 노드↔노드 = **mTLS** (D3 raw TCP 위에 TLS 래핑, 로컬 self-signed CA + 노드별 cert).
- ※ mTLS가 채널을 인증·암호화하므로 프레임별 HMAC은 불필요.
- **구현(Phase 2)**: `transport::tcp::TcpTransport` — rustls 서버측 `WebPkiClientVerifier`(클라 cert를 CA로 검증) + 클라측 `with_client_auth_cert`(상호 인증). 공유 CA로 노드 cert 발급(`generate_mesh`, dev/test). 작은 id가 큰 id에게 dial(D4 §6), HELLO 교환으로 피어 식별(헤더 src_node_id). 끊기면 dial 측 재연결(backoff). 신뢰 안 되는 CA의 피어는 핸드셰이크 거부(테스트 검증).
- **멀티노드 PASETO 키 공유(D14 연계)**: 노드마다 키가 다르면 한 노드가 발급한 access 토큰을 다른 노드가 검증 못 함 → **모든 노드가 같은 PASETO 키 공유** 필수(env `PASETO_SECRET`/`PASETO_PUBLIC`, `auth::TokenKeys::export_hex`/`import_hex`).

### D17. 인가 = Discord 권한 비트마스크
- 계산순서 그대로: `@everyone` → 역할 OR → 채널 오버라이드(deny/allow) → 멤버 오버라이드 → Administrator 통과. 채널마다 재계산.
- **구현(Phase 3)**: 비트(`domain::permissions::Permissions`)·계산(`compute_guild_permissions`/`compute_channel_permissions`)은 domain(순수). 저장 = `roles`/`member_roles`(V5), `@everyone`=`roles.id==realm_id`(길드 생성 시 `default_everyone` 권한으로 자동). `RoleRepository`(create/list/assign/everyone_perms/member_role_perms). 강제는 어댑터에서 DB 데이터를 모아 domain 함수로 계산 후 검사: rest-api `perm::require`(채널생성 MANAGE_CHANNELS·초대 CREATE_INVITE·역할관리 MANAGE_ROLES, 권한상승 방지), gateway `can_send`(SEND_MESSAGES). owner/Admin 단축.
- **채널 오버라이드(Phase 3, 구현됨)**: `channel_overwrites`(V6, `overwrite_kind` enum) + `ChannelOverwriteRepository`(set/list) + `member_roles_with_ids`. `domain::permissions::effective_channel_permissions`가 오버라이드 목록에서 대상별(@everyone=realm_id / 역할 / 멤버)로 골라 `compute_channel_permissions`에 적용(deny→allow, 멤버 최우선). REST `PUT /channels/:id/permissions/:target_id`(MANAGE_ROLES). `can_send`/`perm::effective_in_channel`는 채널 컨텍스트로 계산 → 채널 deny가 길드 허용을 덮어씀. 라이브 검증: @everyone deny SEND → 차단, 멤버 allow → 복구.

### D18. 봇방지 = PoW 챌린지 + Rate limit
- 가입/로그인 시 **Proof-of-Work 해시 퍼즐**(hashcash/mCaptcha/Anubis 스타일) — 제3자 의존 0, 수제 구현.
- **Rate limiting = Token Bucket** (per-route/per-user/global). ← 기존 Q5 흡수.
- **PoW 구현(Phase 4)**: `auth::pow`. **stateless 멀티노드** — 챌린지를 서버에 저장하지 않고(DB-D5 휘발) **PASETO v4.local 토큰**(대칭 인증·암호화 + 만료 내장)으로 발급, 난이도를 인증된 claim(`sub`)에 담아 위변조 차단. 클라는 `sha256(challenge || ":" || nonce)`의 **선행 0비트 ≥ 난이도**가 되는 nonce를 찾아 제출. 서버는 토큰 진위·만료 + (토큰에서 디코드한 인증된 난이도로) 해 검증. **퍼즐 해시만 수제(sha2), 챌린지 MAC은 검증 크레이트(pasetors)** — 수제 암호 금지(P6) 준수. 멀티노드는 `POW_SECRET` 공유(D14와 동일 철학, `server gen-keys`가 발급). `GET /auth/pow-challenge` 발급 → `POST /auth/register`가 `pow_challenge`+`pow_nonce` 필수 검증(실패 400). **seam**: 챌린지 저장 안 함 → 만료(PASETO exp 기본 1h)까지 같은 해 replay 가능(비용 게이트는 난이도). rate limit(D32)·로그인 PoW는 후속.

### D19. MFA = TOTP 코어 + Passkeys 스트레치
- **TOTP (RFC 6238)** 우선 구현.
- **WebAuthn/Passkeys** 는 여유 시 스트레치 목표.
- **구현(Phase 4)**: `auth::totp`(검증 크레이트 `totp-rs`, P6 — SHA1·6자리·30s·skew1). secret(raw)은 `users.mfa_totp_secret`(BYTEA, V1에 이미 존재 → 마이그레이션 0), 민감값이라 `User` 엔티티엔 안 싣고 전용 포트(`UserRepository::{set_totp_secret, totp_secret}`). **흐름**: `POST /auth/mfa/totp/enable`(secret+otpauth URI 발급, **미저장**) → `verify`(secret+code 확인 시 저장=활성, **락아웃 방지** — 미확인 secret로 안 잠김) → `disable`(코드 확인 후 제거). 로그인은 MFA 활성 유저면 토큰 대신 `{mfa_required:true}` → `POST /auth/mfa/totp`(비번 재확인 + 코드)로 토큰 발급. 시각은 주입 clock(now_unix). cli `mfa-enable/mfa-verify/mfa-login/totp-code`(코드 생성=인증앱 대역). **라이브 e2e 검증**(enable→verify→login mfa_required→2단계 토큰, 틀린 코드 401). seam: 로그인 2단계는 비번 재제출(ticket 미사용) · 백업코드(`mfa_backup_codes`)는 후속.
- **구현(Phase 5, WebAuthn/Passkeys)**: 공개키 자격증명(FIDO2)으로 **암호 없는 로그인**. **P6 — 크립토/검증 로직은 직접 안 짠다, 검증 크레이트 `webauthn-rs`(0.5) 사용**(어테스테이션·서명·challenge·counter 전부 위임). 헤드리스 내부검증은 `webauthn-authenticator-rs`의 SoftPasskey로 in-process ceremony 구동(브라우저/실물 인증기 불필요).
  - **저장**: 자격증명은 `webauthn_credentials`(V18) — `credential_id BYTEA UNIQUE`(exclude/조회) + **`passkey JSONB`**(webauthn-rs `Passkey` 직렬화, 공개키+counter 내장). 원안의 분해 컬럼(public_key/sign_count) 대신 라이브러리 단위(Passkey)를 불투명 저장(02-schema 갱신). domain은 **opaque `passkey_json` 문자열**로만 알아 webauthn-rs 무의존(P2). 신규 포트 `WebAuthnRepository`(add/list/update_counter) → Store 합류.
  - **ceremony 상태**: register/auth의 `PasskeyRegistration`/`PasskeyAuthentication` 중간 상태는 **휘발(DB-D5)** — rest-api `AppState`의 인메모리 맵(`ceremony_id → 상태 + 만료`)에 보관. 서버측 보관(클라 위변조 차단). **멀티노드 seam**: finish는 start한 같은 노드로(상태 노드-로컬, 크로스노드 RESUME과 동류 seam).
  - **흐름**: 등록(인증된 유저) `POST /auth/webauthn/register/{start,finish}` → finish 시 Passkey 저장. 로그인 `POST /auth/webauthn/login/{start,finish}`(start는 username으로 등록 자격증명 로드, finish는 서명 검증 후 **access+refresh 발급** = 암호 없는 로그인). counter 증가 시 재저장(클론 탐지는 라이브러리). RP = `WEBAUTHN_RP_ID`/`WEBAUTHN_RP_ORIGIN` env(기본 localhost). webauthn 미설정 노드는 404(기능 비활성).
  - **검증**: auth SoftPasskey 통합 테스트(register→auth 서명 검증 라운드트립) + rest-api MemStore 엔드포인트 + cli `webauthn-register`/`webauthn-login` 라이브 e2e. seam: usernameless(discoverable) 로그인·어테스테이션 정책(현재 None)·멀티노드 ceremony 공유는 후속.

### D20. 입력·세션 하이젠
- **SQL = 파라미터 바인딩만** (문자열 조립 금지).
- **Gateway RESUME 토큰 = CSPRNG 추측불가 + 검증.**
- 프론트 = **CSP** 등 보안 헤더.
- 시크릿(DB 비번 등) = `.env`/환경변수 (코드 하드코딩 금지).

### D21. Voice/Video = 범위 제외 (시그널링만 설계)
- 음성/영상 미디어(WebRTC/SFU/코덱/UDP·SRTP)는 **out**. 테마(분산 텍스트 인프라)와 결이 다르고 비용 과대.
- 시그널링(Gateway 통과 부분)은 설계만. 실제 미디어는 먼 스트레치(Phase 5).
- **시그널링 설계 완료(Phase 5, D47)** → [protocol/voice-signaling.md](../protocol/voice-signaling.md).

---

## 3-D. 구조·운영 결정 (Structure & Operations)

### D22. Crate 경계 = 헥사고날 계층형 (P2의 실체)
- 모듈이 아니라 **crate로 쪼개 컴파일러가 경계를 강제.** 의존성은 **안쪽(domain)으로만**.
```
[bin] server  ── gateway + rest-api + node 조립, 실행 진입점
[bin] cli     ── 독립 클라이언트 + 테스트 하네스 (⚠ server 내부 의존 금지)

  gateway     ── WS 서버, 세션, RESUME, 클라 팬아웃
  rest-api    ── HTTP REST
  node        ── 액터 배선 + consistent hashing + 2단 라우팅
  storage     ── Postgres 리포지토리 (domain의 port를 구현하는 adapter)
  transport   ── trait NodeTransport + stub/raw-TCP-mTLS 구현 (domain 모름)
  actor-rt    ── 수제 액터 런타임 (도메인 무지, 범용)
  protocol    ── 와이어 타입 + 수제 바이트 인코딩/프레이밍
  domain  ◀── 중심. 엔티티+권한계산+순수로직, IO 없음, 무의존
```
- **리포지토리는 domain에 trait(port)로 선언, storage가 구현(adapter).** → "domain이 Postgres를 import" 컴파일 차단.
- ⚠ **개정 (구현 시)**: umbrella Cargo 워크스페이스를 쓰지 않는다. 각 crate는 **독립 패키지**(자체 Cargo.toml·target·버전), 개별 빌드/관리 (CLAUDE.md R7). 최상위는 `backend/`·`frontend/` 분리. crate 내부는 개념별 디렉터리(R6).

### D23. 상태/복구 = Postgres 진실 + 액터 캐시
- **Postgres = 진실의 원천. Realm 액터 인메모리 상태 = 재구축 가능 캐시(write-through).** 메시지는 append-only 저장.
- 노드 사망 → consistent hashing 재배치 → 새 노드가 Postgres에서 **rehydrate** → 죽은 노드 세션 끊김 → 클라가 다른 노드로 RESUME 재연결.
- 이벤트 소싱은 Phase 5 스트레치 후보. **→ 가산형으로 구현됨(D48, v1.44)**: messages(진실)를 대체하지 않고 그 위에 append-only 이벤트 로그(`realm_events`) + 순수 프로젝션을 얹음(CQRS). rehydrate 재생은 `RealmProjection`.
- **구현(Phase 2)**: `node::membership::Membership` + `run_failure_detector` — 피어에 주기 PING(1s),
  PONG/임의 트래픽 수신 시 `record_seen`, `timeout`(3s) 초과 시 down. `Router::owner`는 `HashRing::owner_excluding`로
  **down 노드를 건너뛴 일관 해싱 소유권** → 소유 노드가 죽으면 다음 살아있는 노드가 그 Realm을 자동 소유(failover).
  새 소유 노드는 첫 트래픽에서 RealmActor를 **새로 spawn**(lazy)한다 — 현재 액터 상태는 휘발성 구독자표(D12, DB-D5)
  뿐이라 rehydrate = fresh-spawn + 클라 재연결 시 자동 재구독(D13)으로 충분. **메시지 진실은 Postgres에 persist-then-fanout(D24)로
  이미 보존**되어 유실 없음(히스토리는 REST 조회). **D35 최근메시지 캐시 warmup**(Postgres 적재)은 캐시 도입 시 같은 자리에 들어갈 후속 seam.
  노드 복귀(record_seen) 시 소유권 환원(re-join). ※ DST(D25)에선 주입 clock 기준 결정론 재현.

### D24. 전달/순서 보장
- **순서**: Realm 액터 단일 소유 → Realm 내 전순서 무료 보장. 전역 순서는 미보장.
- **저장→팬아웃 (persist-then-fanout)**: Postgres 먼저, 그다음 팬아웃 (유령 메시지 방지).
- **전달**: 이벤트마다 per-session 시퀀스 번호. 세션 노드에 bounded 재생 버퍼 → RESUME 재생. 버퍼 초과 → **REST 재조회**로 동기화.
- **구현(persist 위치)**: Realm 액터는 ID·순서만 확정해 이벤트를 방출하고(코어는 IO 무의존, P2),
  **단일 소비자인 dispatch 드라이버**(gateway)가 events 채널을 받아 `persist → fanout → 세션 배달` 순서로 처리한다.
  단일 액터가 순서대로 방출 → 단일 소비자가 순서대로 persist → 순서 보존. nonce 중복이면 persist 단계에서 멱등 스킵(D34).
- **현황(Phase 2, 구현됨)**: per-session seq + bounded 재생 버퍼(기본 256)를 **Hub**(세션 소유 노드)가 보유.
  소켓 끊김 = `detach`(live sender만 분리, 버퍼·구독·seq 유지, grace 90s) → RESUME이 `resume_token`(CSPRNG, D20) +
  last seq 검증 후 누락 프레임 재생 + `RESUMED`(t="RESUMED" dispatch). 버퍼 밖 gap·토큰 불일치·만료는 INVALID_SESSION
  → 재IDENTIFY + REST 재조회. RESUME은 버퍼가 노드 로컬이라 **동일 노드** 재연결에 한함(크로스노드 RESUME은 후속).

### D25. 테스트 = 유닛 + DST + e2e
- **결정론적 시뮬레이션(DST)**: 가상 시계 + 시뮬레이션 네트워크 + 시드 RNG로 클러스터를 단일 프로세스에서 재현 가능 실행, 카오스(지연/유실/파티션) 주입.
- `transport` trait + `actor-rt` 덕에 `SimTransport`+`SimClock` 주입만으로 가능. Phase 2부터.
- + 유닛 테스트 + 소수 실프로세스 e2e (WS/Postgres 엣지).
- **구현(Phase 2)**: `transport::sim` — `SimNetwork`(가상 시계 + `BinaryHeap` 시간순 스케줄 + 노드별 ready 큐) +
  `SimTransport`(`NodeTransport` 구현, `send`는 즉시 큐 적재) + `DetRng`(splitmix64 시드 PRNG). 카오스 주입:
  지연(`min/max_latency_ms`), 유실(`drop_prob`), 파티션(`partition`/`heal`). 하네스가 `advance_to`로 가상 시간을
  진행시키면 그 시점까지 도착할 메시지를 `take_inbound`로 꺼내 `Router::handle_inbound`에 먹인다. SimClock =
  `node::ManualClock`. `Router`/`RealmActor`는 이제 `Arc<dyn Clock>`를 주입받아(하드코딩 SystemClock 제거) DST에서
  Snowflake id까지 결정론. 하네스 e2e(`node/tests/dst.rs`): 동일 시드 재현성 + 파티션 유실 검증.
  후속: 액터까지 단일스레드 가상 실행기로 돌리는 완전 결정론 클러스터(현재 액터는 tokio, 네트워크 경로만 가상시간).

### D26. 관찰성 = tracing
- `tracing` 크레이트 + 구조적 span + **노드 간 trace-id 전파**(프로토콜 헤더). (선택) OTel→Jaeger. **Phase 0부터.**

### D27. Backpressure = bounded everywhere
- 모든 메일박스/채널 bounded. 느린 WS 클라 → 버퍼 차면 연결 끊기(클라는 재연결+RESUME). 노드 간 → bounded + 백프레셔, drop 시 resync.
- **구현(Phase 2)**: WS 세션 채널 bounded(기본 256). `Hub::push_live`가 `try_send` 실패(느린 클라로 채널 가득)
  시 **live sender를 drop** → 세션 채널이 닫혀 `pump` 루프 종료·소켓 close = 끊김. 프레임은 재생 버퍼에 남아
  클라가 재연결+RESUME으로 복구(D24). 노드↔노드는 `TcpTransport`의 피어 writer 채널 bounded(256) + `send().await`로
  자연 백프레셔(드롭 아님). 액터 메일박스도 bounded(`spawn(actor, 256)`).

### D28. DB 접근 = sqlx + 메시지 RANGE 파티셔닝
- `sqlx` (async + **컴파일타임 쿼리 검증** → 파라미터 바인딩 강제, D20과 일석이조). 마이그레이션 Phase 0부터.
- 메시지 파티셔닝: Phase 1~3 단일 테이블 + 인덱스 `(channel_id, id DESC)`. **Phase 4에서 Snowflake 시간 RANGE 파티셔닝**(월별).
- **구현(Phase 4, v1.38, V17)**: `messages`를 `PARTITION BY RANGE (id)`로 드롭&재생성(로컬 데이터 폐기). 월별 파티션(id 경계 = `(month_start_ms - EPOCH_MS) << 22`) + `messages_default` 캐치올. 인덱스(`ix_messages_channel`, FTS GIN)는 부모에 정의→파티션 상속.
- **신규 월 파티션 사전 생성(구현됨 v1.43, V19, Phase 5 하드닝)**: ~~운영 작업(현재 DEFAULT 흡수)~~ → 멱등 plpgsql `ensure_message_partitions(months_ahead)`(달력 계산을 Postgres에 위임, 앱에 날짜 라이브러리 무도입)가 `(month_start_ms - EPOCH_MS) << 22` 경계로 다가오는 달 파티션을 `to_regclass` 가드로 생성. server가 startup에 `ensure_message_partitions(2)` 호출(이번 달 + 2개월). 경계가 V17과 연속임을 라이브 검증. 04 §6.
- **nonce 멱등(D34)과의 충돌 해소**: Postgres는 파티션 테이블 유니크 인덱스에 파티션 키(id) 포함을 강제 → `uq(channel,author,nonce)` 불가 → **앱레벨 dedup으로 이전**(D34 참조). **첨부 FK는 유지**(PG 12+가 파티션 부모 참조 FK+CASCADE 지원 → 04 §2의 앱레벨 완화(a)는 불필요).

### D29. 멀티노드 로컬 실행 = dev 런처
- `cargo xtask`/`just` 레시피가 클러스터 config(D5 정적 노드목록) 읽어 N노드 기동. Postgres는 기존 로컬.
- 각 노드는 클러스터 config에서 **고유 worker-id**(Snowflake용) 수령.

### D30. Frontend = React + TS + Vite
- REST = TanStack Query, WS = 네이티브 클라이언트. (모던 선호 시 SvelteKit이 대안이나 기본은 React.)

---

## 3-E. 마감 결정 (Edge cases & Hardening)

### D31. 이중 직렬화 (클라 vs 노드)
- **클라↔Gateway(WS) = JSON** (브라우저/TS 디버깅 용이, Discord와 동일 철학).
- **노드↔노드 = 수제 바이트**(D3). 두 직렬화는 별개 레이어로 분리.

### D32. 분산 Rate limiting = per-node 근사 시작
- 토큰 버킷을 **노드별 로컬 보유**(근사). 로컬 노드 3~10개에선 누수 미미.
- 정밀도가 꼭 필요한 엔드포인트만 **유저-해시 소유 노드 집중관리(b)**로 후속 승격.
- **구현(Phase 4)**: `rest-api::ratelimit`. 순수 `TokenBucket`(용량+초당 리필, 연속 토큰) + per-node `RateLimiter`(`rule:identity`별 버킷, **인메모리 DB-D5 휘발**) + axum 미들웨어. 버킷 클래스: `/auth/*`=노드 전역(가입/로그인, PoW(D18)와 상보적) · 그 외 인증=유저별(토큰 검증) · 미인증=전역 anon. 초과 시 **429** + `X-RateLimit-{Limit,Remaining,Reset}`/`Retry-After` 헤더(rest.md §0). 시각은 주입 clock(DST 결정론). server가 `with_defaults`(auth 20·user 120·anon 60) 주입, 테스트는 `lenient`/`from_rules`. **라이브 검증**: cli scenario 정상 통과 + pow-challenge 폭주 시 정확히 20개 통과→429. **seam**: 노드별 독립이라 전역 정밀 한도 아님(D32 근사) · 메시지 전송(gateway 서빙)·gateway WS는 아직 미적용(후속) · 유저-해시 승격(b)은 후속.

### D33. 합의(consensus) 레이어 = 없음 (명시적 경계)
- **Raft 등 합의 없음.** 정적 config 해시링을 모든 노드가 신뢰.
- **split-brain/네트워크 파티션 처리 = 범위 밖.** (로컬 study 전제. Discord도 Realm 소유에 합의 미사용.)
- 포트폴리오엔 "왜 합의를 안 넣었는지 안다"로 서술.

### D34. 메시지 멱등성 = 클라 nonce
- at-least-once(D24) 재전송 중복 방지: 클라가 보내는 **nonce로 dedup**(Discord 방식).
- **구현 변경(Phase 4, v1.38 — 파티셔닝 D28)**: 초기엔 `uq_messages_nonce(channel_id, author_id, nonce)` 부분 유니크 인덱스로 DB가 강제했으나, 메시지를 RANGE(id) 파티셔닝하면서 **앱레벨 dedup으로 이전**(Postgres가 파티션 유니크에 파티션 키 포함을 강제 → 부분 유니크 불가). `create_message`가 가드 INSERT(`SELECT ... WHERE $nonce IS NULL OR NOT EXISTS (동일 channel,author,nonce)`)로 dedup한다. **레이스 안전성**: 송신 경로의 persist는 dispatch 드라이버(events 채널의 **단일 직렬 소비자**, D24)만 수행 → 동시 INSERT 없음. 사용자 승인 결정.

### D35. 읽기 캐싱 = Realm 액터 인메모리
- Redis 없음 → **Realm 액터가 최근 N개 메시지 bounded 캐시**. 콜드 데이터는 Postgres 직격.

### D36. 프로토콜 버저닝 = 헤더 version 바이트
- 수제 바이트 프레임 헤더에 **version 1바이트**. 롤링 재시작/DST 재현 시 포맷 진화 대비.

### D37. 파일 첨부 = 로컬 FS (Phase 4) — 구현됨 (v1.35)
- 로컬 파일시스템(`LocalFsBlobStore`)으로 구현. ~~**MinIO(S3 호환)** 선택적 업그레이드~~ → **범위 제외**(아래 결정).
- **MinIO 범위 제외 결정 (Phase 5, 사용자 결정 2026-06-16)**: MinIO 어댑터는 **범위에서 제외**한다. 구현 불가라서가 아니라(MinIO는 `minio server /data`로 로컬에서 도는 S3 호환 서버 = Postgres와 같은 로컬 의존성, 철학 충돌 없음), **이 프로젝트가 로컬 테스트 전용 + 다중 PC/확장 의사가 없어** MinIO가 주는 가치(객체 스토리지 분리)가 불필요하기 때문. `LocalFsBlobStore`로 첨부 요구가 완전히 충족된다.
  - **단, `BlobStore` 포트는 유지** — 이건 MinIO용 발판이 아니라 *domain이 IO를 모른다*(P2)는 헥사고날 경계 그 자체이고, 지금 `LocalFsBlobStore`가 구현 중이다. 포트를 없애면 domain이 FS에 직결돼 퇴보. 훗날 S3/MinIO가 필요해지면 **같은 포트에 어댑터 1개**(`aws-sdk-s3`/`object_store`, env 게이팅 테스트)만 추가하면 된다 — 설계는 그대로 열려 있음.
- **구현(Phase 4)**: 메타데이터와 바이트를 **분리**한다 — 메타(`attachments` V14: filename/size/content_type/url)는 `AttachmentRepository`(PgStore), 바이트는 **`domain::blob::BlobStore` 포트** 뒤로(로컬 FS=`storage::LocalFsBlobStore`, MinIO/S3는 같은 포트의 후속 adapter). domain은 둘 다 포트로만 안다(P2/P6). key=첨부 Snowflake id(경로 탈출 차단).
- **사후 첨부(seam)**: 전송 경로가 비동기(gateway→Router→actor, 업로드 시점 message_id 없음)라 **이미 존재하는 메시지에 첨부**로 단순화. REST `POST /channels/:cid/messages/:mid/attachments`(멀티파트, 작성자+ATTACH_FILES, 8 MiB) · `GET .../attachments`(목록) · `GET /attachments/:id`(다운로드, 채널 VIEW_CHANNEL). 전송 시 동시 첨부·width/height·MinIO·스트리밍은 후속.

### D38. 페이지네이션 = Snowflake 커서
- 메시지 히스토리 = `before`/`after`/`around` + Snowflake 커서.

### D39. Realm 이벤트 팬아웃 = 범용 envelope (메시지 특수화 흡수)
- **문제**: Phase 1~2의 팬아웃 경로(`RealmEvent`/`Router::fanout`/`LocalDelivery`/wire `REALM_FANOUT`)가 전부 `MESSAGE_CREATE` 모양으로 하드코딩돼 있었다. 멤버 변동(`GUILD_MEMBER_ADD/_UPDATE/_REMOVE`) 같은 **비-메시지 실시간 이벤트**를 같은 구독자표(D12)로 흘려보낼 수 없었다.
- **결정 (P4 특수케이스는 추상화로 흡수)**: 팬아웃을 **범용 envelope `(t, payload)`**로 일반화한다. `t`=이벤트 이름(`"MESSAGE_CREATE"`, `"GUILD_MEMBER_ADD"` …), `payload`=**클라에 그대로 나갈 JSON을 미리 직렬화한 불투명 문자열**. 메시지는 이 envelope의 한 경우가 된다.
  - **JSON 단일 출처 = 생산 엣지**: payload JSON은 그 이벤트를 만든 엣지(메시지는 dispatch 드라이버, 멤버는 rest-api 핸들러)가 **한 번** 조립한다. 그 아래 계층(domain 포트·node·protocol wire)은 payload를 **파싱하지 않고 통과**시킨다 → node/protocol은 serde 무의존 유지(P2). 최종 배달 직전 gateway가 문자열→`Value`로 1회 역파싱해 세션에 push.
- **persist 분기**: persist-then-fanout(D24)에서 **persist는 메시지에만** 적용. 멤버 이벤트는 진실이 이미 `members` 테이블(REST 트랜잭션)에 있으므로 **비-persist 팬아웃**. dispatch 드라이버가 이벤트 종류로 분기.
- **emit 포트 (P2)**: REST(rest-api)는 Router를 직접 모른다 → domain에 **`RealmEmitter` 포트**(repo 포트와 같은 자리)를 두고 `Router`가 구현(`route_emit`: 로컬 소유면 액터로, 원격이면 wire `REALM_EMIT`로 위임 — `route_send`와 대칭). server가 `Arc<dyn RealmEmitter>`를 rest-api `AppState`에 주입.
- **wire(node-wire.md §5 갱신)**: `REALM_FANOUT`(0x0103) 바디를 `realm_id, t:String, payload:String, user_ids:Vec<u64>`로 일반화(메시지 전용 필드 제거 — 모두 payload 안으로). 비소유 노드→소유 노드 emit 위임용 `REALM_EMIT`(0x0104) 신설(`realm_id, t:String, payload:String`). 기존 평탄 서브셋(D24 노트)을 이 범용형으로 대체.
- **멤버 관리(Phase 3)**: `GuildRepository`에 `get_member/list_members/update_member_nick/remove_member` 추가. REST `GET/PATCH/DELETE /guilds/:id/members[/:uid]`(조회=멤버, nick=본인 CHANGE_NICKNAME 또는 타인 MANAGE_NICKNAMES, 추방=KICK_MEMBERS, leave=본인). 변동 시 위 emit으로 `GUILD_MEMBER_*` 팬아웃. 신규 합류자는 아직 미구독이라 **기존 접속 멤버**에게 통지되고, 본인은 redeem 응답/다음 READY로 상태 확보(D13).
- **메시지 편집·삭제·리액션(Phase 3, 1.23)**: 같은 envelope로 `MESSAGE_UPDATE`/`MESSAGE_DELETE`/`MESSAGE_REACTION_ADD`/`_REMOVE`를 팬아웃(비-persist — 진실은 REST 트랜잭션이 DB에 기록). `MessageRepository`에 `get_message/edit_message/soft_delete_message`, 신규 `ReactionRepository`(add/remove). 편집=작성자 본인, 삭제=작성자 또는 `MANAGE_MESSAGES`, 리액션 추가=`ADD_REACTIONS`(채널 컨텍스트)·제거=본인. 소프트 삭제는 `messages.deleted_at`(히스토리 쿼리가 `deleted_at IS NULL` 필터). 리액션 저장 = **V7 `reactions`**(유니코드 emoji 1컬럼 PK — 02-schema 단순화 노트, 커스텀 이모지는 Phase 4).
- **답장·멘션(Phase 3, 1.24)**: 둘은 메시지 **생성(MESSAGE_CREATE)** 경로에 얹는다(D39 broadcast가 아니라 기존 persist-then-fanout, D24).
  - **답장**: `messages.reference_message_id`(구조적 입력)를 송신 경로 전체에 관통 — gateway `POST /channels/:id/messages` 바디 → `Router::route_send` → `RealmCommand::SendMessage` → `RealmEvent::MessageCreated` → wire `REALM_SEND`(크로스노드) → `NewMessage` persist → `MESSAGE_CREATE` payload. 참조 대상은 같은 채널의 살아있는 메시지여야(gateway에서 검증, 아니면 400).
  - **멘션**: content에서 **파생**되므로 파이프라인을 안 건드린다 — dispatch 드라이버가 persist 후 `domain::mention::parse_mentions`(`<@id>`/`<@!id>`)로 뽑아 **V8 `message_mentions`**(유저만, FK 존재하는 유저로 한정)에 적재하고 `MESSAGE_CREATE` payload에 `mentions:[id]` 포함. 역할 멘션·"나를 멘션한 메시지" 조회는 Phase 4.

### D40. 친구·차단(relationships) + 유저 단위 이벤트 emit
- **데이터 (Phase 3 구현)**: Discord식 **방향성 행** `relationships(user_id, target_id, kind)` (02-schema §6, wire V10). A↔B 친구 = 양쪽 행 2개. kind = `friend`/`pending_in`/`pending_out`/`blocked`. 친구 요청 = 내 행 `pending_out`/상대 `pending_in` → 수락 시 양쪽 `friend`. 차단 = 내 행 `blocked` + 상대 행 제거. 상태 전이의 원자성(두 행)은 storage 트랜잭션(`RelationshipRepository`).
- **차단 강제 (permissions.md §5 seam 닫힘)**: 어느 한쪽이라도 차단 시 **1:1 DM 열기·전송 거부**(rest-api `open_channel` + gateway `can_send`의 1:1 DM 분기에서 `is_blocked_between` 검사). 그룹DM엔 미적용(Discord 동일).
- **유저 단위 이벤트 emit (`UserEmitter`) — D12의 "팬아웃 ↔ 전역 presence 분리"의 실체**: 친구·차단(`RELATIONSHIP_ADD/_REMOVE`)은 Realm 무관 전역 유저 이벤트라 구독자표(D12) 기반 `RealmEmitter`(D39)로 보낼 수 없다 → 별도 **`UserEmitter` 포트**(domain `emit`)를 둔다. 구현(adapter) = gateway `Hub`(대상 유저의 **이 노드 로컬 세션**에 배달), server가 rest-api `AppState`에 주입(`RealmEmitter`와 대칭).
  - ⚠ **seam → D43에서 해소**: 도입 당시엔 대상 유저가 **다른 노드**에 접속 중이면 미배달(로컬 한정)이었다. **D43**이 `Presence` 디렉터리(D42, user→호스팅 노드)를 라우팅 키로 `USER_DELIVER`를 타깃 노드에 보내 크로스노드 배달을 닫았다. (Realm 이벤트는 구독자표로 이미 크로스노드.)

### D41. 읽음 상태(read_states) + 미읽음 멘션 카운트
- **데이터 (Phase 3 구현)**: `read_states(user_id, channel_id, last_read_message_id, mention_count)` (02-schema §8, wire V11). 채널별 "어디까지 읽었나" + 안 읽은 멘션 수. `last_read_message_id`는 FK 없음(messages는 Phase 4 파티셔닝 대상 — attachments/reactions와 동일 방침).
- **mention_count 유지 전략 (자문자답)**: 매 조회 시 전체 카운트 = 비쌈. 매 메시지마다 갱신 = 핫패스 비용. → **증분 유지 + ack 시 재계산** 하이브리드:
  - 멘션 발생 시(dispatch가 `message_mentions` 적재 직후) 대상들의 `mention_count` **+1**(`bump_mentions`, 작성자 제외, 존재 유저만 upsert). 새 메시지는 항상 최신 → 단순 증가가 정확.
  - **ack**(`POST /channels/:cid/messages/:mid/ack`) 시 `last_read`=mid + `mention_count`를 **그 이후(`m.id > last_read`) 살아있는 멘션 수로 재계산**(한 문장 upsert). 증분 누적 오차를 ack가 정정.
- **실시간 `MESSAGE_ACK`**: 한 유저의 여러 기기 동기화용 → Realm 무관 유저 이벤트라 **`UserEmitter`(D40) 재사용**(본인 세션들에 배달). READY 스냅샷에 `read_states` 포함(자동구독 D13 시점에 상태 확보). 크로스노드 유저 라우팅 seam은 D40과 함께 **D43에서 해소**(여러 기기가 다른 노드에 흩어져도 ACK 동기화됨).

### D42. 전역 presence = gossip broadcast + 로컬 친구 필터 (Q11/D12 해소)
- **문제 (D12 §"팬아웃 ↔ 전역 presence 분리"의 미정 갈래, Q11)**: 친구 온라인 여부는 Realm 무관 전역 유저 상태 → 구독자표(D12) 팬아웃으로 못 보낸다. D40/D41의 `UserEmitter`도 **로컬 노드 세션에만** 배달돼 크로스노드 미배달 seam이 있었다.
- **결정 (Phase 3 구현)**: presence를 **풀메시 gossip broadcast(D4) + 각 노드의 로컬 친구 필터**로 전파한다.
  - **휘발 상태(DB-D5)**: `node::presence::Presence` 인메모리 레지스트리. `user → (status, 호스팅 노드 집합)` — 집합이 비면 offline("any node hosts ⇒ online", 멀티노드 정확). 현재 online/offline(idle/dnd는 op 3 후속).
  - **전이 기준 = live 세션**: 유저의 첫 live(소켓 연결) 세션에서 online, 마지막 live 세션 종료에서 offline(`Hub::live_count`). detach(버퍼만 남음)는 offline로 안 침 → RESUME 유예와 일관.
  - **전파**: 전이 시 `PRESENCE_GOSSIP{user, node, status}`(wire 0x0201)을 **모든 피어에 broadcast**(`Router::broadcast`). 각 노드는 자기 view 갱신 후, 그 유저의 친구(relationships, D40) 중 **로컬 세션 보유자**에게 `PRESENCE_UPDATE` 배달(`Hub::deliver`가 로컬 없는 유저 자동 스킵). 수신 노드는 **재브로드캐스트 안 함**(원본이 풀메시로 이미 전 피어에 전송 → 루프·증폭 방지). READY 스냅샷에 친구 presence 포함.
  - **친구 산출 = relationships 재사용**: 새 repo/포트 없이 `list_relationships(user) filter friend`로 대상 산출 → domain/storage/rest-api 무변경(친구 그래프가 presence 라우팅 키).
- **이것이 닫는 것**: D40(RELATIONSHIP_*)·D41(MESSAGE_ACK)의 크로스노드 유저 라우팅 → **D43**이 구현. 단 presence의 **풀메시 broadcast**가 아니라 Presence **디렉터리로 타깃 노드에만** `USER_DELIVER` 전송(수신자가 특정 유저라 broadcast 불필요). 같은 디렉터리를 공유하는 상보적 두 패턴.
- **남은 seam**: 신규 노드 join 시 과거 presence 동기화(anti-entropy/SWIM) 없음(델타 only, Phase 5 gossip discovery와 함께) · ~~idle/dnd 클라 op(3)~~ **→ 구현됨(v1.42, Phase 5 하드닝)**: 클라 op 3(`{status:online|idle|dnd}`)이 `presence::set_status`로 전이 → 기존 gossip/친구 통지 경로 재사용. 연결 중엔 online 계열만(offline은 세션 종료가 담당). · 전 노드 동시 재시작 시 presence 리셋(휘발).

### D43. 크로스노드 유저 이벤트 라우팅 = Presence 디렉터리 기반 타깃 전송 (D40/D41 seam 닫음)
- **문제**: D40(`RELATIONSHIP_ADD/_REMOVE`)·D41(`MESSAGE_ACK`)은 `UserEmitter`(Realm 무관 유저 이벤트 포트)를 통해 통지되는데, 구현(adapter)이 gateway `Hub`라 **이 노드의 로컬 세션에만** 배달됐다. 대상 유저가 다른 노드에 접속 중이면(친구 요청 실시간 수신, 다기기 ACK 동기화) 미배달. D42 presence는 풀메시 broadcast로 크로스노드를 뚫었지만 그건 presence 델타에 한정.
- **핵심 관찰**: D42가 만든 `node::Presence` 레지스트리는 `user → (status, 호스팅 노드 집합)`을 이미 보유 → 이게 곧 **"유저 X가 어느 노드에 있나"를 답하는 유저 위치 디렉터리**다. 새 레지스트리 없이 라우팅 키로 재사용한다.
- **결정**: `UserEmitter`를 **크로스노드 인지 어댑터**로 일반화한다. presence처럼 전 노드에 broadcast하지 **않고**, 대상 유저별로 Presence 디렉터리에서 호스팅 노드를 조회해 **그 노드에만 타깃 전송**한다(수신자가 특정 유저라 풀메시 불필요 — presence와 상보적). 포트 시그니처(`emit_to_users`)는 불변이라 **rest-api 라우트(relationship/read_state)는 무변경**, server가 주입하는 *구현*만 교체.
  - **로컬**: 대상 유저의 이 노드 세션에 `Hub::deliver`(detach 중 버퍼 세션도 포함 → RESUME 복구).
  - **원격**: 대상 유저를 호스팅하는 (로컬 제외) 노드별로 묶어 `USER_DELIVER{t, payload, user_ids}`(wire 0x0202) 전송 → 수신 노드가 `Hub::deliver`로 자기 로컬 세션에 배달.
  - **어댑터 위치**: gateway `user_route::UserRouter`(Hub + `Arc<Presence>` + `Arc<Router>` 결합) — `RealmEmitter`=Router와 대칭. `Router::send_to`(타깃 노드 1개 전송) + `Presence::nodes_for` 신설.
- **견고성**: `Hub::deliver`는 로컬 세션 없는 유저를 자동 스킵 → 디렉터리가 약간 stale해도 **무해(no-op)**. 디렉터리에 없는 유저(오프라인)는 어차피 받을 세션이 없어 정상 드롭.
- **남은 seam**: 디렉터리는 **live(온라인) 세션**만 추적 → 원격 노드에서 detach-grace 중인 세션은 in-flight 이벤트를 못 받음(RESUME/다음 READY 스냅샷으로 상태 확보) · 신규 노드 anti-entropy 없음(D42와 동일, Phase 5).

### D44. 스레드/포럼 = 부모를 가리키는 채널 (P4 재사용)
- **문제**: 스레드/포럼을 별도 1급 개념으로 두면 메시징·팬아웃·권한 경로가 또 갈라진다(특수 케이스 번짐 = 지난 실패 패턴).
- **결정 (Phase 4, P4)**: 스레드를 **채널의 한 종류**로 흡수한다. 스레드 = 부모 채널과 **같은 Realm의 `channels`(kind='thread', parent_id=부모) 한 행** + `thread_meta`(owner/archived/auto_archive) 보강(V13). 포럼 = `channels.kind='forum'` 컨테이너.
  - **무변경 재사용**: Realm 액터는 realm 단위 구독자표(D12)로 팬아웃하므로, 스레드 채널 메시지도 **기존 메시지 경로(gateway send → Router → RealmActor → dispatch persist-then-fanout)를 그대로** 탄다. 자동구독(D13)도 realm 단위라 스레드 명시 join 불필요. → gateway/node/protocol/server 코드 추가 0.
  - **권한**: 생성=`CREATE_PUBLIC_THREADS`(부모 채널 컨텍스트), 아카이브=소유자 또는 `MANAGE_THREADS`. `default_everyone`에 CREATE_PUBLIC_THREADS·SEND_MESSAGES_IN_THREADS 추가(기본 사용 가능, Discord 정렬). 스레드 채널 자체엔 오버라이드 없어 권한이 realm 역할로 폴백(DM과 같은 메커니즘, D8).
  - **message_count**: `thread_meta` 컬럼은 스키마 충실성용이고, 실제 값은 **조회 시 `messages` 집계** — 쓰기 핫패스에 카운터 결합을 만들지 않는다.
  - 어댑터: `domain::thread`(`Thread`/`NewThread`) + `ThreadRepository`(storage 트랜잭션). REST `POST/GET /channels/:id/threads`·`PATCH /channels/:id/thread` + `THREAD_CREATE`/`THREAD_UPDATE` 팬아웃(D39 envelope).
- **남은 seam**: SEND_MESSAGES_IN_THREADS 분리 강제(현재 SEND_MESSAGES만)·포럼 태그(`forum_tags`)·자동 아카이브 만료 스케줄·스레드 멤버 명시 목록은 후속.

---

## 3-F. Phase 5 — 동적 클러스터 (Dynamic Cluster)

### D45. 노드 발견/장애감지 = SWIM (D5의 정적 config를 동적 합류로 확장)
- **문제 (D5의 후속 갈래, Q11)**: Phase 2~4는 **정적 config**(`cluster-config`의 peers 목록)로 풀메시를 구성했다. 모든 노드가 부팅 시 전체 노드 목록을 알아야 하고, 노드 추가/제거 시 전 노드의 config를 고쳐 재시작해야 한다. 장애감지(D23)도 단순 PING/PONG + timeout sweep이라 **2상태(Alive/Down)**뿐 — 일시적 지연/비대칭 단절을 곧장 down으로 오판(false positive)해 소유권이 출렁인다.
- **결정 (Phase 5)**: 멤버십을 **SWIM**(Scalable Weakly-consistent Infection-style process group Membership)으로 일반화한다. SWIM의 세 기둥을 그대로 구현(수제, P3 stub 위에서 출발):
  1. **3상태 + incarnation number**: 멤버 상태 = `Alive` / `Suspect` / `Dead`. 각 노드는 자기 **incarnation**(단조 증가)을 소유. 충돌 해소 규칙(표준 SWIM): **높은 incarnation 우선**, 같으면 `Dead > Suspect > Alive`. 한 노드가 "나를 Suspect"라는 소문을 보면 incarnation을 **+1 해서 Alive를 반박(refute)** 전파 → 자기 자신만이 자신을 살릴 수 있다(오탐 자동 복구).
  2. **direct + indirect 탐침(probe)**: 주기마다 멤버 1명을 골라 `SwimPing`. 타임아웃 시 곧장 죽이지 않고 **k명의 임의 멤버에게 `SwimPingReq`**(대신 그 타깃을 ping해 달라) → 간접 ack가 하나라도 오면 Alive 유지. 직접·간접 모두 실패해야 **Suspect**로 전이. SWIM의 핵심 — 비대칭/일시 단절에서 오탐을 줄인다.
  3. **감염형 전파(infection-style dissemination)**: 멤버 상태 변화(join/alive/suspect/dead)를 ping/ack/ping-req에 **피기백**하고 + 임의 소수에게 `SwimGossip` 배치로 확산. 각 업데이트는 **유한 횟수**(≈ λ·log N)만 재전파(가장 최근/덜 퍼진 것 우선) → O(N) broadcast 없이 전 노드 수렴.
- **동적 합류(join)**: 신규 노드는 config의 **seed(introducer)** 1명에게 `SwimJoin{addr, incarnation}` → seed가 전체 멤버 테이블을 `SwimGossip`로 회신 + 신규 멤버를 Alive로 감염 전파 → 전 노드가 학습. **주소(addr)를 멤버 업데이트에 실어** 각 노드가 신규 노드를 **런타임 dial**(transport)할 수 있게 한다(풀메시 자가구성, D4).
- **동적 해시링**: 멤버 상태 변화가 곧 링 변화 — `Alive` 학습 → `ring.add_node`, `Dead` 확정 → `ring.remove_node`. `Suspect`는 링에 남기되(아직 살아있을 수 있음) **신규 소유권 부여에서 제외**(보수적). 일관 해싱이라 노드 1개 출입은 그 노드 몫 Realm만 재배치(D6) → 새 소유 노드가 Postgres에서 rehydrate(D23). 이를 위해 **`Router.ring`을 런타임 가변(RwLock)으로** 전환.
- **경계 (P2/P5)**: 상태머신(`node::swim::Swim`)은 **순수 상태 + now_ms 주입**(IO 없음) — 멤버 테이블 + 합병 규칙 + "다음에 보낼 메시지/적용할 링 변화"를 산출. 실제 송신/링 변형/dial은 드라이버(`run_swim`)가 transport·Router·clock으로 수행 → **DST(D25) 결정론 재현**(SimNetwork + ManualClock로 join/suspect→dead/refute/partition 시드 재현).
- **wire (node-wire.md §4-5 갱신)**: 0x03xx 클러스터 범위에 SWIM 5종 — `SwimJoin`(0x0301, 예약됐던 `RING_UPDATE` 대체) · `SwimPing`(0x0302) · `SwimAck`(0x0303) · `SwimPingReq`(0x0304) · `SwimGossip`(0x0305). 공통 페이로드 `SwimMember{node_id, addr, incarnation, state:u8}` 리스트로 멤버 델타를 피기백.
- **합의 아님(D33 불변)**: SWIM은 약한 일관성(eventually consistent membership)일 뿐 합의가 아니다 — split-brain은 여전히 범위 밖. 정적 config 경로도 유지(SWIM 미구동 시 fallback, 기존 테스트 보존).
- **하드닝(v1.40)**: ① **robust join** — `run_swim`이 bootstrap(seed 응답 수신) 전까지 매 tick `SwimJoin` 재전송 → 신규 노드가 **seed보다 먼저 떠도** seed 등장 시 합류(startup 1회 전송 유실 seam 닫음, 라이브 검증). ② **round-robin probe** — 임의 선택 대신 셔플 라운드로빈으로 한 라운드에 각 멤버 1회 탐지(탐지시간 상한). ③ **주기적 anti-entropy** — bounded 전파 누락 대비 N tick마다 full-snapshot을 임의 멤버에 push(수렴 보강). ④ SWIM 파라미터 **env 노출**(`SWIM_*`).
- **남은 seam**: 클러스터 전체 동시 재시작 시 seed도 죽으면 부트스트랩 불가(seed 다중화로 완화) · 멤버 테이블 영속화 없음(**의도적 휘발, DB-D5**) · 네트워크 파티션 양쪽이 서로 Dead 처리(합의 부재, D33) · presence는 join-시 1회 push(주기 digest 아님, D46).

### D46. presence anti-entropy = 합류 시 스냅샷 동기화 (D42의 델타-only seam 닫음)
- **문제 (D42 남은 seam)**: 전역 presence는 **델타 전파만** 한다(전이 시 gossip broadcast). 신규 노드는 **합류 이전에 일어난 전이**를 영영 못 받아, 이미 온라인인 친구를 offline으로 본다(anti-entropy 부재).
- **결정 (Phase 5)**: 신규 노드 합류를 SWIM(D45)이 감지하면, 기존 각 노드가 **자기가 호스팅하는 유저들의 현재 presence 스냅샷**을 신규 노드에 push(기존 `PRESENCE_GOSSIP` 0x0201 재사용 — 신규 wire 불필요). 신규 노드는 받은 gossip을 `apply_gossip`(기존 경로)로 흡수 → view 수렴.
- **경계 (P2)**: presence는 server/gateway 관심사(Store+Hub 필요)라 node 코어가 모른다 → SWIM 드라이버는 "노드 X가 합류"를 **이벤트 채널로 방출**만 하고, server가 그걸 받아 `Presence::snapshot_local`(이 노드 호스팅 유저들)로 스냅샷을 만들어 `Router::send_to(X, ...)`로 보낸다. SWIM↔presence는 느슨히 결합.
- **남은 seam**: 완전 anti-entropy(주기적 merkle/digest 교환)는 아님 — join 시 1회 push(델타 + join-스냅샷이면 study 범위 충분). 유저가 많으면 스냅샷 배치 분할은 후속.

### D47. Voice 시그널링 = 제어 평면만 설계 (D21 경계 구체화, 미디어 제외)
- **결정 (Phase 5, 설계 전용)**: 음성 **시그널링**(누가 어느 음성 채널에 있나·mute/deaf·미디어 서버 안내)을 **기존 Realm 라우팅으로 흡수**(P4)해 설계만 한다. **미디어 평면(WebRTC/SFU/Opus/SRTP/UDP)은 D21대로 제외** — 코드 미구현(사용자 결정: Voice=설계 문서만).
- **핵심 관찰**: voice state는 "Realm 안의 실시간 휘발 상태"라 **presence(D42)·구독자표(D12)와 동형**이다 → DB에 안 둠(DB-D5), Realm 액터가 `{user→VoiceState}` 맵 보유(구독자표 옆자리), 팬아웃은 **D39 범용 envelope `REALM_FANOUT`을 그대로 재사용**(`t="VOICE_STATE_UPDATE"`). 즉 시그널링은 텍스트 메시징 인프라에 **거의 코드 0으로 얹힌다**.
- **경로**: gateway op 4(`VOICE_STATE_UPDATE` C→S) → 권한(CONNECT, D17) → 소유 노드 위임(예약 wire `VOICE_STATE_SET` 0x0120, `REALM_EMIT`과 대칭) → Realm 액터 맵 갱신 → `REALM_FANOUT`로 구독자에 `VOICE_STATE_UPDATE`. 입장 세션엔 `VOICE_SERVER_UPDATE` 회신하되 **endpoint=null stub**(미디어 서버 없음 = D21 경계 표식). 권한 CONNECT/SPEAK/MUTE·DEAFEN·MOVE_MEMBERS는 permissions.md 예약 비트의 소비처.
- **명시적 비구현(이 결정의 핵심)**: 미디어 전송 일체. 포트폴리오 서술 = "시그널링은 메시징과 동형이라 흡수, 미디어는 의도적 경계 밖".
- 상세: [protocol/voice-signaling.md](../protocol/voice-signaling.md).

### D48. 이벤트 소싱 = append-only Realm 이벤트 로그 + 순수 프로젝션 (D23의 스트레치 후보 구현, 가산형)
- **결정 (Phase 5, 설계 후 구현)**: D23이 "이벤트 소싱은 Phase 5 스트레치 후보"라 한 갈래를 **가산형(additive, 비파괴)** 으로 구현한다. `messages`(엔티티 진실)를 **대체하지 않고**, 그 위에 **타입화된 도메인 사실의 append-only 로그**(`realm_events`, V20)를 얹는다 → CQRS: events = write-side fact, projection = read model. *진실 중복으로 꼬일 위험*(코드 꼬임 = 지난 실패)을 피하려 messages를 ES의 진실로 만드는 파괴적 마이그레이션은 **하지 않는다**(그건 더 큰 후속).
- **순수 코어 (P2, DST 친화)**: `domain::event` — `RealmEventKind`(MessageCreated/Deleted·MemberJoined/Left, 안정 `code()`) + `RealmProjection`(이벤트 시퀀스를 **결정론적으로 fold** → members·message_count·last_message_id·last_seq). IO 없음 → "상태 = 이벤트의 함수" 불변식을 단위 테스트로 고정(재생 결정론·증분 재생=전체 재생·언더플로 없음). domain은 serde 무의존 유지 — 직렬화는 storage 어댑터가 (code + 타입화된 nullable bigint 슬롯)으로 소유.
- **port (D22)**: `EventLogRepository`(append_event→per-realm 단조 seq, replay_events→증분 커서) → `Store` 슈퍼트레잇 합류. seq 부여 = `coalesce(max(seq),0)+1`인데 **단일 직렬 소비자**(dispatch 드라이버, D24)만 append하므로 경합 없음(nonce 멱등 D34와 동일한 앱레벨 직렬화 논리).
- **생산자 (가산 배선)**: dispatch 드라이버가 persist(D24) 직후 `MessageCreated`를 append(messages 진실 + 로그 사실 둘 다 단일 소비자가 순서대로 기록). append 실패는 warn 후 계속(배달 안 막음 — messages가 진실, 로그는 보조). **라이브 검증**(cli scenario → `realm_events`에 MessageCreated 1건, message_id 일치).
- **rehydrate 연결(D23/D35)**: `replay_events`→`RealmProjection`가 Realm 파생 상태(특히 last_message_id)를 재구성 → **D35 최근메시지 캐시 warmup**의 입력이 될 자리(후속 배선 지점).
- **남은 seam**: append가 persist와 같은 트랜잭션이 아님(완전 무결성은 후속) · 멤버/삭제 이벤트 생산자는 dispatch 외(rest-api) 경로라 후속 배선 · 스냅샷/컴팩션 없음(전체 재생) · 메시지 핫패스 2차 쓰기(write amplification — 배치/비동기 append가 운영 최적화). 비파괴라 messages 경로는 그대로.
- 코어: `domain::event`(순수) · 포트 `EventLogRepository` · `storage` V20 `realm_events`.

### D49. CRDT 오프라인 동기화 = 상태 기반 CRDT(CvRDT) + LWW 유저 문서
- **결정 (Phase 5, 설계 후 구현)**: 여러 기기가 **오프라인 편집 후 재연결 시 충돌 없이 수렴**하는 동기화를 **상태 기반 CRDT**(CvRDT)로 구현한다. 핵심 가치 = **순수·테스트 가능한 merge 엔진**(semilattice join: 결합·교환·멱등) — 네트워크가 재정렬/중복/재전송해도 안전한 수렴(분산 테마 정합, 합의 없이도 일관 D33).
- **순수 코어 (P2/DST)**: `domain::crdt` 툴킷 — `LwwRegister`((ts,node) 타이브레이크), `LwwMap`(키별 LWW + 툼스톤 삭제), `OrSet`(observed-remove, add-wins 쇼케이스), `PnCounter`. merge 법칙을 단위 테스트로 고정(멱등·교환·결합·오프라인 2기기 수렴). domain serde 무의존.
- **적용 = 유저 동기화 문서(LWW-Map)**: 드래프트/설정 같은 per-user 키-값을 키별 LWW로 동기화. 병합 권위는 domain `LwwMap`, 영속은 **storage의 LWW 가드 upsert**(`(ts,node)` 큰 것만 채택 — DB가 LWW 시맨틱 보존, `user_crdt_entries` V21). 포트 `CrdtRepository`(load/merge) → Store 합류.
- **동기화 프로토콜 (상태 기반)**: REST `GET/POST /users/@me/sync`. 기기가 로컬 상태(엔트리들)를 POST → 서버가 LWW 병합 → 병합 문서 회신(기기는 응답을 자기 복제본과 다시 병합). 상태 기반이라 델타 추적 불필요 — 어느 순서로 push해도 수렴. **라이브 검증**(phone@200 → laptop@210 채택 → 이른 phone 재push 무시 → GET 수렴, 툼스톤 삭제).
- **경계/seam**: 현재 LWW-Map 1종만 wire(OrSet/PnCounter는 툴킷에 있으나 미배선 — 리액션/카운터 적용은 후속) · 동기화는 REST 폴링(WS push/실시간 anti-entropy 아님) · 메시지 본문은 CRDT 아님(append-only + 서버 id가 더 단순, D24) · 멀티노드 간 user_crdt_entries 수렴은 DB 공유로 자연(노드 무관 무상태 REST).
- 코어: `domain::crdt`(순수 툴킷) · 포트 `CrdtRepository` · `storage` V21 `user_crdt_entries` · rest-api `/users/@me/sync`.

---

## 4. Discord Backend 도메인 지형도 (참고)

카피 대상 전체 그림 (컷라인은 §5 열린 질문에서 결정).

- **엣지**: REST API / Gateway(WS) / Voice(WebRTC, 별도) / CDN / Rate Limiter
- **Gateway 내부**: 연결 수명주기(IDENTIFY→READY→HEARTBEAT→RESUME), 세션 상태, 이벤트 팬아웃
- **엔티티**: User / Guild / Channel(텍스트·음성·카테고리·스레드·DM·그룹DM·공지·포럼) / Message / Role·Permission / Member / Invite / Emoji·Sticker·Reaction / Presence / Relationship(친구·차단) / Webhook·Integration·Bot
- **권한 시스템**: 비트마스크. 계산순서 = `@everyone` → 역할 OR → 채널 오버라이드 → 멤버 오버라이드 → Administrator 통과. 채널마다 재계산.
- **저장소**: (실제 Discord는 Cassandra→ScyllaDB) → 여기선 **PostgreSQL**. 메시지 테이블 파티셔닝/인덱싱 = 열린 질문.
- **횡단 관심사**: 인증/인가, Rate limiting, Snowflake, 검색, 알림, 감사 로그, 관찰성.

---

## 4-R. 단계별 로드맵 (Phased Roadmap)

> 원칙 P1~P6 준수. **단일노드 → 분산 활성화** 순서로, 경계부터 굳히고 기능을 얹는다.

- **Phase 0 — 뼈대 (인프라 stub)**: Cargo workspace/crate 경계, 노드 런타임, 수제 액터 시스템, 전송 `trait` + in-process stub, Snowflake, DB 연결. *기능 0개.*
- **Phase 1 — MVP 메시징 (단일노드)**: 가입/로그인(PASETO+Argon2), 길드 생성, 텍스트 채널, 메시지 송수신, WS Gateway, CLI 클라이언트.
- **Phase 2 — 분산 활성화**: raw TCP + mTLS 전송 구현체로 stub 교체, 멀티노드, Consistent Hashing, Realm 2단 라우팅, 팬아웃.
- **Phase 3 — Discord 본체**: 역할/권한 비트마스크, DM/그룹DM, 멤버, 초대, 리액션, 편집/삭제, 멘션/답장, presence(gossip).
- **Phase 4 — 살붙이기**: 스레드, 포럼, 웹훅, 감사로그, 검색, 파일첨부, TOTP, PoW 봇방지, rate limit.
- **Phase 5 — 스트레치**: gossip discovery(SWIM, D45) + presence anti-entropy(D46), Passkeys(D19), **이벤트 소싱(D48)**, **CRDT 오프라인 동기화(D49)**, Voice 시그널링 설계(D47). *(MinIO 첨부 = 범위 제외 — 로컬 테스트 전용·확장 의사 없음, LocalFsBlobStore로 충분. BlobStore 포트는 유지. D37.)*

---

## 5. 열린 질문 (Open Questions) — 아직 안 정함

- [x] ~~**Q1. 기능 컷라인 / Voice**~~ → D21 + §4-R 로드맵으로 해결.
- [x] ~~**Q2. 공유 상태 위치 (팬아웃)**~~ → D12로 해결 (Realm-local). 전역 presence → **D42**(gossip broadcast), 크로스노드 유저 이벤트 라우팅 → **D43**(Presence 디렉터리 타깃 전송)으로 잔여 갈래 모두 해결.
- [x] ~~**Q3. 메시지 저장 스키마**~~ → D28로 해결.
- [x] ~~**Q4. 인증 방식**~~ → D14/D19로 해결 (PASETO+refresh, TOTP). 남은 갈래: OAuth2/봇토큰 범위는 Q1 기능 컷라인과 함께.
- [x] ~~**Q5. Rate limiting**~~ → D18로 해결 (Token Bucket).
- [x] ~~**Q6. 장애/복구 범위**~~ → D23으로 해결 (rehydrate + RESUME 재연결).
- [x] ~~**Q8. Frontend**~~ → D30으로 해결 (React+TS+Vite).
- [x] ~~**Q3 외 DB**~~ → D28.

### 남은 자잘한 디테일 (Phase 진입 시 결정)
- [ ] **Q7. 액터 supervisor/재시작 전략** (D23이 큰 틀, 세부는 Phase 2).
- [x] ~~**Q9. CLI 시나리오 스크립트 포맷**~~ → 해결: CLI `scenario` 서브커맨드(헤드리스 종단 자동 검증 — 가입→길드→WS구독→전송→MESSAGE_CREATE 수신, PASS/FAIL+exit code). 별도 스크립트 DSL 없이 코드 시나리오로 시작, 필요 시 후속 확장.
- [x] ~~**Q10. 검색 구현**~~ → 해결 (Phase 4, v1.33): **Postgres FTS**. `messages.content_tsv`(tsvector STORED 생성 컬럼, config=`simple`) + GIN 인덱스(`ix_messages_fts`, 04 §5). 쿼리는 `websearch_to_tsquery`(안전 파싱). REST `GET /guilds/:id/messages/search` — 길드 단위 검색이되 결과는 **VIEW_CHANNEL 있는 채널로 한정**(채널 오버라이드 존중, D17). 외부 검색엔진(Elasticsearch) 회피 = 로컬 study 범위 적합. seam: author/before/after 필터·ts_rank 랭킹은 후속.
- [x] ~~**Q11. gossip discovery + 전역 presence**~~ — 전역 presence=**D42**, 크로스노드 유저 라우팅=**D43**, **gossip discovery(SWIM 동적 합류/장애감지)=D45**, **presence anti-entropy=D46**(Phase 5) 으로 모두 해결.

---

## 6. 독창성 포인트 (포트폴리오 어필)

- 자체 수제 바이너리 프로토콜 (raw TCP, length-prefix, 수제 직렬화)
- 풀 메시 노드 클러스터 + Consistent Hashing 기반 Realm 배치
- 수제 Actor 런타임 (tokio + mpsc), 길드=프로세스 모델링 (Discord 충실)
- 2단 라우팅 (세션 소유 vs Realm 소유)
- **SWIM gossip 멤버십** — 동적 노드 합류/이탈 + 의심 기반 장애감지 + 동적 해시링 (D45, Phase 5 구현)
- **이벤트 소싱** — append-only 사실 로그 + 순수 결정론 프로젝션(CQRS, D48, Phase 5 구현)
- **CRDT 오프라인 동기화** — 상태 기반 CvRDT(LWW-Map/OR-Set/PN-Counter), 충돌 없는 수렴 (D49, Phase 5 구현)
