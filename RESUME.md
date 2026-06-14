# RESUME — 다음 세션 이어서 작업

> 이 파일 하나로: **문서 읽기 → 코드 검토 → 작업 재개**. (AI/사람 공용 온보딩)

---

## 1. 먼저 읽기 (순서)

1. **`CLAUDE.md`** — 개발 규칙 R1~R7 + 핵심 원칙 P1~P6. **필수.**
2. `docs/README.md` — 문서 인덱스
3. `docs/architecture/decisions.md` — 결정 원장 D1~D38(+ 정제 갱신, Q1~Q11) (왜 이렇게 만들었나 = source of truth)
4. `TODO.md` — 진행 상태 (`[x]` 완료 / `[~]` 진행중 / `[ ]` 미착수)
5. `CHANGELOG.md` — 최근 한 일 (최상단이 최신)

필요 시 깊게: `docs/design-discussion.md`(논쟁 서사), `docs/database/*`, `docs/api/*`, `docs/protocol/node-wire.md`, `docs/architecture/permissions.md`.

## 2. 현재 상태 (2026-06-14, v1.19.0)

- 설계 문서 + Phase 0/1 + **Phase 2(분산 활성화) 완료**. **Phase 3(Discord 본체) 진행 중** — 초대(invites) 완료.
- Phase 3 시작(1.19): **초대(invites)** — 코드 발급/redeem(트랜잭션) + 멤버 추가 → 자동구독 → 크로스유저 팬아웃 **2유저 라이브 검증**. 다음은 권한/멤버/DM 등.
- Phase 2 마감(1.15~1.18): **Gateway RESUME**(per-session seq+재생버퍼+CSPRNG resume_token, D24/D20) · **PING/PONG 생사판정+소유권 failover**(Membership+owner_excluding, D23) · **Backpressure**(느린 클라 끊기, D27) · **DST 하네스**(SimTransport+SimClock+시드 카오스, D25).
- **멀티노드 라이브 검증**: 노드1↔노드2 mTLS 연결 + 공유 PASETO 키 → 노드1 WS 구독 + 노드2 REST 전송 → 노드1이 MESSAGE_CREATE 수신(크로스노드 팬아웃). 단일노드 모드도 유지.
- 구조: `backend/`(rust, **독립 crate** — umbrella 워크스페이스 없음) + `frontend/`(web, 미착수) + `docs/`.
- crate: `domain` `protocol` `actor-rt` `transport` `storage` `cluster-config` `node` `auth` `rest-api` `gateway` + `bins/{server,cli}`.
- **분산 코어**: consistent hashing(ring) + Realm 액터 + 2단 라우팅 + 크로스노드 팬아웃. 전송 = **raw-TCP+mTLS**(`TcpTransport`, rustls) 또는 단일노드 무전송.
- **인증 종단**: `/auth/register|login|refresh` (PASETO + refresh 회전/재사용탐지 D14).
- **실시간 메시징 종단**: `PgStore`(통합 저장소, `Store` 슈퍼트레잇) → REST(`/guilds`, `/channels/:id/messages`, 히스토리 D38) → **WS Gateway**(IDENTIFY/READY/HEARTBEAT/DISPATCH, 자동구독 D13) → dispatch 드라이버(persist-then-fanout D24, nonce 멱등 D34) → 세션 push. CLI `scenario`로 종단 자동검증(D1).
- Snowflake generator는 **노드당 1개**(D11, lock-free CAS)를 server가 소유해 Router·REST·Gateway에 주입.
- 테스트 **64개** 통과 (DB 통합 + 실 mTLS 2노드 + DST 하네스 포함) + CLI scenario·멀티노드·초대 2유저 라이브 검증. DB 라이브(V1~V4 적용).

## 3. 빌드·테스트·DB (⚠ crate별 독립 — R7)

```bash
# 빌드/테스트는 crate 디렉터리에서 개별 실행 (전체 한방 빌드 구조 아님!)
cd backend/crates/<name> && cargo test

# DB: role=david, db=discord_v1, 유닉스소켓 /var/run/postgresql 포트 48853 (peer auth)
# backend/.env 의 DATABASE_URL (소켓 host는 percent-encoding 필수!):
#   postgres://david:2147483647@%2Fvar%2Frun%2Fpostgresql:48853/discord_v1
cd backend/crates/storage && DATABASE_URL='postgres://david:2147483647@%2Fvar%2Frun%2Fpostgresql:48853/discord_v1' cargo test
```
- 마이그레이션 V1(users/realms/guilds/channels/messages) + **V2 `refresh_tokens`** + **V3 `members`** 적용됨. psql: `psql -p 48853 -d discord_v1`.
- 서버 실행(단일노드): `cd backend/bins/server && DATABASE_URL=... REST_ADDR=127.0.0.1:8080 cargo run`.
- **멀티노드(mTLS 메시)**: `server gen-certs /tmp/mesh 1 2` + `server gen-keys` → 노드별 `CLUSTER_CONFIG`(TOML: node id/worker_id/listen_addr + peers) + `TLS_CA/TLS_CERT/TLS_KEY` + 공유 `PASETO_SECRET/PASETO_PUBLIC` env로 각각 기동. (작은 id가 큰 id에게 dial.)
- 종단 데모(서버 띄운 뒤): `cd backend/bins/cli && cargo run -- --url http://127.0.0.1:8080 scenario` → 가입~메시지수신 자동 검증.
- 수동: `cli register` → `cli create-guild --token T --name G` → `cli listen --token T`(다른 터미널) → `cli send --token T --channel C --content hi`.

## 4. 다음 작업 — 여기서 이어서

**Phase 3 — Discord 본체** 진행 중. **초대(invites) 완료**(멀티유저 합류·크로스유저 팬아웃 라이브 검증). 다음 후보:

1. **역할/권한 비트마스크 + 계산순서**(D17) + 채널 권한 오버라이드 — `docs/architecture/permissions.md` 청사진 존재.
2. **멤버 관리**(nick/joined/roles), **GUILD_MEMBER_ADD 이벤트**(현재 join은 멤버 추가만, 기존 멤버 실시간 통지 X).
3. **DM/그룹DM**(Realm + dm_pairs, DB-D2).
4. **리액션 / 편집·삭제(소프트) / 멘션·답장**, **친구·차단**, **읽음 상태**.

> Phase 2 후속 seam(서두를 필요 없음): D35 Realm 최근메시지 캐시 warmup(rehydrate 시 Postgres 적재), 크로스노드 RESUME(현재 버퍼가 노드 로컬), 액터까지 도는 완전 결정론 DST 실행기.
> 참고: PASETO 키는 단일노드는 기동마다 새로 생성됨(재시작 시 기존 토큰 무효) — 운영/멀티노드는 영속 키 필요(D14, env 로드 구현됨).
> 남은 Phase 1: PoW(D18, Phase 4 가능).

## 5. 작업 중 지킬 것 (요약)

- 최신 lib `cargo add`, edition 2024 (R1).
- 코드 바꾸면 **문서·TODO·CHANGELOG 같이 갱신** (R2). TODO **내용 수정은 사용자 승인 후**(R5), 체크는 자유.
- 버전: 수정 +0.0.1 / 기능 +0.1.0 → `CHANGELOG.md` 기록 (R3).
- 개념=디렉터리(R6), crate 독립 빌드(R7), 크립토 직접 안 짬(P6), 경계는 trait(P2), 상태는 액터(P5).
- 각 crate 구현 후 **테스트 작성 + 통과 확인**.
