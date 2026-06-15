# RESUME — 다음 세션 이어서 작업

> 이 파일 하나로: **문서 읽기 → 코드 검토 → 작업 재개**. (AI/사람 공용 온보딩)

---

## 1. 먼저 읽기 (순서)

1. **`CLAUDE.md`** — 개발 규칙 R1~R7 + 핵심 원칙 P1~P6. **필수.**
2. `docs/README.md` — 문서 인덱스
3. `docs/architecture/decisions.md` — 결정 원장 D1~D44(+ 정제 갱신, Q1~Q11) (왜 이렇게 만들었나 = source of truth)
4. `TODO.md` — 진행 상태 (`[x]` 완료 / `[~]` 진행중 / `[ ]` 미착수)
5. `CHANGELOG.md` — 최근 한 일 (최상단이 최신)

필요 시 깊게: `docs/design-discussion.md`(논쟁 서사), `docs/database/*`, `docs/api/*`, `docs/protocol/node-wire.md`, `docs/architecture/permissions.md`.

## 2. 현재 상태 (2026-06-16, v1.38.0)

- 설계 문서 + Phase 0/1 + **Phase 2(분산 활성화) 완료** + **Phase 3(Discord 본체) 완료** + **Phase 4(살붙이기) 완료**.
- **Phase 4 완료(v1.33~1.38)**: 인증/봇방지(PoW D18·rate limit D32·TOTP MFA D19) + **전문검색(Q10, FTS V12)** + **스레드/포럼(D44, V13)** + **파일첨부(D37, V14 + BlobStore 로컬 FS)** + **웹훅(V15)** + **감사로그(V16)** + **메시지 RANGE 파티셔닝(D28, V17 — nonce 멱등은 앱레벨 dedup으로 이전)**. 각 기능 라이브 e2e 검증. **다음: Phase 5 스트레치 또는 frontend(D30).**
- Phase 4(1.38, D28): **메시지 RANGE 파티셔닝** — `messages PARTITION BY RANGE(id)` 월별+DEFAULT(V17, 드롭&재생성). 인덱스/FTS GIN 부모 정의→상속. **D34 nonce 멱등을 DB 부분유니크→앱레벨 dedup**(파티션 유니크 제약, create_message 가드 INSERT, dispatch 단일 소비자라 레이스 없음). 첨부 FK 유지(PG12+). 파티션 라우팅+nonce dedup 라이브 검증.
- Phase 4(1.37): **감사로그** audit_log_entries(V16) — 채널/역할/멤버/웹훅 mutation best-effort 기록 + `GET /guilds/:id/audit-logs`(VIEW_AUDIT_LOG).
- Phase 4(1.36): **웹훅** webhooks(V15) — opaque 토큰(SHA-256 해시, auth 재사용) + `POST/GET /channels/:cid/webhooks`·`DELETE /webhooks/:id`·실행 `POST /webhooks/:id/:token`(Bearer 없음, persist+MESSAGE_CREATE 액터 우회).
- Phase 4(1.35, D37): **파일첨부** attachments(V14) 메타 + `BlobStore` 포트(LocalFsBlobStore 바이트). 사후 첨부 `POST/GET /channels/:cid/messages/:mid/attachments`·`GET /attachments/:id`.
- Phase 4(1.34, D44): **스레드/포럼** — 스레드=channels(kind=thread,parent_id)+thread_meta(V13), 메시징/팬아웃/권한 길드 경로 재사용(P4). `POST/GET /channels/:id/threads`·`PATCH .../thread`. forum=kind.
- Phase 4(1.33, Q10): **전문검색** — messages content_tsv(STORED)+GIN(V12), `websearch_to_tsquery`, `GET /guilds/:id/messages/search`(VIEW_CHANNEL 채널 필터).
- Phase 4(1.32, D19): **TOTP MFA** — 2단계 인증(RFC 6238). `auth::totp`(totp-rs, P6). secret은 `users.mfa_totp_secret`(BYTEA, **V1 기존 → 마이그레이션 0**), 민감값이라 전용 포트(`set_totp_secret`/`totp_secret`). 흐름: `enable`(발급·미저장) → `verify`(확인 시 저장=활성, **락아웃 방지**) → `disable`. 로그인은 MFA 활성 시 `{mfa_required:true}` → `POST /auth/mfa/totp`(비번+코드)로 토큰. cli `mfa-enable/mfa-verify/mfa-login/totp-code`. **라이브 e2e**(enable→verify→login mfa_required→2단계 토큰, 틀린 코드 401). seam: 2단계 비번 재제출·백업코드·WebAuthn은 Phase 5. 다음: 스레드/검색 또는 파티셔닝(D28).
- Phase 4(1.31, D32): **Rate limiting (Token Bucket per-node)** — 봇/폭주 방지. 순수 `TokenBucket`(용량+초당 리필) + per-node `RateLimiter`(`rule:identity` 버킷, **인메모리 DB-D5 휘발**, 분산 근사). REST 미들웨어 — `/auth/*`=전역·인증 유저별·미인증 anon, 초과 시 **429** + `X-RateLimit-*`/`Retry-After`. server `with_defaults`(auth20·user120·anon60) 주입. `rest-api::ratelimit`. **라이브 검증**(scenario 통과 + pow-challenge 폭주 정확히 20→429). seam: 노드별 독립(전역 정밀X)·메시지/WS 미적용·유저해시 승격(b) 후속. 다음: TOTP(D19) 또는 스레드/검색.
- Phase 4(1.30, D18): **가입 봇방지 PoW** — Phase 1 잔여이자 Phase 4 첫 항목. **stateless 멀티노드**: 챌린지를 저장하지 않고(DB-D5) **PASETO v4.local 토큰**으로 발급(난이도를 인증 claim에 담아 위변조 차단+만료 내장), 공유 키(`POW_SECRET`)로 어느 노드든 검증(D14 철학). 클라는 `sha256(challenge||":"||nonce)` 선행 0비트 ≥ 난이도(기본 18)를 푼다 — **퍼즐 해시만 수제(sha2), 챌린지 MAC은 pasetors(P6 준수)**. `auth::pow`(PowKeys/solve) · `GET /auth/pow-challenge` · `POST /auth/register`가 `pow_challenge`+`pow_nonce` 필수(400) · cli register가 자동 풀이(`auth::pow::solve` 재사용). **단일노드 `cli scenario` e2e PASS**. seam: 챌린지 미저장→만료(1h)까지 같은 해 replay 가능(게이트=난이도)·로그인 PoW·rate limit은 후속. 다음: rate limit(D32) 또는 TOTP(D19).
- Phase 3(1.29, D43): **크로스노드 유저 이벤트 라우팅** — D40/D41의 마지막 seam을 닫음. 유저 단위 이벤트(`RELATIONSHIP_*`/`MESSAGE_ACK`)가 로컬 노드 세션에만 가던 것을, 대상 유저가 **다른 노드**에 접속 중이어도 배달되게 일반화. **D42 `Presence`의 `user→호스팅 노드` 디렉터리를 재사용**(`nodes_for`) — presence처럼 broadcast가 아니라 **호스팅 노드에만 타깃 전송**(`USER_DELIVER` wire 0x0202). 어댑터=gateway `user_route::UserRouter`(Hub+Presence+Router, `RealmEmitter`=Router와 대칭), server가 주입. **포트 시그니처 불변 → rest-api 무변경**. `Hub::deliver`가 세션 없는 유저 자동 스킵 → stale 디렉터리 무해. **2노드 mTLS 라이브 검증**(node1 alice 친구요청 → node2 bob가 RELATIONSHIP_ADD 실시간 수신). 다음: Phase 4.
- Phase 3(1.28, Q11/D42): **전역 presence(gossip)** — 친구 온라인 여부. 휘발 레지스트리(`node::Presence`, DB-D5: user→(status, 호스팅 노드 집합)). gateway 세션 첫/마지막 live 세션 전이 시 `PRESENCE_GOSSIP`(wire 0x0201) **풀메시 broadcast** + 그 유저의 **로컬 친구**(relationships 재사용)에게 `PRESENCE_UPDATE` 배달. 수신 노드는 재브로드캐스트 안 함(루프 방지). READY에 친구 presence(`presences`) 포함. **D40/D41의 크로스노드 유저 라우팅 seam을 닫음.** domain/storage/rest-api 무변경. **단일+2노드 mTLS 라이브 검증**(node1 A 접속→node2 B가 PRESENCE_UPDATE 수신). 다음: Phase 4.
- Phase 3(1.27, D41): **읽음 상태(read_states)** — 채널별 `last_read_message_id` + 안 읽은 `mention_count`. ack(`POST /channels/:cid/messages/:mid/ack`)가 last_read 갱신 + 그 이후 멘션 수 재계산. mention_count는 dispatch가 멘션 적재 시 +1(작성자 제외) 증분 유지 + ack 재계산으로 정정. **READY 스냅샷에 read_states 포함**, ack 시 `MESSAGE_ACK`를 본인 세션들에 통지(`UserEmitter` D40 재사용, 다기기 동기화). wire V11. **라이브 검증**(멘션 2개→count 2→ack→0, READY 스냅샷, MESSAGE_ACK 수신). 다음: 전역 presence 또는 Phase 4.
- Phase 3(1.26, D40): **친구·차단(relationships)** — Discord식 방향성 행(친구 요청/수락/취소·삭제, 차단/해제, storage 트랜잭션 전이). **차단 시 1:1 DM 열기·전송 거부**(permissions.md §5 seam 닫힘). 친구·차단은 Realm 무관이라 **유저 단위 `UserEmitter` 포트**(Hub 구현, 로컬 세션에 `RELATIONSHIP_ADD/_REMOVE` 배달; 크로스노드 유저 라우팅은 Q11 seam). REST `GET/PUT/DELETE /users/@me/relationships`, wire V10. **라이브 검증**(요청→실시간 수신→수락→friend / 차단→DM 양방향 거부+전송 거부). 다음: 읽음 상태, 전역 presence.
- Phase 3(1.25, D8/DB-D2): **DM/그룹DM** — Realm 통일 추상(P4)의 쇼케이스. DM·그룹DM도 길드와 같은 `realms`+`channels`(+`members`)라 **메시징·권한·분산 팬아웃·자동구독(D13)을 길드와 무변경 공유**(gateway/node/protocol/server 변경 0). 1:1은 `dm_pairs`(V9) find-or-create(같은 두 사람=같은 채널), 그룹은 자체 realm(kind=group_dm,owner)+참가자 관리(소유자 추가/타인제거·본인탈퇴·소유자탈퇴불가). DM Realm은 @everyone 없어 권한이 `default_everyone` 폴백 → 멤버면 전송·조회 통과. 신규 포트 `DmRepository`, REST `POST /users/@me/channels`·`PUT/DELETE /channels/:id/recipients/:uid`, `CHANNEL_RECIPIENT_ADD/_REMOVE` 팬아웃. **1:1+그룹 라이브 검증**(상대 READY 자동구독→MESSAGE_CREATE 수신). 다음: 친구·차단, 읽음 상태.
- Phase 3(1.24, D39): **답장**(`reference_message_id`를 송신 경로 전체에 관통: gateway→Router→RealmCommand→RealmEvent→`REALM_SEND` wire→NewMessage→payload, 같은 채널 검증) + **멘션**(content 파생 → dispatch가 `parse_mentions`로 뽑아 V8 `message_mentions` 적재+payload, 파이프라인 무변경). 다음: DM/그룹DM, 친구·차단, 읽음 상태.
- Phase 3(1.23, D39): **메시지 편집/소프트삭제/리액션** — D39 범용 envelope를 타고 `MESSAGE_UPDATE/_DELETE`, `MESSAGE_REACTION_ADD/_REMOVE` 실시간 통지(비-persist). V7 `reactions`.
- Phase 3(1.22, D39): **멤버 관리**(조회/nick수정/추방·탈퇴 REST, `@me`) + **범용 Realm 이벤트 팬아웃**(메시지 전용 경로를 `(t,payload)` envelope로 일반화). `domain::emit::RealmEmitter` 포트를 Router가 구현, server가 rest-api에 주입.
- Phase 3(1.19~1.21): **초대**(redeem→자동구독→크로스유저 팬아웃) + **역할/권한(D17)**(roles/member_roles, @everyone 기본) + **채널 오버라이드**(channel_overwrites, 채널 deny가 길드 허용 덮음) — 모두 **2유저 라이브 검증**.
- Phase 2 마감(1.15~1.18): **Gateway RESUME**(per-session seq+재생버퍼+CSPRNG resume_token, D24/D20) · **PING/PONG 생사판정+소유권 failover**(Membership+owner_excluding, D23) · **Backpressure**(느린 클라 끊기, D27) · **DST 하네스**(SimTransport+SimClock+시드 카오스, D25).
- **멀티노드 라이브 검증**: 노드1↔노드2 mTLS 연결 + 공유 PASETO 키 → 노드1 WS 구독 + 노드2 REST 전송 → 노드1이 MESSAGE_CREATE 수신(크로스노드 팬아웃). 단일노드 모드도 유지.
- 구조: `backend/`(rust, **독립 crate** — umbrella 워크스페이스 없음) + `frontend/`(web, 미착수) + `docs/`.
- crate: `domain` `protocol` `actor-rt` `transport` `storage` `cluster-config` `node` `auth` `rest-api` `gateway` + `bins/{server,cli}`.
- **분산 코어**: consistent hashing(ring) + Realm 액터 + 2단 라우팅 + 크로스노드 팬아웃. 전송 = **raw-TCP+mTLS**(`TcpTransport`, rustls) 또는 단일노드 무전송.
- **인증 종단**: `/auth/register|login|refresh` (PASETO + refresh 회전/재사용탐지 D14).
- **실시간 메시징 종단**: `PgStore`(통합 저장소, `Store` 슈퍼트레잇) → REST(`/guilds`, `/channels/:id/messages`, 히스토리 D38) → **WS Gateway**(IDENTIFY/READY/HEARTBEAT/DISPATCH, 자동구독 D13) → dispatch 드라이버(persist-then-fanout D24, nonce 멱등 D34) → 세션 push. CLI `scenario`로 종단 자동검증(D1).
- Snowflake generator는 **노드당 1개**(D11, lock-free CAS)를 server가 소유해 Router·REST·Gateway에 주입.
- 테스트 **138개** 통과 (domain 18 + gateway 8 + node 22 + protocol 9 + auth 18 + actor-rt 2 + transport 10 + cluster-config 3 + **rest-api 30** + **storage 18**). Phase 4 추가분: 검색·스레드·첨부·웹훅·감사로그 storage/rest-api 통합 + 파티션 라우팅. CLI scenario(PoW e2e) + 각 Phase 4 기능 라이브 e2e(검색/스레드/첨부 업다운/웹훅 실행/감사로그/파티션 nonce dedup). DB 라이브(**V1~V17 적용**, presence·유저라우팅·PoW·rate limit·blob FS는 무DB).

## 3. 빌드·테스트·DB (⚠ crate별 독립 — R7)

```bash
# 빌드/테스트는 crate 디렉터리에서 개별 실행 (전체 한방 빌드 구조 아님!)
cd backend/crates/<name> && cargo test

# DB: role=david, db=discord_v1, 유닉스소켓 /var/run/postgresql 포트 48853 (peer auth)
# backend/.env 의 DATABASE_URL (소켓 host는 percent-encoding 필수!):
#   postgres://david:2147483647@%2Fvar%2Frun%2Fpostgresql:48853/discord_v1
cd backend/crates/storage && DATABASE_URL='postgres://david:2147483647@%2Fvar%2Frun%2Fpostgresql:48853/discord_v1' cargo test
```
- 마이그레이션 V1(users/realms/guilds/channels/messages) · **V2 `refresh_tokens`** · **V3 `members`** · **V4 `invites`** · **V5 `roles`+`member_roles`** · **V6 `channel_overwrites`**(+`overwrite_kind` enum) · **V7 `reactions`**(유니코드 emoji PK) · **V8 `message_mentions`**(유저 멘션 PK) · **V9 `dm_pairs`**(1:1 DM 중복방지, user_lo<user_hi) · **V10 `relationships`**(친구·차단 방향성 행 + `relation_kind` enum) · **V11 `read_states`**(채널별 last_read + mention_count) · **V12** messages `content_tsv`+GIN(FTS Q10) · **V13 `thread_meta`**(스레드 D44) · **V14 `attachments`**(첨부 D37) · **V15 `webhooks`** · **V16 `audit_log_entries`** · **V17** messages `PARTITION BY RANGE(id)`(D28, 월별+DEFAULT, nonce 유니크 제거→앱레벨 dedup) 적용됨. psql: `psql -p 48853 -d discord_v1`.
- 서버 실행(단일노드): `cd backend/bins/server && DATABASE_URL=... REST_ADDR=127.0.0.1:8080 cargo run`.
- **멀티노드(mTLS 메시)**: `server gen-certs /tmp/mesh 1 2` + `server gen-keys` → 노드별 `CLUSTER_CONFIG`(TOML: node id/worker_id/listen_addr + peers) + `TLS_CA/TLS_CERT/TLS_KEY` + 공유 `PASETO_SECRET/PASETO_PUBLIC` + **공유 `POW_SECRET`**(D18, gen-keys가 발급) env로 각각 기동. (작은 id가 큰 id에게 dial.)
- 종단 데모(서버 띄운 뒤): `cd backend/bins/cli && cargo run -- --url http://127.0.0.1:8080 scenario` → 가입~메시지수신 자동 검증.
- 수동: `cli register` → `cli create-guild --token T --name G` → `cli listen --token T`(다른 터미널) → `cli send --token T --channel C --content hi`. (register는 PoW D18 자동 풀이.)
- MFA(Phase 4, D19) CLI: `mfa-enable --token T`(secret hex+otpauth URI) → `totp-code --secret HEX`(현재 코드, 인증앱 대역) → `mfa-verify --token T --secret HEX --code C`(활성화) → 이후 `login`은 `mfa_required` → `mfa-login --username U --password P --code C`(2단계 토큰).
- 멀티유저/권한(Phase 3) CLI: `create-invite --token T --guild G` → `join --token T2 --code C`(둘째 유저 합류) · `create-role --token T --guild G --name r --permissions <u64>` · `assign-role --token T --guild G --user U --role R` · `set-channel-perm --token T --channel C --target <role|user id> --kind role|member --allow <u64> --deny <u64>` · `create-channel --token T --guild G --name n`. (권한 강제: 전송=SEND_MESSAGES, 채널생성=MANAGE_CHANNELS, 히스토리=VIEW_CHANNEL+READ_MESSAGE_HISTORY, 역할/오버라이드=MANAGE_ROLES.)
- 멤버 관리(Phase 3, D39) CLI: `members --token T --guild G`(목록) · `set-nick --token T --guild G --user U [--nick N]`(생략 시 제거) · `kick --token T --guild G --user U`(KICK_MEMBERS) · `leave --token T --guild G`(본인 탈퇴=@me). 변동 시 `GUILD_MEMBER_*` 이벤트가 그 Realm 접속 구독자에 팬아웃(`listen`으로 확인). (권한: nick=본인 CHANGE_NICKNAME/타인 MANAGE_NICKNAMES, 소유자 추방·탈퇴 불가.)
- 메시지 편집·삭제·리액션(Phase 3, D39) CLI: `edit --token T --channel C --message M --content "..."` · `delete-message --token T --channel C --message M`(작성자/MANAGE_MESSAGES) · `react`/`unreact --token T --channel C --message M --emoji 👍`(본인). 변동 시 `MESSAGE_UPDATE/_DELETE/_REACTION_ADD/_REMOVE`가 채널 구독자에 팬아웃(`listen`으로 확인).
- 답장·멘션(Phase 3, D39): `send ... --reply <message_id>`(답장, 같은 채널 검증) · content에 `<@<user_id>>` 넣으면 멘션 파싱·적재. `MESSAGE_CREATE` payload에 `reference_message_id`·`mentions:[id]` 포함.
- DM/그룹DM(Phase 3, D8) CLI: `open-dm --token T --user U`(1:1, find-or-create) · `create-group-dm --token T --users U1,U2 [--name N]`(소유자=호출자) · `add-recipient --token T --channel C --user U`(소유자) · `remove-recipient --token T --channel C --user U`(소유자=타인제거 / 본인 id=탈퇴). DM 채널도 `listen`/`send`/히스토리 그대로 동작(길드와 동일 경로).
- 친구·차단(Phase 3, D40) CLI: `add-friend --token T --user U`(요청, 상대가 이미 요청했으면 수락) · `block-user --token T --user U` · `remove-relationship --token T --user U`(친구삭제/취소·거절/차단해제) · `relationships --token T`(목록). 변동 시 `RELATIONSHIP_ADD/_REMOVE`가 대상 유저의 접속 세션에 통지(`listen`으로 확인). 차단하면 그 상대와 1:1 DM 열기·전송 거부.
- 읽음 상태(Phase 3, D41) CLI: `ack --token T --channel C --message M`(채널을 그 메시지까지 읽음) · `read-states --token T`(채널별 last_read + 안 읽은 멘션 수). 멘션 받으면 mention_count 증가, ack로 재계산·리셋. ack 시 본인 다른 세션에 `MESSAGE_ACK` 통지(`listen`으로 확인). READY에도 read_states 포함.
- 전역 presence(Phase 3, D42): 별도 CLI 없음 — 친구끼리 `listen` 중에 상대가 접속/종료하면 `PRESENCE_UPDATE`(online/offline) 수신, READY `presences`에 현재 온라인 친구. 크로스노드는 2노드 mesh에서 자동(gossip). idle/dnd 설정(C→S op 3)은 후속.
- Phase 4 CLI: `search --token T --guild G --content "q"`(전문검색) · `create-channel ... --kind forum` · `create-thread --token T --channel C --name n`/`list-threads`/`archive-thread --thread TID --archived true|false` · `upload-attachment --token T --channel C --message M --file PATH`/`attachments`/`download-attachment --attachment A --out PATH` · `create-webhook --token T --channel C --name n`(토큰 1회 반환)/`list-webhooks`/`execute-webhook --webhook W --webhook-token WT --content "..."`(Bearer 없음)/`delete-webhook` · `audit-logs --token T --guild G`. 서버 첨부 저장: env `ATTACHMENT_DIR`(기본 ./attachments).

## 4. 다음 작업 — 여기서 이어서

**Phase 4 완료.** Phase 3(D17·D39·D8·D40~D43) + Phase 4(인증/봇방지 + 검색 Q10 + 스레드/포럼 D44 + 첨부 D37 + 웹훅 + 감사로그 + 파티셔닝 D28) 전부 완료. 다음 후보:

1. **frontend 착수 (권장)** — R4상 backend·API·CLI가 사실상 완료(Phase 0~4) → web UI(React+TS+Vite, D30) 시작이 자연스러움.
2. **Phase 5 스트레치** — WebAuthn/Passkeys(D19) · CRDT 오프라인 동기화 · **gossip discovery(SWIM 동적 노드 합류·presence anti-entropy, Q11 잔여)** · 이벤트 소싱(D23) · Voice 시그널링(D21) · MinIO 첨부(D37).
3. **하드닝/seam 회수** — 크로스노드 RESUME(버퍼 노드 로컬), D35 Realm 캐시 warmup, idle/dnd op(3), 신규 월 파티션 사전 생성 스케줄, 웹훅/스레드 잔여 seam(SEND_MESSAGES_IN_THREADS 분리·forum_tags 등).

> Phase 4 구현 seam 요약: nonce 멱등 = 앱레벨 dedup(파티셔닝 제약, D34) · 첨부는 사후 첨부(전송 시 동시 첨부 아님, D37) · 웹훅 실행은 Realm 액터 우회(persist-then-emit) · 감사로그는 대표 mutation만 기록(best-effort) · 검색은 길드 단위(author/랭킹 후속).

> D42 presence: 전역 presence가 gossip(풀메시 broadcast + 로컬 친구 필터) 경로를 확립 + `user→호스팅 노드` 디렉터리를 만듦. **D43**이 이 디렉터리를 재사용해 `RELATIONSHIP_*`/`MESSAGE_ACK`의 **크로스노드 유저 라우팅을 완료**(presence처럼 broadcast가 아니라 호스팅 노드에 `USER_DELIVER` 타깃 전송). presence 자체 seam(잔존): 신규 노드 join 시 과거 presence 동기화(anti-entropy) 없음 · idle/dnd(op 3) · 전 노드 재시작 시 휘발 리셋 · 디렉터리는 live 세션만 추적(원격 detach-grace 세션 in-flight 미수신, RESUME/READY로 복구).
> D39 후속 seam: GUILD_MEMBER_ADD는 현재 **기존 접속 멤버**에게만 통지(신규 합류자 본인은 redeem 응답/다음 READY로 상태 확보) — 합류 직후 자동구독 트리거는 후속. emit은 fire-and-forget(라우팅 실패 조용히 드롭). 리액션 집계 카운트(GET reactions/유저목록)·커스텀 이모지(emoji_id)는 Phase 4.

> Phase 2 후속 seam(서두를 필요 없음): D35 Realm 최근메시지 캐시 warmup(rehydrate 시 Postgres 적재), 크로스노드 RESUME(현재 버퍼가 노드 로컬), 액터까지 도는 완전 결정론 DST 실행기.
> 참고: PASETO 키는 단일노드는 기동마다 새로 생성됨(재시작 시 기존 토큰 무효) — 운영/멀티노드는 영속 키 필요(D14, env 로드 구현됨).
> 남은 Phase 1: PoW(D18, Phase 4 가능).

## 5. 작업 중 지킬 것 (요약)

- 최신 lib `cargo add`, edition 2024 (R1).
- 코드 바꾸면 **문서·TODO·CHANGELOG 같이 갱신** (R2). TODO **내용 수정은 사용자 승인 후**(R5), 체크는 자유.
- 버전: 수정 +0.0.1 / 기능 +0.1.0 → `CHANGELOG.md` 기록 (R3).
- 개념=디렉터리(R6), crate 독립 빌드(R7), 크립토 직접 안 짬(P6), 경계는 trait(P2), 상태는 액터(P5).
- 각 crate 구현 후 **테스트 작성 + 통과 확인**.
