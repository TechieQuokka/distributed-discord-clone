# 설계 토론 기록 (Design Discussion Log)

> 문제분석 단계에서 오간 **논쟁의 서사**. decisions.md가 "무엇을 정했나"라면, 이 문서는 **"어떤 선택지를 두고 어떻게 다퉈서 그렇게 정했나"** 를 그대로 남긴다.
> 작성 시점: 2026-06-14. 진행 방식: 사용자 제안 → Claude가 근거로 반박/대안 → 접점 → 결정.

---

## 0. 출발점과 철학

- 사용자: Discord와 거의 동일한 backend(Rust) + frontend web(TS 등) 구현. **공부 + 포트폴리오**, 가능한 전 기능 카피.
- 전부 **로컬 테스트 범주**, 이론 확립이 목적. 단 철학은 **"공부는 실전처럼"** → 수만 동접 감당 구조.
- 검증: 로컬에서 수십 개 테스트 클라이언트(일부는 Claude가 직접 구동).
- 사용자 배경: git 프로젝트 ~300개, 숙련. DB는 PostgreSQL(이미 설치).
- **지난 실패 교훈**: 이전 유사 프로젝트가 "코드 내부적으로 꼬여서" 실패. → 이번엔 **청사진 먼저**.

> Claude 초기 반박: "거의 전부 카피"와 "공부용"은 범위상 충돌 가능. → 사용자: 숙련됐으니 걱정 말고 논의하자. → 범위 우려 접고 설계 논쟁 모드로 전환.

---

## 1. SCC(강한 연결 요소)로 유저 그룹 만들기 — 기각

- 사용자: 독창성을 위해 SCC 개념으로 유저 그룹을 만들고 싶다.
- **Claude 반박 3가지**:
  1. SCC는 *방향 그래프*에서만 의미. Discord 친구는 상호(무방향) → SCC가 연결요소로 퇴화.
  2. 소셜 그래프는 거대 연결 요소로 뭉침(small-world) → "그룹 1개에 전원" 무용지물. 임계값 필요.
  3. 실시간 그래프 변경마다 재계산 비용(증분 SCC는 어려움).
- 대안 제시: 정말 원하는 게 커뮤니티 탐지라면 SCC보다 **Louvain 등 community detection**이 정석.
- **결정: SCC 기각.** 독창성은 다른 축으로.

## 2. 독창성의 축 — 인프라 vs 기능

- Claude가 두 갈래 제시:
  - **A. 인프라**: Consistent Hashing, Gossip(SWIM), HyperLogLog, CRDT, Token Bucket.
  - **B. 기능**: Community Detection, PageRank, BFS 친구추천, Inverted Index.
- Claude 추천: A의 **Consistent Hashing** (테마 "수만 동접 분산"과 일치, 실제 Discord 인프라).
- **결정: A — 분산 인프라 정공법.** 테마 한 줄: *"단일 노드로 못 버티는 규모를 여러 노드로 나눠 감당하는 구조를 직접 구현·증명."*

## 3. 노드 간 팬아웃 수단 — raw TCP 직접 통신

세 갈림길:
1. 순수 Postgres `LISTEN/NOTIFY` — 수만 팬아웃엔 부적합 → 테마 배신, 기각.
2. Postgres + Redis/NATS 브로커 — 진짜 분산이지만 어려운 부분을 브로커가 대신 → 배울 게 줄어듦.
3. **노드 간 직접 통신(mesh)** — 가장 밑바닥, 가장 공부됨.

- Claude 추천: (3). "Redis에 얹으면 어려운 부분을 Redis가 해줘서 정작 못 배운다."
- **결정: (3) 노드↔노드 raw TCP + 자체 프로토콜.** 브로커 없음.

## 4. 전송 계층 세부

- **엣지 강제 사실**: 프론트가 web → 브라우저는 raw TCP 불가 → **클라↔Gateway = WebSocket** 필연. (raw TCP는 노드↔노드 전용)
- 토폴로지: **풀 메시** (로컬 노드 3~10개엔 압도적 단순). vs 코디네이터(SPOF, 기각).
- 발견: **정적 config** (v1), gossip은 후속.
- 직렬화: **수제 바이트 레이아웃** (bincode/protobuf 아님) — 사용자 "밑바닥부터" 성향.
- 프레이밍: **길이 접두사**(4바이트+페이로드) — TCP 스트림 경계 문제.
- **원칙 합의(P2/P3)**: 전송은 `trait` 뒤에 격리, in-process stub부터. → 지난 실패(코드 꼬임) 방어.

## 5. 노드 내부 동시성 — Actor 모델

- (a) **Actor**: Realm 하나 = 액터 하나, 상태 독점, 락 없음, 메시지로만 소통. Erlang Discord 방식.
- (b) 공유 상태 + 락(`Arc<RwLock<...>>`): "정확히 사용자를 망쳤을 패턴" — 락 경합/데드락/추적 지옥.
- Claude 추천: (a). 이유 — ①코드 꼬임을 구조적으로 차단 ②Discord 충실 ③원격 액터에게 메시지 보내기 = raw TCP 전송과 자연 연결.
- 구현: **외부 크레이트 없이 tokio + mpsc 수제** (사용자 일관성).
- **결정: Actor 모델, tokio+mpsc 수제.**

## 6. 샤딩 단위 — 길드에서 Realm으로

- 1차: **길드 단위** 샤딩(Discord 정답, 지역성 단위).
- 발견된 구멍: **DM/그룹DM은 길드가 없다** → `hash(guild)` 불가 → 특수 케이스가 코드에 번질 위험(=지난 실패 패턴).
- Claude 제안: 샤딩 단위를 **"Realm"** 으로 추상화 — 길드/DM/그룹DM을 전부 한 추상으로 통일. 소유 노드 = `hash(realm_id)`.
- **결정: Realm 1급 추상.** DM 특수 케이스 소멸.
- 동반: **2단 소유권** — 세션 소유(클라가 접속한 노드) vs Realm 소유(`hash(realm_id)`).

## 7. 팬아웃 위치 추적 — "구독자가 어느 노드에?"

- (a) 전역 세션 레지스트리(모든 노드 풀맵) — 수다스러움/중복.
- (b) 유저-해시 조회 — 팬아웃마다 RTT 폭증, 느림.
- (c) **Realm 액터가 자기 구독자의 노드를 직접 추적** — 팬아웃 시 조회 0번, 액터 모델과 합치.
- Claude 추천: (c). + **팬아웃 위치추적과 전역 presence를 분리** (presence는 Realm 무관 전역 상태).
- 구독 모델: **입장 시 자동 구독(Discord식)**.
- **결정: (c) + presence 분리 + 자동구독.**

## 8. 보안 논쟁

### 8.1 노드 간 신뢰 (아키텍처 고유 위협)
- raw TCP 포트에 누구나 붙어 가짜 프레임 주입 가능 → 엣지 인증을 뚫는 뒷문.
- (A) 공유 시크릿 + HMAC / (B) **mTLS** / (C) 핸드셰이크 1회.
- **결정: (B) mTLS** (rustls, TLS1.3, 로컬 self-signed CA). mTLS가 채널을 인증·암호화하므로 HMAC 불필요.

### 8.2 사용자 토큰 — JWT 반박 → PASETO
- 사용자: JWT(많이 쓰니까).
- Claude 반박: JWT는 ①폐기 불가 ②장수명 WS 연결과 충돌. 그리고 `alg` 혼동 등 레거시 함정.
- 현대 정석: **짧은 access(PASETO v4.public, Ed25519, 15분) + opaque refresh(DB저장, 회전+재사용탐지)**.
- 사용자: "최신 기술 많이 적용하고 싶다" → PASETO가 더 부합.
- **결정: PASETO + Ed25519 + access/refresh 회전.**

### 8.3 나머지
- 비번 **Argon2id**, 전송 **TLS1.3(rustls)**, 인가 **Discord 비트마스크**, 봇방지 **PoW 챌린지(mCaptcha식) + Token Bucket**, MFA **TOTP 코어 + Passkeys 스트레치**.
- **원칙 P6 추가**: 암호화는 직접 짜지 않는다 — 크립토 프리미티브는 검증된 크레이트. ("수제"는 전송/액터에만.)

## 9. 기능 컷라인 & 로드맵

- **헤드라인**: Voice/Video(WebRTC/SFU/코덱)는 나머지 백엔드만큼 큰 별세계 + 테마와 결 다름 → **미디어 out, 시그널링만 설계**.
- 평면 in/out 대신 **6단계 로드맵**으로 분해 (Phase 0 뼈대 → Phase 5 스트레치). 단일노드(Phase1) → 분산 활성화(Phase2) 순.
- **결정: Voice out + 6-phase 로드맵.**

## 10. 빠진 논의 M1~M9 (Claude 자문자답 → 사용자 일괄 승인)

| # | 쟁점 | 결정 |
|---|---|---|
| M1 | crate 경계 | **헥사고날 9-crate**, domain 중심, 의존성 안쪽으로 (컴파일러가 꼬임 차단) |
| M2 | 상태/복구 | Postgres=진실, 액터=재구축 캐시, 재배치 시 rehydrate (이벤트소싱은 스트레치) |
| M3 | 전달/순서 | Realm 단일소유=순서 무료, persist-then-fanout, per-session seq+재생버퍼, REST 폴백 |
| M4 | 테스트 | 유닛 + **DST(결정론적 시뮬레이션)** + 소수 e2e |
| M5 | 관찰성 | `tracing` + 노드간 trace-id 전파 (Phase 0부터) |
| M6 | backpressure | 전부 bounded, 느린 클라 끊기 |
| M7 | DB | `sqlx`(컴파일타임 검증), 파티셔닝은 Phase 4 |
| M8 | 멀티노드 실행 | dev 런처(xtask/just) + 노드별 worker-id |
| M9 | frontend | React+TS+Vite (SvelteKit 대안) |

## 11. 마감 엣지케이스 G1~G8 (Claude 자문자답 → 일괄 승인)

| # | 쟁점 | 결정 |
|---|---|---|
| G1 | 클라 페이로드 | **JSON**(엣지) vs 수제 바이트(노드) — 이중 직렬화 |
| G2 | 분산 rate limit | per-node 근사 시작, 필요한 곳만 유저-해시 정밀화 |
| G3 | 합의(consensus) | **없음을 명시** — Raft 미사용, split-brain은 범위 밖 |
| G4 | 멱등성 | 클라 nonce dedup |
| G5 | 읽기 캐싱 | Realm 액터 최근메시지 bounded 캐시 |
| G6 | 프로토콜 버저닝 | 프레임 헤더 version 바이트 |
| G7 | 파일 첨부 | 로컬 FS 시작, MinIO 선택 |
| G8 | 페이지네이션 | Snowflake 커서(before/after/around) |

## 12. DB 설계 (별도 문서군)

- 핵심 모델링 결정 DB-D1~D6: Realm 하이브리드 표현 / 1:1 DM 합성키 재조정(`dm_pairs`) / 메시지→채널→Realm 단일경로 / 권한 오버라이드 / **휘발 상태 DB 배제** / nonce 멱등성.
- 상세: [database/01-overview.md](database/01-overview.md) 이하.

---

## 메타: 이 프로젝트가 지난 실패를 막는 법

논쟁 내내 반복된 한 가지 — **"또 꼬일까 봐"**. 그 방어선을 구조에 박았다:
- 경계를 **컴파일러가 강제**(헥사고날 crate, P2).
- 어려운 구현은 **stub 뒤로**(P3) — 기능부터 굴리고 나중 교체.
- **특수 케이스를 추상화로 흡수**(Realm, P4).
- **상태를 액터에 격리**(P5), **휘발 상태를 DB에서 배제**(DB-D5).
- 결정은 전부 **문서로 고정**(이 폴더) — 의지력이 아니라 기록에 의존.
