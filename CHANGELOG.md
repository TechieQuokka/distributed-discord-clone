# Changelog

버전 규칙(CLAUDE.md R3): 1.0.0 시작 · 수정 +0.0.1 · 새 기능 +0.1.0.
형식: `## [버전] - 날짜` + 변경 내역.

---

## [1.21.2] - 2026-06-14
### 테스트/품질
- **rest-api 통합 테스트 7개 추가** (이전 0개) — in-memory `Store`(8개 repo trait 구현) + axum `oneshot`으로 DB 없이 핸들러·`AuthUser` 추출기·권한 강제·에러 매핑 검증. 커버: 무토큰 401, 길드 생성+@everyone, 채널생성 MANAGE_CHANNELS(비멤버 403/owner 단축), 초대 redeem→멤버화·미존재 404, 역할 생성 권한상승 방지, 역할 부여→권한 획득, **히스토리 VIEW_CHANNEL 게이팅 회귀 테스트**(1.21.1 수정분). dev-dep `tower`/`serde_json`. 전 crate 합계 74개.
- 문서: `docs/api/rest.md`에 **구현 현황(Phase 1–3)** 블록 추가 — 실제 라우트(초대=길드 레벨, 역할 부여 PUT, 채널 권한 PUT, 히스토리/전송 권한 강제)를 청사진과 구분해 명시(R2 동기화).

## [1.21.1] - 2026-06-14
### 수정
- **검증 패스**: 메시지 히스토리 조회(`GET /channels/:id/messages`)가 멤버십만 검사하고 채널 권한을 무시하던 **불일치 수정** — 이제 `perm::require_in_channel`로 VIEW_CHANNEL + READ_MESSAGE_HISTORY 강제(D17, 전송 경로와 일관). 라이브 검증: @everyone VIEW_CHANNEL deny 시 히스토리 403, owner는 200. rest-api 1.3.0→1.3.1.
- clippy 정리(machine-applicable): gateway(collapsible if/let-chains)·auth(is_multiple_of)·domain(slice::from_ref). 동작 변화 없음. (잔여: transport tcp 리더 루프·cli gateway_client의 pre-existing 1줄씩은 의도적 보류.)
- 전 crate 테스트 67개 통과 + CLI scenario(D1) 재확인 + 초대/권한/오버라이드/히스토리 2유저 라이브 재검증.

## [1.21.0] - 2026-06-14
### 새 기능
- **채널 권한 오버라이드 (D17)** (Phase 3) — 채널별 역할/멤버 allow·deny. 길드 허용을 채널 deny가 덮어씀. 2유저 라이브 검증.
  - `domain`: `permissions::{OverwriteKind, ChannelOverwrite}` + `effective_channel_permissions`(오버라이드를 대상별[@everyone=realm/역할/멤버]로 골라 `compute_channel_permissions`에 적용). `ChannelOverwriteRepository` port + `RoleRepository::member_roles_with_ids` → `Store`. 테스트 +2(채널 deny→멤버 allow 복구, 역할 오버라이드).
  - `storage`: **V6 `0006_channel_overwrites.sql`**(+`overwrite_kind` enum). set(upsert)/list 구현 + member_roles_with_ids.
  - `rest-api`: `perm::effective_in_channel` + `PUT /channels/:id/permissions/:target_id`(MANAGE_ROLES).
  - `gateway`: `can_send`를 **채널 컨텍스트**로 전환(오버라이드 반영).
  - `cli`: `set-channel-perm`.
  - **라이브 e2e**: @everyone deny SEND_MESSAGES → bob 전송 403 → bob 멤버 overwrite allow → 전송 성공(멤버 최우선). domain/storage/rest-api 1.2→1.3, gateway 1.3→1.4, cli 1.2→1.3.
- 문서: decisions D17(채널 오버라이드 구현), TODO 체크.

## [1.20.0] - 2026-06-14
### 새 기능
- **역할/권한 (D17) — 비트마스크 + DB 역할 + 강제** (Phase 3) — @everyone 기본 + 커스텀 역할로 행동 게이팅. 2유저 라이브 검증.
  - `domain`: `role::{Role,NewRole}`(@everyone=id==realm 규약) + `permissions::default_everyone`/`compute_guild_permissions` + `RoleRepository` port + `GuildRepository::get_guild` → `Store` 합류.
  - `storage`: **V5 `0005_roles.sql`**(roles + member_roles). 길드 생성 트랜잭션에 `@everyone` 역할 자동 삽입. RoleRepository 구현 + get_guild. DB 통합 테스트 +1(역할 할당 전후 유효권한).
  - `rest-api`: `perm::{effective,require}`(DB→domain 계산). 역할 라우트(`POST/GET /guilds/:id/roles`, `PUT /guilds/:id/members/:uid/roles/:rid`, MANAGE_ROLES+권한상승 방지). 강제 추가: 채널생성 MANAGE_CHANNELS·초대 CREATE_INVITE.
  - `gateway`: 메시지 전송에 `can_send`(SEND_MESSAGES) 강제(이전 is_member 대체). owner/Administrator 단축.
  - `cli`: `create-channel`/`create-role`/`assign-role`.
  - **라이브 e2e**: bob(@everyone)→create-channel 403 → alice가 MANAGE_CHANNELS 역할 생성·부여 → bob create-channel 성공. domain/storage/rest-api 1.1→1.2, gateway 1.2→1.3, cli 1.1→1.2.
- 문서: decisions D17(구현), TODO 체크(채널 오버라이드는 다음). 채널 오버라이드 계산은 domain에 이미 존재(저장·로딩만 후속).

## [1.19.0] - 2026-06-14
### 새 기능
- **초대(invites) — 멀티유저 합류** (Phase 3 시작) — 초대 코드로 길드 합류 → 자동구독(D13) → 크로스유저 팬아웃. 2유저 라이브 검증 완료.
  - `domain`: `invite::{Invite, NewInvite}`(+`is_valid`) 엔티티 + `InviteRepository` port(create/find/redeem) → `Store` 슈퍼트레잇에 합류. 테스트 +2.
  - `storage`: `PgStore` invite 구현 + **V4 `0004_invites.sql`**(청사진 스키마와 일치). `redeem_invite`는 **한 트랜잭션**(행 `FOR UPDATE` → 만료/소진 검사 → 멤버 멱등 삽입 → uses++). DB 통합 테스트 +1(생성·redeem·멱등·소진·만료·미존재).
  - `rest-api`: `POST /guilds/:id/invites`(멤버 전용 생성, base62 8자 CSPRNG 코드) + `POST /invites/:code`(redeem→멤버 추가→채널목록). `rand` 의존 추가.
  - `cli`: `create-invite`/`join` 서브커맨드 + rest 헬퍼.
  - **라이브 e2e**: alice 길드 생성→초대 발급, bob join→READY가 그 길드 멤버로 표시→alice 전송 시 bob WS가 MESSAGE_CREATE(s=2) 수신. domain/storage/rest-api/cli 1.0.0→1.1.0.
- 문서: TODO Phase 3 invites 체크. (스키마 `invites`는 청사진에 기존재 — 구현이 그에 일치.)

## [1.18.0] - 2026-06-14
### 새 기능
- **DST 하네스 — SimTransport + SimClock + 시드 카오스** (Phase 2, D25) — 멀티노드 클러스터를 단일 프로세스·가상 시간에서 결정론적으로 재현.
  - `transport::sim`: `SimNetwork`(가상 시계 + 시간순 BinaryHeap 스케줄 + 노드별 ready 큐), `SimTransport`(`NodeTransport`; `send`는 즉시 큐 적재), `DetRng`(splitmix64 시드 PRNG). 카오스: 지연(min/max_latency_ms)·유실(drop_prob)·파티션(partition/heal). 하네스 API: `advance_to`/`advance`/`next_event_time`/`take_inbound`/`dropped`. 테스트 +5(지연 보류, 동일시드 동일순서, 전량 유실, 파티션 격리, 미지 노드).
  - `node`: `Router`·`RealmActor`가 `Arc<dyn Clock>` **주입**받음(하드코딩 SystemClock 제거) → DST에서 Snowflake id까지 결정론(D11/D25). `Router::new` 시그니처에 clock 추가(server·테스트 갱신).
  - `node/tests/dst.rs`: 하네스 e2e — 동일 시드 2회 동일 결과(메시지 id+배달) 재현성, 노드2 파티션 시 팬아웃 유실. SimClock=`ManualClock`.
  - 후속: 액터까지 단일스레드 가상 실행기로 돌리는 완전 결정론(현재 네트워크 경로만 가상시간). transport 1.0.0→1.1.0, node 1.1.0→1.2.0.
- 문서: decisions D25(구현), TODO Phase 2 DST 체크. **Phase 2 분산 활성화 전 항목 완료.**

## [1.17.0] - 2026-06-14
### 새 기능
- **Backpressure — 느린 WS 클라 끊기 정책** (Phase 2, D27) — 채널 가득 시 침묵 드롭 대신 연결을 끊어 RESUME 복구 유도.
  - `gateway::hub`: `SessionEntry::push_live` — `try_send` 실패(느린 클라로 채널 가득/닫힘) 시 **live sender drop** → 세션 채널이 닫혀 `pump` 종료·소켓 close. 프레임은 재생 버퍼에 남아 재연결+RESUME으로 복구(D24). `deliver`/`dispatch_one`이 이를 사용(이전: try_send 침묵 드롭).
  - 노드↔노드(TcpTransport peer writer 256 + send().await)·액터 메일박스(256)는 이미 bounded — 문서에 명시.
  - 테스트 +1: 안 읽는 세션이 채널 채우면 끊기고(rx None) 버퍼 내 RESUME은 여전히 가능. gateway 1.1.0→1.2.0.
- 문서: decisions D27(구현), TODO Phase 2 backpressure 체크.

## [1.16.0] - 2026-06-14
### 새 기능
- **PING/PONG 생사 판정 + Realm 소유권 failover (rehydrate)** (Phase 2, D23) — 소유 노드 사망 시 Realm이 다음 살아있는 노드로 자동 이동.
  - `node::membership::Membership`: 피어 생사 뷰(`record_seen`/`mark_down`/`is_down`/`down_set`/`sweep`). 시간은 주입 clock(ms)로 다뤄 DST/테스트 결정론(D25).
  - `node::ring::HashRing::owner_excluding`: down 노드를 건너뛴 일관 해싱 소유권(영향 받는 Realm만 이동). `owner`는 이를 통해 membership down_set 소비.
  - `node::router`: `Router`에 `Membership` 보유 + `membership()` 노출. `handle_inbound`가 PING→PONG 회신, PONG/HELLO는 무처리(수신=liveness). `run_failure_detector`(주기 PING + sweep) 추가.
  - `server`: 멀티노드 시 failure detector spawn(interval 1s/timeout 3s) + inbound 루프가 `record_seen`(주입 clock)으로 liveness 갱신.
  - rehydrate: 새 소유 노드는 액터를 fresh-spawn(현재 액터 상태=휘발 구독자표 D12). 메시지 진실은 Postgres에 이미 persist(D24)되어 유실 없음. D35 캐시 warmup은 후속 seam.
  - 테스트 +6: membership sweep/recover·record_seen, ring failover(영향 Realm만 이동·전부 down→None), router 소유권 failover+복귀. node 1.0.0→1.1.0.
- 문서: decisions D23(구현), TODO Phase 2 rehydrate 체크.

## [1.15.0] - 2026-06-14
### 새 기능
- **Gateway RESUME — per-session seq + 재생 버퍼 완성** (Phase 2, D24/D20) — 끊긴 세션을 놓친 이벤트와 함께 재개.
  - `gateway::hub`: Hub가 **세션별 영속 상태**(user_id, 단조 seq, bounded 재생 버퍼[기본 256, D27], live sender, CSPRNG `resume_token`)를 **소켓 수명보다 오래** 보유. seq 부여·버퍼 적재를 Hub로 단일화(세션 소유 노드 권위). `attach`/`activate`(READY가 seq=1로 먼저 가도록 팬아웃 활성화 분리)/`dispatch_one`/`deliver`(세션별 seq)/`detach`(live만 분리·버퍼 유지)/`resume`(토큰·seq 검증→누락 프레임)/grace(90s) purge.
  - `gateway::session`: 핸드셰이크를 IDENTIFY|RESUME 분기. RESUME = `resume_token`(D20)+last seq 검증 → 놓친 프레임 재생(원래 seq 보존) + `RESUMED`(t="RESUMED" dispatch). 버퍼 밖 gap·토큰 불일치·만료·미지 세션 → INVALID_SESSION(재IDENTIFY+REST 재조회). 끊김 시 `detach`로 버퍼 보존.
  - `gateway::protocol`: `Outgoing` Clone, `ResumeData`{session_id,token,seq}, `Outgoing::resumed`, READY에 `resume_token`.
  - `rand` 의존 추가(CSPRNG resume_token). 테스트 +5(seq 단조, 재생, 토큰 거부, evict gap 탐지, 미지 세션). 크로스노드 RESUME(다른 노드 재연결)은 버퍼가 노드 로컬이라 후속.
- 문서: gateway.md(§2 RESUME 흐름/payload/RESUMED·resume_token, READY), decisions D24(구현 현황) 갱신, TODO Phase 2 RESUME 체크.

## [1.14.0] - 2026-06-14
### 새 기능
- **raw TCP + mTLS 전송 + 멀티노드 메시 완성** (Phase 2 핵심, D3/D4/D5/D16) — in-process stub 교체, 2노드 실시간 크로스노드 채팅 라이브 검증.
  - `transport`: `tls`(rustls mTLS 설정 — 서버 `WebPkiClientVerifier`/클라 `with_client_auth_cert`, 공유 CA, `generate_mesh` dev 인증서, `init_crypto`), `wire`(TLS 스트림 위 길이접두사 NodeMessage I/O), `tcp::TcpTransport`(accept/dial[작은→큰 id, D4]/HELLO 핸드셰이크/피어별 writer 큐/재연결). 테스트 +2: 2노드 mTLS 메시지 교환, 신뢰 안 되는 CA 거부.
  - `protocol`: `RealmSend`/`RealmFanout`에 `channel_id`(+fanout `nonce`) — (1.13에서 추가됨, 전송 경로 일관).
  - `node`: 크로스노드 통합 테스트 — **실제 raw-TCP+mTLS 위에서** 구독 포워딩 + RealmFanout 배달(`cross_node_fanout_over_tcp_mtls`).
  - `auth`: `TokenKeys::export_hex`/`import_hex` — 멀티노드 **공유 PASETO 키**(노드 간 토큰 상호 검증, D14). 테스트 +1.
  - `server`: `CLUSTER_CONFIG`(TOML) 지정 시 멀티노드 — 링에 전 노드, `TcpTransport` listen+dial, 크로스노드 inbound 루프(`handle_inbound`→`deliver_local`). 단일노드는 무설정 기본 유지. dev 유틸 `gen-certs`/`gen-keys`. TLS·PASETO 키 env 로드.
  - `gateway`: `deliver_local` 공개(dispatch 드라이버 + 크로스노드 inbound 공용).
  - **라이브 메시 검증**: 노드1·노드2 mTLS 연결 수립 → 노드1 발급 토큰을 노드2가 수락(공유 키) → 노드1 WS 구독 + 노드2 REST 전송 → 노드1 WS가 MESSAGE_CREATE 수신 + DB persist. 단일노드 scenario 회귀 통과. 테스트 44.
- 문서: decisions D16(구현·멀티노드 키 공유) 갱신, TODO Phase 2(전송/풀메시) 체크, .env.example(CLUSTER/TLS/PASETO) 보강.

## [1.13.0] - 2026-06-14
### 새 기능
- **WS Gateway + 실시간 메시징 종단 완성** (Phase 1 거의 완료) — register→길드/채널→WS구독→전송→수신 라이브 검증.
  - `domain`: `channel`/`message`/`guild` 엔티티 + `GuildRepository`/`ChannelRepository`/`MessageRepository` port + **`Store` 슈퍼트레잇**(조합 루트 제네릭 1개로 통합, 제네릭 폭발 방지). repo `create` 메서드는 트레잇별 고유명(`create_user`/`create_refresh_token`/`create_channel`/`create_message`)으로 — 통합 store 모호성 제거.
  - `storage`: 모든 port를 **단일 `PgStore`** 가 구현(개념 모듈 분산). 마이그레이션 **V3 `members`**. nonce 멱등 persist(부분 유니크 ON CONFLICT), Snowflake 커서 페이지네이션, 길드 생성 트랜잭션(realm+guild+owner member).
  - `node`: 메시지 경로에 `channel_id`(+nonce) 관통 — `RealmCommand`/`RealmEvent`/`LocalDelivery`/`Router::route_send`/`fanout`/`handle_inbound` 및 `protocol`의 `RealmSend`/`RealmFanout` 와이어. `Router::new`는 주입된 `Arc<SnowflakeGenerator>` 사용(D11).
  - `rest-api`: `AppState<S: Store>`로 단순화. `AuthUser` 추출기(Bearer PASETO). 신규 라우트 `POST /guilds`, `POST /guilds/{id}/channels`, `GET /channels/{id}/messages`(D38).
  - `gateway`(신규 구현): WS 수명주기(HELLO/IDENTIFY/READY/HEARTBEAT/DISPATCH, gateway.md), 세션 레지스트리(`Hub`, D9 세션 소유), **dispatch 드라이버**(events→persist→fanout→세션 배달, D24), `POST /channels/{id}/messages`(전송은 REST, Discord 모델). 자동구독(D13).
  - `server`: 단일노드 분산코어(ring+in-process transport) + Router + dispatch 드라이버 spawn + REST·Gateway 라우터 merge.
  - `cli`: `create-guild`/`send`/`listen`(WS) + **헤드리스 `scenario`**(D1/Q9 — 종단 자동 검증 PASS/FAIL).
  - 라이브 검증: scenario PASS, DB persist 2행, nonce 중복 1건 거부(D34), 히스토리 최신순(D38). 테스트 40 통과.
- 문서: decisions D11(generator 주입)·D24(persist는 dispatch 드라이버, RESUME은 Phase 2) 갱신, Q9 해결, node-wire.md에 구현 서브셋 명시, TODO Phase 1 체크.

## [1.12.0] - 2026-06-14
### 새 기능
- **가입/로그인/refresh 종단 배선 완성** (Phase 1, D14/D15) — storage→REST→server→CLI 전 경로 실동작.
  - `storage`: `PgUserRepository`(create/find_by_username·대소문자 무시/find_by_id, 유니크 위반→Conflict) + `PgRefreshTokenRepository`(회전 + 재사용 탐지).
  - `domain`: `RefreshToken`/`NewRefreshToken` 엔티티 + `RefreshTokenRepository` port(create/find_active/find_by_hash/revoke/revoke_all_for_user) + `RefreshTokenId`.
  - 마이그레이션 **V2 `0002_refresh_tokens.sql`** — 청사진 `docs/database/02-schema.md §1`의 `refresh_tokens`와 일치(token_hash 해시저장, rotated_from 체인, TIMESTAMPTZ 만료).
  - `rest-api`(axum 0.8): `POST /auth/{register,login,refresh}`. access=PASETO v4.public, refresh=opaque+회전. Argon2id 해싱은 `spawn_blocking`. **재사용 탐지(D14)**: 폐기된 refresh 재제시 시 유저 토큰 체인 전체 무효화. 제네릭 `AppState<R, T>`(repo trait가 RPITIT라 dyn 불가).
  - `server`: `.env`(dotenvy)+PgPool+마이그레이션+`TokenKeys`+노드당 단일 `SnowflakeGenerator`(D11) 배선 → HTTP 서빙.
  - `cli`(clap+reqwest): `register`/`login`/`refresh` 서브커맨드 — 헤드리스 시연(D1).
  - 라이브 종단 검증: register(201)/중복(409)/login(200)/오답(401)/refresh 회전(200)/재사용 탐지로 체인 전체 무효화(401). storage 통합테스트 +2. 총 테스트 39 통과.
- `Router::new`가 worker_id 대신 주입된 `Arc<SnowflakeGenerator>`를 받도록 변경 — server가 generator 1개를 소유해 Router·REST에 동일 인스턴스 주입(D11 노드 전역 유일성 일관).

## [1.11.2] - 2026-06-14
### 수정
- **Snowflake 전역 유일성 버그 수정** (D11 불변식 `1 worker_id = 1 generator`): Realm 액터마다 `SnowflakeGenerator`를 따로 만들어 같은 노드의 두 Realm이 동일 worker_id·동일 ms에 동일 ID를 발급할 수 있던 문제. `SnowflakeGenerator`를 lock-free(`AtomicU64` packing + CAS, `&self next()`)로 바꾸고 **노드당 1개**를 `Arc`로 모든 Realm 액터에 주입. `RealmActor::new`/`Router`가 worker_id 대신 주입된 generator 사용. 회귀 테스트 추가(동시 발급 유일성 + 두 Realm 비충돌). domain 6 + node 10 통과. decisions.md D11 갱신.
- CHANGELOG `[1.6.0]` 블록 위치 오류 정정(시간순 복원).

## [1.11.1] - 2026-06-14
### 수정
- `RESUME.md` 추가 (다음 세션 온보딩: 문서 읽기 순서 → 빌드/DB → 다음 작업 → 규칙). CLAUDE.md 상단에 RESUME 포인터.

## [1.11.0] - 2026-06-14
### 새 기능
- `domain`에 User 엔티티 + `UserRepository` port(repo) 추가.
- `auth` crate (개념 모듈 password/token/refresh/error): Argon2id 해싱, PASETO v4.public(Ed25519) access 토큰 발급·검증, opaque refresh 토큰(SHA-256). 테스트 6개 통과. (P6: 크립토는 argon2/pasetors 검증된 크레이트)

## [1.10.0] - 2026-06-14
### 새 기능
- **크로스노드 팬아웃 완성** (분산 메시지 경로 정점): protocol에 `RealmSend`/`RealmFanout` 와이어 추가. `node::router`에 `route_send`(로컬/원격 fire-forget), `fanout`(노드별 그룹화 → 로컬 `LocalDelivery` + 원격 `RealmFanout` 전송), `handle_inbound`(Subscribe/RealmSend/RealmFanout 처리). 종단 테스트 `cross_node_fanout_end_to_end`: 두 노드에 흩어진 구독자에게 한 메시지 팬아웃 검증. protocol 6 + node 9 테스트.

## [1.9.0] - 2026-06-14
### 새 기능
- `node::router` — 2단 라우팅(D9): ring으로 Realm 소유 노드 판정, 로컬은 Realm 액터(lazy spawn)로 디스패치·원격은 transport 포워딩. `route_subscribe`(로컬/원격), `route_send_local`. 테스트 3개(총 node 9개). 원격 SendMessage 포워딩(REALM_COMMAND 와이어)은 후속.

## [1.8.0] - 2026-06-14
### 새 기능
- `node` crate (개념 모듈 ring/realm/clock): **consistent hashing 링**(splitmix64+vnode, D6 — 균등분산·최소재배치 성질 테스트로 증명), **Realm 액터**(단일소유 순서보장 D24, 구독자표 D12, 인메모리 팬아웃 이벤트), **Clock 주입**(SystemClock/ManualClock, DST D25). 테스트 6개 통과.

## [1.7.0] - 2026-06-14
### 새 기능
- `cluster-config` crate: TOML 정적 클러스터 설정(노드 id/worker_id/listen_addr + peers), 검증(worker 범위·self/중복 피어), `peers_to_dial`(풀메시 쌍당 1연결 D4). 테스트 3개. 예시 `backend/config/cluster.example.toml`.

## [1.6.1] - 2026-06-14
### 수정
- DB 셋업(role david + DB discord_v1) 후 마이그레이션 V1 **적용 완료** (테이블 6개). storage 통합 스모크 테스트(migrate + user 라운드트립) 통과. `.env`/`.env.example`에 소켓 DATABASE_URL(percent-encoded host) 확정.

## [1.6.0] - 2026-06-14
### 새 기능
- `storage` 골격 (개념 모듈 pool): sqlx 0.9 연결 풀 `connect` + `run_migrations`(임베드). 마이그레이션 V1 `0001_init.sql`(users/realms/guilds/channels/messages + enum). DB 없이 컴파일 검증. 실제 적용은 DB 셋업 후.

## [1.5.0] - 2026-06-14
### 새 기능
- `server` 실행 골격: `#[tokio::main]` + `tracing`/`tracing-subscriber`(env-filter) 초기화 (D26). 구조적 로그 출력 확인.

## [1.4.0] - 2026-06-14
### 새 기능
- `transport` 구현 (개념 모듈 iface/stub): `NodeTransport` trait + in-process `Switchboard`/`InProcessTransport`(DST·Phase0 배선용). 테스트 3개 통과. raw-TCP+mTLS는 Phase 2.

## [1.3.0] - 2026-06-14
### 새 기능
- `protocol` 구현 (개념 모듈 codec/frame/message): 수제 바이트 코덱(BE+길이접두사), 28바이트 프레임 헤더(version/trace_id), `NodeMessage`(Hello/Ping/Subscribe 등) 인코딩·디코딩. 테스트 4개 통과(SUBSCRIBE=56바이트 doc 교차검증 포함).

## [1.2.0] - 2026-06-14
### 새 기능
- `actor-rt` 구현 (개념 모듈 actor/mailbox): 수제 액터 런타임(tokio+mpsc), bounded 메일박스(백프레셔 D27), `Actor` trait + `spawn`. 테스트 2개 통과.

## [1.1.0] - 2026-06-14
### 새 기능 / 구조
- **Phase 0 스캐폴딩 시작**: 최상위 `backend/`·`frontend/` 분리.
- backend를 **독립 crate 구조**로 구성 (umbrella 워크스페이스 없음, 각 crate 독립 빌드/관리 — R7): domain/protocol/actor-rt/transport/storage/node/gateway/rest-api + bins server/cli.
- `domain` 코어 구현 (개념별 디렉터리 R6): `id`(Snowflake+엔티티 id, 주입형 clock), `permissions`(비트마스크+계산 D17), `error`. 단위 테스트 4개 통과.
- CLAUDE.md 규칙 추가: R6(개념=디렉터리), R7(독립 빌드/관리).

## [1.0.5] - 2026-06-14
### 수정
- DB 스키마 리뷰/보강: `applications`(봇/토큰), `user_settings`, `forum_tags` 추가 + 보류 항목 명시 (`docs/database/02-schema.md` §9).

## [1.0.4] - 2026-06-14
### 수정
- 권한 시스템 문서 추가: `docs/architecture/permissions.md` (비트 레이아웃, 계산 알고리즘, 강제 지점).

## [1.0.3] - 2026-06-14
### 수정
- 노드 간 와이어 프로토콜 명세 추가: `docs/protocol/node-wire.md` (길이접두사 프레이밍, 28B 헤더, msg_type 카탈로그, 바디 레이아웃, mTLS 핸드셰이크, 워크드 예시).
- README 인덱스 Protocol 섹션 연결, TODO 항목 체크.

## [1.0.2] - 2026-06-14
### 수정
- API 설계 문서 추가: `docs/api/rest.md`(REST 엔드포인트 카탈로그), `docs/api/gateway.md`(Gateway 이벤트/opcode 카탈로그).
- README 인덱스에 API 섹션 연결, TODO 해당 항목 체크.

## [1.0.1] - 2026-06-14
### 수정
- CLAUDE.md에 개발 규칙 추가: R4(작업 순서 backend+API→CLI→web UI, web UI 후순위), R5(TODO 체크 규율 + 수정 시 승인).
- TODO.md에 체크용 규율 및 작업 순서 명시.

## [1.0.0] - 2026-06-14
### 설계 베이스라인 (코딩 전)
- 문제분석 단계 완료 — 아키텍처 결정 D1~D38 + DB 모델링 DB-D1~D6 확정.
- 문서군 작성: `docs/README.md`, `architecture/decisions.md`, `design-discussion.md`, `database/01~04`.
- `TODO.md`(6단계 로드맵), `CLAUDE.md`(개발 규칙), `CHANGELOG.md` 추가.
- 확정 스택: Rust(edition 2024)/tokio, PostgreSQL+sqlx, raw TCP+mTLS 수제 프로토콜, Actor 모델, PASETO 인증, React+TS.
