# Architecture Decision Log

> Discord 클론 — 공부 + 포트폴리오용. 로컬 전용, 이론 확립.
> 철학: **"공부는 실전처럼"** — 실제로 수만 명을 붙이진 않지만, *수만 동접을 감당할 수 있는 구조*로 설계하고 로컬 시뮬레이션으로 증명한다.
>
> 최종 갱신: 2026-06-14

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

### D5. 노드 발견 = 정적 config (v1)
- v1: 노드 주소 목록을 설정 파일에 정적 정의.
- 나중에 gossip(SWIM 등)으로 동적 합류 확장 가능 (열린 질문/후속).

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

### D18. 봇방지 = PoW 챌린지 + Rate limit
- 가입/로그인 시 **Proof-of-Work 해시 퍼즐**(hashcash/mCaptcha/Anubis 스타일) — 제3자 의존 0, 수제 구현.
- **Rate limiting = Token Bucket** (per-route/per-user/global). ← 기존 Q5 흡수.

### D19. MFA = TOTP 코어 + Passkeys 스트레치
- **TOTP (RFC 6238)** 우선 구현.
- **WebAuthn/Passkeys** 는 여유 시 스트레치 목표.

### D20. 입력·세션 하이젠
- **SQL = 파라미터 바인딩만** (문자열 조립 금지).
- **Gateway RESUME 토큰 = CSPRNG 추측불가 + 검증.**
- 프론트 = **CSP** 등 보안 헤더.
- 시크릿(DB 비번 등) = `.env`/환경변수 (코드 하드코딩 금지).

### D21. Voice/Video = 범위 제외 (시그널링만 설계)
- 음성/영상 미디어(WebRTC/SFU/코덱/UDP·SRTP)는 **out**. 테마(분산 텍스트 인프라)와 결이 다르고 비용 과대.
- 시그널링(Gateway 통과 부분)은 설계만. 실제 미디어는 먼 스트레치(Phase 5).

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
- 이벤트 소싱은 Phase 5 스트레치 후보.

### D24. 전달/순서 보장
- **순서**: Realm 액터 단일 소유 → Realm 내 전순서 무료 보장. 전역 순서는 미보장.
- **저장→팬아웃 (persist-then-fanout)**: Postgres 먼저, 그다음 팬아웃 (유령 메시지 방지).
- **전달**: 이벤트마다 per-session 시퀀스 번호. 세션 노드에 bounded 재생 버퍼 → RESUME 재생. 버퍼 초과 → **REST 재조회**로 동기화.
- **구현(persist 위치)**: Realm 액터는 ID·순서만 확정해 이벤트를 방출하고(코어는 IO 무의존, P2),
  **단일 소비자인 dispatch 드라이버**(gateway)가 events 채널을 받아 `persist → fanout → 세션 배달` 순서로 처리한다.
  단일 액터가 순서대로 방출 → 단일 소비자가 순서대로 persist → 순서 보존. nonce 중복이면 persist 단계에서 멱등 스킵(D34).
- **현황(Phase 1)**: per-session seq는 세션 노드가 부여(구현됨). **RESUME 재생버퍼는 Phase 2** — 현재는 재연결 시 INVALID_SESSION 후 재IDENTIFY.

### D25. 테스트 = 유닛 + DST + e2e
- **결정론적 시뮬레이션(DST)**: 가상 시계 + 시뮬레이션 네트워크 + 시드 RNG로 클러스터를 단일 프로세스에서 재현 가능 실행, 카오스(지연/유실/파티션) 주입.
- `transport` trait + `actor-rt` 덕에 `SimTransport`+`SimClock` 주입만으로 가능. Phase 2부터.
- + 유닛 테스트 + 소수 실프로세스 e2e (WS/Postgres 엣지).

### D26. 관찰성 = tracing
- `tracing` 크레이트 + 구조적 span + **노드 간 trace-id 전파**(프로토콜 헤더). (선택) OTel→Jaeger. **Phase 0부터.**

### D27. Backpressure = bounded everywhere
- 모든 메일박스/채널 bounded. 느린 WS 클라 → 버퍼 차면 연결 끊기(클라는 재연결+RESUME). 노드 간 → bounded + 백프레셔, drop 시 resync.

### D28. DB 접근 = sqlx
- `sqlx` (async + **컴파일타임 쿼리 검증** → 파라미터 바인딩 강제, D20과 일석이조). 마이그레이션 Phase 0부터.
- 메시지 파티셔닝: Phase 1~3 단일 테이블 + 인덱스 `(realm_id, id DESC)`. **Phase 4에서 Snowflake 시간 RANGE 파티셔닝**(월별).

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

### D33. 합의(consensus) 레이어 = 없음 (명시적 경계)
- **Raft 등 합의 없음.** 정적 config 해시링을 모든 노드가 신뢰.
- **split-brain/네트워크 파티션 처리 = 범위 밖.** (로컬 study 전제. Discord도 Realm 소유에 합의 미사용.)
- 포트폴리오엔 "왜 합의를 안 넣었는지 안다"로 서술.

### D34. 메시지 멱등성 = 클라 nonce
- at-least-once(D24) 재전송 중복 방지: 클라가 보내는 **nonce로 dedup**(Discord 방식).

### D35. 읽기 캐싱 = Realm 액터 인메모리
- Redis 없음 → **Realm 액터가 최근 N개 메시지 bounded 캐시**. 콜드 데이터는 Postgres 직격.

### D36. 프로토콜 버저닝 = 헤더 version 바이트
- 수제 바이트 프레임 헤더에 **version 1바이트**. 롤링 재시작/DST 재현 시 포맷 진화 대비.

### D37. 파일 첨부 = 로컬 FS (Phase 4)
- 로컬 파일시스템으로 시작. **MinIO(S3 호환)** 는 선택적 업그레이드.

### D38. 페이지네이션 = Snowflake 커서
- 메시지 히스토리 = `before`/`after`/`around` + Snowflake 커서.

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
- **Phase 5 — 스트레치**: Passkeys, CRDT 오프라인 동기화, gossip discovery, (선택) Voice 시그널링.

---

## 5. 열린 질문 (Open Questions) — 아직 안 정함

- [x] ~~**Q1. 기능 컷라인 / Voice**~~ → D21 + §4-R 로드맵으로 해결.
- [x] ~~**Q2. 공유 상태 위치 (팬아웃)**~~ → D12로 해결 (Realm-local). **남은 갈래: 전역 presence 메커니즘 미정** (gossip 후보).
- [x] ~~**Q3. 메시지 저장 스키마**~~ → D28로 해결.
- [x] ~~**Q4. 인증 방식**~~ → D14/D19로 해결 (PASETO+refresh, TOTP). 남은 갈래: OAuth2/봇토큰 범위는 Q1 기능 컷라인과 함께.
- [x] ~~**Q5. Rate limiting**~~ → D18로 해결 (Token Bucket).
- [x] ~~**Q6. 장애/복구 범위**~~ → D23으로 해결 (rehydrate + RESUME 재연결).
- [x] ~~**Q8. Frontend**~~ → D30으로 해결 (React+TS+Vite).
- [x] ~~**Q3 외 DB**~~ → D28.

### 남은 자잘한 디테일 (Phase 진입 시 결정)
- [ ] **Q7. 액터 supervisor/재시작 전략** (D23이 큰 틀, 세부는 Phase 2).
- [x] ~~**Q9. CLI 시나리오 스크립트 포맷**~~ → 해결: CLI `scenario` 서브커맨드(헤드리스 종단 자동 검증 — 가입→길드→WS구독→전송→MESSAGE_CREATE 수신, PASS/FAIL+exit code). 별도 스크립트 DSL 없이 코드 시나리오로 시작, 필요 시 후속 확장.
- [ ] **Q10. 검색 구현** — Postgres FTS 유력, Phase 4.
- [ ] **Q11. gossip discovery + 전역 presence** (D5/Q2 후속, Phase 3/5).

---

## 6. 독창성 포인트 (포트폴리오 어필)

- 자체 수제 바이너리 프로토콜 (raw TCP, length-prefix, 수제 직렬화)
- 풀 메시 노드 클러스터 + Consistent Hashing 기반 Realm 배치
- 수제 Actor 런타임 (tokio + mpsc), 길드=프로세스 모델링 (Discord 충실)
- 2단 라우팅 (세션 소유 vs Realm 소유)
- (후속 후보) CRDT 오프라인 동기화, SWIM gossip presence
