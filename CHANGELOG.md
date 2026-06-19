# Changelog

버전 규칙(CLAUDE.md R3): 1.0.0 시작 · 수정 +0.0.1 · 새 기능 +0.1.0.
형식: `## [버전] - 날짜` + 변경 내역.

---

## [1.51.1] - 2026-06-19
### 수정 — 첨부 업로드 2MB 한계 버그 (웹 UI 검증 중 발견)
- **증상**: 2MB 초과 이미지 첨부가 `400 "read field: Error parsing multipart/form-data request"`로 실패. 핸들러는 `MAX_ATTACHMENT_BYTES = 8 MiB`를 의도했으나, axum 기본 `DefaultBodyLimit`(2MB)이 그 검사에 닿기 전에 본문을 잘라 8 MiB 상한이 **죽은 코드**였다.
- **수정**: `routes/attachment.rs`의 업로드 라우트에 `DefaultBodyLimit::max(MAX_ATTACHMENT_BYTES + 64 KiB)` 레이어 추가 → 핸들러 의도대로 8 MiB까지 실제 허용(멀티파트 경계 여유 포함). 라우트 한정 적용이라 다른 엔드포인트 본문 한계는 불변.
- **검증**: 3MB 업로드 `400 → 201`, 1MB·9MB+ 경계 확인. crate: `rest-api` 1.22.0→1.22.1.
- 발견 경위: frontend 첨부 e2e 중 표면화. frontend도 동행 수정(0.1.1, WS 프레임 유실 대비 REST 백업 flush — 별개 이슈).

---

## [1.51.0] - 2026-06-19
### 새 기능 — 웹 UI 디스커버리 읽기 엔드포인트 (D30 frontend 착수)
- **`GET /guilds/{realm_id}/channels`** (멤버) — 길드 채널 목록. 웹 클라이언트의 채널 트리 로딩용. 기존 `ChannelRepository::list_by_realm` 재사용(저장소 무변경).
- **`GET /users/@me/realms`** (인증) — 내가 멤버인 Realm 목록을 **이름·종류(guild/dm/group_dm)·소유자**와 함께 반환. READY의 realm-id 목록(이름 없음)을 보완. 기존 `member_realm_ids` + `get_realm` 조합(저장소 무변경).
- **배경**: CLI는 id를 인자로 직접 넘겨 목록 엔드포인트가 불필요했으나, 브라우저 클라이언트는 로그인 직후 "내 서버 → 채널" 트리를 그려야 함. 어떤 웹 클라이언트에도 필요한 읽기 조각이라 사용자 승인 후 추가(규칙2). read-only·마이그레이션 없음·도메인/저장소 무변경 — **rest-api 라우트만 추가**.
- crate 변경: `rest-api` — `routes/guild.rs`에 `list_channels` GET 추가 + 신규 `routes/realm.rs`(`/users/@me/realms`) + 통합 테스트 4(30→34). (1.21→1.22)
- 문서: `decisions.md` D30(읽기 엔드포인트 추가 노트), `api/rest.md` 구현 현황 표 2행.

---

## [1.50.0] - 2026-06-18
### 새 기능 — 크로스노드 RESUME = 세션 마이그레이션 (D24)
- **재연결이 다른 노드에 닿아도 RESUME** — 기존엔 재생 버퍼가 노드 로컬(Hub)이라 원조가 아닌 노드에 재연결하면 INVALID_SESSION(전체 재동기)였다. 이제 그 노드가 원조에서 세션을 **핸드오프**받아 재개.
- **흐름**: 재연결 노드 B가 `ResumeFetch`를 풀메시 broadcast(원조만 세션 보유) → 원조 A가 `export_migration`(토큰·gap 검증 → 미수신 프레임 직렬화 + **자기 세션 제거**) → `ResumeState` 회신 → B가 `import_migration`(세션 재생성 seq=원조값, 빈 버퍼) + 프레임 재생 + RESUMED + **유저 realm 재구독**(구독자표 D12를 B로 이전 → 이후 이벤트가 B로 팬아웃). 조정 = Hub `pending` 맵(oneshot) + 2s 타임아웃. 원조 사망 시 무응답→타임아웃→INVALID_SESSION(무손실).
- crate 변경:
  - `protocol`: `ResumeFetch`(0x0203)/`ResumeState`(0x0204) 와이어 + 라운드트립 테스트(14→15). (1.6→1.7)
  - `gateway`: Hub `export_migration`/`import_migration`/`begin`·`complete`·`cancel_migration` + pending 맵 + run_resume 마이그레이션 분기 + 라운드트립 테스트(9→10). (1.15→1.16)
  - `node`: router handle_inbound에 Resume* → Ok(None)(server가 처리). (1.10→1.10.1)
  - `server`: run_inbound가 ResumeFetch(export→회신)·ResumeState(complete) 처리. (1.9→1.10)
- 문서: `decisions.md` D24(크로스노드 RESUME), `node-wire.md` 0x0203/0x0204, TODO.
- seam: 멀티-세션 유저가 여러 노드에 흩어지면 재구독이 user→node를 단일 노드로 덮음(D12 가정) · 마이그레이션 창(원조 제거~B 재구독) in-flight 이벤트 드물게 유실(다음 이벤트/READY 복구).

---

## [1.49.0] - 2026-06-18
### 새 기능 — WebAuthn usernameless(discoverable) 로그인 (D19)
- **username 없는 passkey 로그인** — 인증기가 resident key로 유저를 고르고, 서버는 자격증명의 user handle(Uuid)에서 유저를 식별(`uuid_to_snowflake`). 기존 로그인은 username으로 passkey를 로드했지만, discoverable은 그 단계가 없다.
- **흐름**: `POST /auth/webauthn/login/discoverable/start`(본문 없음 → challenge + `Ceremony::Discoverable`) → 클라가 resident 자격증명으로 서명 → `/finish`(자격증명 → user 식별 → 그 유저 passkey로 검증 → access+refresh).
- **P6 유지**: 크립토는 webauthn-rs(`conditional-ui` feature의 discoverable API) — `start/identify/finish_discoverable_authentication` 위임. 직접 짠 것 없음.
- crate 변경:
  - `auth`: `WebauthnService`에 discoverable 3메서드 + `uuid_to_snowflake` + `conditional-ui` feature. 헤드리스 검증 테스트(challenge 발급 + handle↔Snowflake 왕복, 19→20). (1.3→1.4)
  - `rest-api`: `Ceremony::Discoverable` 변형 + `/auth/webauthn/login/discoverable/{start,finish}` 라우트. (1.20→1.21)
- 문서: `decisions.md` D19(usernameless), `api/rest.md` discoverable 엔드포인트.
- seam: 전체 서명 라운드트립은 resident-key 탐색 필요 → SoftPasskey(헤드리스) 미지원, 실제 인증기 영역 · 멀티노드 ceremony 공유(start/finish 같은 노드)는 후속.

---

## [1.48.0] - 2026-06-18
### 새 기능 — Voice 시그널링 제어 평면 (D47, 미디어 제외 D21)
- **음성 시그널링 제어 평면 구현** (이전 "설계 문서만"에서 전환) — gateway op 4(`VOICE_STATE_UPDATE` C→S): 입장/이동/퇴장 + self mute/deaf → 권한 CONNECT(채널 컨텍스트) → 같은 Realm 구독자에 `VOICE_STATE_UPDATE` 팬아웃. 입장 시 `VOICE_SERVER_UPDATE`(endpoint=**null** stub)를 그 세션에만 회신 — 미디어 서버 경계(D21) 표식.
- **"거의 코드 0" (스펙 §3)**: 팬아웃은 멤버 이벤트와 같은 **기존 emit 경로(`Router::route_emit`)를 그대로 재사용** — 로컬/원격(`REALM_EMIT`/`REALM_FANOUT`) 자동 처리, **신규 와이어·신규 액터 상태 0**. 음성 상태가 텍스트 메시징과 동형(Realm 라우팅+구독자표+범용 envelope)임을 실증.
- **미디어 평면은 D21대로 영구 제외**: WebRTC/SFU/Opus/SRTP/UDP 일체. `VOICE_SERVER_UPDATE.endpoint=null`이 그 경계 표식.
- crate 변경:
  - `gateway`: protocol op 4 상수 + `handle_voice_state`(session, 권한+팬아웃+VOICE_SERVER_UPDATE) + `effective_channel_perms` 헬퍼(can_send 일반화, CONNECT 공유) + pump에 session_id. (1.14→1.15)
- 문서: `decisions.md` D47(구현), `voice-signaling.md`(상태+§8 체크리스트), `api/gateway.md`(op 4 + VOICE dispatch).
- seam(스펙 §1/§5/§6): 액터 voice_states 맵·READY voice_states 스냅샷·서버 모더레이션(MUTE/DEAFEN/MOVE_MEMBERS, server_mute/deaf 항상 false) — 현재 fanout-only(클라가 스트림으로 상태 재구성). 액터 상태화는 후속 증분.

---

## [1.47.0] - 2026-06-18
### 새 기능 — D35 채널별 last_message_id warmup (D48 연결)
- **콜드 Realm 액터를 이벤트 로그 프로젝션으로 warmup** — failover/Q7 respawn으로 빈 액터가 떠도 채널별 last_message_id를 이벤트 로그(`replay_events`→`RealmProjection`)로 복원. "상태=이벤트의 함수"(D48) rehydrate의 실사용 쇼케이스.
- **경로(P2 준수)**: node는 IO 무지라 storage를 모름 → 엣지(gateway)가 프로젝션을 산출해 `Router::warm_realm_last_ids`→액터 `WarmAndGet`(max-merge·멱등)로 주입. 액터는 send 시 라이브 갱신 + warmup으로 콜드 복원.
- **소비자**: gateway READY 스냅샷 `last_message_ids`(채널별, Discord식 "최신으로 점프") — 로컬 소유 realm은 액터 권위값, 원격 소유는 프로젝션 값 직접. (소비자 없는 dead code를 피하려 content 캐시 전체가 아닌 last_message_id부터.)
- **프로젝션 확장**: `RealmProjection`에 `last_message_by_channel`(채널별 max) 추가 — 순수 fold, 결정론 단위 테스트.
- crate 변경:
  - `domain`: `RealmProjection.last_message_by_channel` + fold + 테스트. (1.19→1.20)
  - `node`: `RealmActor`가 `last_by_channel` 보유(send 갱신) + `WarmAndGet` 명령 + `Router::warm_realm_last_ids` + 테스트(1→). (1.9→1.10)
  - `gateway`: READY가 `channel_last_ids`로 콜드 액터 warmup + `last_message_ids` 필드. (1.13→1.14)
- 문서: `decisions.md` D35(warmup 구현)+D48 연결, `api/gateway.md` READY `last_message_ids`, TODO.
- seam: 메시지 **content** 캐시(액터 히스토리 서빙)는 읽기 경로 재라우팅 필요한 후속 · warmup이 realm마다 전체 재생(O(events)) — D48 스냅샷/컴팩션과 함께 최적화 · 원격 realm 액터 warmup 조회 없음.

---

## [1.46.0] - 2026-06-18
### 새 기능 — 이벤트 소싱 멤버/삭제 생산자 (E2, D48 후속)
- **멤버 입퇴장·메시지 삭제 사실을 이벤트 로그에 기록** — 기존엔 `MessageCreated`만 append됐고 `MemberJoined/Left`·`MessageDeleted`는 enum에만 있고 생산자가 없었다(D48 seam). 이제 `realm_events` 로그가 멤버십·삭제까지 포착 → `RealmProjection`의 members 집합·message_count가 정확.
- **단일 소비자 위임 (E2)**: seq 무경합은 *소유 노드 dispatch만 append*(D24)에서 나오므로 rest-api 핸들러가 직접 append하지 않는다. `RealmEmitter::emit`에 **타입 사실(`Option<RealmEventKind>`)을 동반**시켜 emit 경로로 흘리고, 소유 노드 dispatch가 `Broadcast` 처리 시 append.
- **크로스노드 (B 정석)**: 원격 소유 realm은 `REALM_EMIT` 와이어에 사실을 **primitive 슬롯(`EventFact`)** 으로 실어 위임 → 소유 노드가 복원해 자기 dispatch에서 append(protocol은 domain 무의존 P2 — `node`가 변환 소유). payload-파싱 대안은 D48 정신 위배라 기각.
- **무결성 (E1, 수용)**: append는 persist와 별개(eventual) — 진실은 members/messages 테이블, 로그는 보조라 append 실패는 warn 후 계속. 완전 원자성은 의도적 후속.
- 배선: invite redeem→MemberJoined · kick/leave→MemberLeft · 그룹DM recipient±→MemberJoined/Left · message delete→MessageDeleted. 닉변경·리액션·스레드 등은 None. 웹훅 MESSAGE_CREATE는 액터 우회라 미기록(seam).
- crate 변경:
  - `domain`: `RealmEmitter::emit`에 `fact: Option<RealmEventKind>` 추가. (1.18→1.19)
  - `protocol`: `EventFact` 구조체 + `RealmEmit`에 `fact` 필드 + 코덱 + 라운드트립 테스트(13→14). (1.5→1.6)
  - `node`: RealmCommand/RealmEvent `Broadcast`에 fact 관통 + `route_emit`/`emit` 시그니처 + `EventFact`↔`RealmEventKind` 변환 + 크로스노드 fact 보존 테스트. (1.8.1→1.9)
  - `gateway`: dispatch가 `Broadcast` fact를 append_event(단일 소비자). (1.12→1.13)
  - `rest-api`: 12개 emit 호출부에 fact 인자(3 사실+그룹DM 2 + 나머지 None), RecordingEmitter 테스트 더블 갱신. (1.19→1.20)
- 문서: `decisions.md` D48 E1/E2 갱신, `node-wire.md` REALM_EMIT fact, TODO.
- seam: E1 비트랜잭션(수용) · 웹훅 메시지 미기록 · 스냅샷/컴팩션 없음(전체 재생).

---

## [1.45.1] - 2026-06-18
### 하드닝 — 액터 supervisor (Q7 해소, D50)
- **Let-it-crash + Router lazy 재시작**: Realm 액터가 패닉으로 죽으면 `Router::local_realm`이 닫힌 메일박스를 감지해 **fresh 액터로 재spawn**(= D23 rehydrate). 기존엔 죽은 메일박스가 캐시에 남아 그 Realm이 프로세스 재시작 전까지 영구 사망하던 버그를 닫음. 액터 상태는 재구축 가능한 휘발 캐시(구독자표, DB-D5)뿐 + 메시지 진실은 Postgres(D24)라 fresh-spawn으로 충분(클라 자동 재구독 D13).
- **actor-rt 계약 명문화**: `spawn` doc에 let-it-crash 계약(패닉=task 종료=메일박스 닫힘, 자동 재시작 안 함=상위 supervisor 책임). 런타임은 범용·미니멀 유지(새 의존성 0). 계약 단위 테스트 추가(`panicking_handler_closes_mailbox`).
- crate 변경:
  - `actor-rt`: `spawn` 계약 doc + 패닉→메일박스 닫힘 테스트(2→3 테스트). (1.0.0→1.0.1)
  - `node`: `Router::local_realm`이 죽은 메일박스 감지→재spawn(supervisor, 3줄). (1.8.0→1.8.1)
- 문서: `decisions.md` D50 신설 + Q7 해결 체크, TODO Q7 체크.
- seam: 재시작 폭주 제한(restart intensity)·backoff 없음(study 범위 충분) · 크로스노드 RESUME과 함께 장애복구 그림 완성(후속).

---

## [1.45.0] - 2026-06-16
### 새 기능 — CRDT 오프라인 동기화 (Phase 5, D49)
- **상태 기반 CRDT(CvRDT)로 충돌 없는 멀티기기 동기화** — 여러 기기가 오프라인 편집 후 재연결 시 수렴. 핵심 = 순수·테스트 가능한 merge 엔진(결합·교환·멱등 semilattice join).
- **순수 코어(P2/DST)**: `domain::crdt` 툴킷 — `LwwRegister`·`LwwMap`(키별 LWW + 툼스톤)·`OrSet`(observed-remove, add-wins)·`PnCounter`. merge 법칙 + 오프라인 2기기 수렴을 단위 테스트 8개로 고정. domain serde 무의존.
- **적용 = 유저 동기화 문서(LWW-Map)**: 드래프트/설정 같은 per-user 키-값. 병합 권위는 domain `LwwMap`, 영속은 storage **LWW 가드 upsert**((ts,node) 큰 것만 채택 → DB가 LWW 보존). 포트 `CrdtRepository`(load/merge) → Store 합류. REST `GET/POST /users/@me/sync`(상태 기반 — 어느 순서로 push해도 수렴).
- **라이브 검증**(curl): phone@200 push → laptop@210 채택(LWW) → 이른 phone@200 재push 무시 → GET 수렴=laptop → 툼스톤 삭제. + rest-api HTTP 통합 테스트(2기기 수렴·인증 401).
- crate 변경:
  - `domain`: `crdt` 모듈(순수 툴킷, 8 단위 테스트) + `CrdtEntry`/`LwwMap` 변환 + `CrdtRepository` 포트 + Store 합류. (1.17→1.18)
  - `storage`: **마이그레이션 V21** `user_crdt_entries` + `crdt` 어댑터(LWW 가드 upsert) + 수렴 DB 테스트. (1.19→1.20)
  - `rest-api`: `routes/sync`(GET/POST) + 라우트 등록 + MemStore CRDT 인메모리 구현 + HTTP 통합 테스트 2. (1.18→1.19)
  - `server`: PgStore가 자동으로 CrdtRepository 구현(Store) — 배선 변경 없음. (1.8→1.9, 파티션 startup과 동반 bump)
- 문서: `decisions.md` D49 + §6 독창성 + Phase 5 로드맵, `02-schema.md` user_crdt_entries, `api/rest.md` sync, TODO 체크.
- seam: 현재 LWW-Map 1종만 wire(OrSet/PnCounter 적용은 후속) · REST 폴링(실시간 WS push 아님) · 메시지 본문은 CRDT 아님(append-only D24).

---

## [1.44.0] - 2026-06-16
### 새 기능 — 이벤트 소싱 (Phase 5, D48 / D23, 가산형 CQRS)
- **append-only Realm 이벤트 로그 + 순수 프로젝션** — D23의 이벤트 소싱 스트레치를 **비파괴(가산형)**로 구현. `messages`(엔티티 진실)를 대체하지 않고 그 위에 타입화된 도메인 사실 로그(`realm_events` V20)를 얹음(CQRS: events=fact, projection=read model). 진실 중복 꼬임 회피 = messages 파괴적 마이그레이션 안 함.
- **순수 코어(P2/DST)**: `domain::event` — `RealmEventKind`(MessageCreated/Deleted·MemberJoined/Left, 안정 code) + `RealmProjection`(이벤트 시퀀스를 **결정론적 fold** → members·message_count·last_message_id·last_seq). "상태=이벤트의 함수" 불변식을 단위 테스트로 고정(재생 결정론·증분=전체·언더플로 없음). domain serde 무의존(직렬화는 storage가 타입화 bigint 슬롯으로 소유).
- **port + 배선**: `EventLogRepository`(append→per-realm 단조 seq, replay→증분 커서) → Store 합류. dispatch 드라이버가 persist(D24) 직후 `MessageCreated` append(**단일 직렬 소비자라 seq 경합 없음**, nonce D34 동형). 실패는 warn 후 계속(messages 진실, 로그 보조).
- **라이브 검증**: cli scenario → `realm_events`에 MessageCreated 1건(message_id 일치). rehydrate 재생(`RealmProjection`)은 D35 캐시 warmup 입력 자리.
- crate 변경:
  - `domain`: `event` 모듈(순수 enum+projection) + `EventLogRepository` 포트 + Store 합류 + 단위 테스트 4. (1.16→1.17)
  - `storage`: **마이그레이션 V20** `realm_events` + `event` 어댑터(타입화 슬롯, serde 무의존) + append→replay→projection 라운드트립 DB 테스트. (1.18→1.19)
  - `gateway`: dispatch가 MessageCreated를 이벤트 로그에 append. (1.11→1.12)
  - `rest-api`: 테스트 MemStore에 EventLogRepository 인메모리 구현. (no version change — 테스트만)
- 문서: `decisions.md` D48 + D23 갱신, `02-schema.md` realm_events, TODO 체크.
- seam: append-persist 비트랜잭션 · 멤버/삭제 생산자는 후속(rest-api 경로) · 스냅샷/컴팩션 없음 · 핫패스 2차 쓰기(write amplification).

---

## [1.43.1] - 2026-06-16
### 문서 — Voice 시그널링 설계 (Phase 5, D47, 코드 미구현)
- **Voice = 설계 문서만**(사용자 결정) — `docs/protocol/voice-signaling.md` 신설. 음성 **시그널링(제어 평면)**만 설계, **미디어(WebRTC/SFU/Opus/SRTP)는 D21 경계 밖**.
- 핵심: voice state는 Realm 휘발 상태(D12/D42 동형)라 **REALM_FANOUT 범용 envelope(D39) 재사용** → 텍스트 메시징 인프라에 거의 코드 0으로 얹힘. gateway op 4(`VOICE_STATE_UPDATE`), 권한 CONNECT/SPEAK/MUTE·DEAFEN·MOVE_MEMBERS(permissions.md 예약 비트), wire `VOICE_STATE_SET`(0x0120 예약). VOICE_SERVER_UPDATE는 endpoint=null stub(미디어 경계 표식).
- 결정 **D47** 추가, D21에 링크. gateway.md op 4 표기, README 인덱스, TODO 체크.

---

## [1.43.0] - 2026-06-16
### 새 기능 — 신규 월 메시지 파티션 사전 생성 (Phase 5 하드닝, D28 / 04 §6)
- **다가오는 달 파티션 자동 생성** — V17은 2026_06/_07 + DEFAULT만 만들었다. 시간이 흐르면 새 달 메시지가 DEFAULT로 쌓여 "최근=핫" 시간 지역성(04 §2)이 무너진다. 이 seam을 닫음.
- **달력 계산은 Postgres에 위임**(앱에 날짜 라이브러리 무도입) — V19 plpgsql `ensure_message_partitions(months_ahead)`가 `(month_start_ms - EPOCH_MS) << 22` 경계로 월별 파티션을 `to_regclass` 가드로 멱등 생성. 경계가 V17 파티션과 정확히 연속(contiguous)임을 라이브 검증.
- server가 **startup에 `ensure_message_partitions(2)` 호출**(이번 달 + 2개월). 미래 달은 DEFAULT에 행이 없어 안전.
- crate 변경:
  - `storage`: **마이그레이션 V19** + `partition` 모듈(`PgStore::ensure_message_partitions`) + 멱등 DB 통합 테스트(2차 호출 0개). (1.17→1.18)
  - `server`: startup 파티션 보장 호출. (1.7→1.8)
- 문서: `docs/database/04-partitioning-and-distributed.md` §6, `decisions.md` D28 seam 갱신.
- seam: DEFAULT에 이미 미래-id 행이 쌓였다면 그 달 파티션 생성이 실패(로컬 study에선 비현실적) · 자동 스케줄(cron) 아님(startup 1회).

---

## [1.42.0] - 2026-06-16
### 새 기능 — presence idle/dnd (Phase 5 하드닝, D42 seam)
- **클라 상태 변경 op 3 (PRESENCE_UPDATE C→S)** — D42 presence의 "idle/dnd는 op 3 후속" seam을 닫음. presence 레지스트리는 이미 `Idle`/`Dnd`를 지원했고, 빠졌던 클라→서버 상태 변경 경로만 추가.
- 클라가 WS로 `{ "op": 3, "d": { "status": "online"|"idle"|"dnd" } }`를 보내면 그 유저의 presence가 전이 → `PRESENCE_GOSSIP` 풀메시 broadcast + 로컬/크로스노드 친구에게 `PRESENCE_UPDATE` 배달(기존 D42 경로 재사용). 연결 중엔 online 계열만 허용(offline/미상은 무시 — 오프라인 전이는 세션 종료가 담당).
- crate 변경:
  - `gateway`: `protocol::op::PRESENCE_UPDATE=3` + `presence::set_status`(상태 인자화, `set_online`이 위임) + `session::pump`가 op 3 처리(user_id 전달) + `parse_presence_status` 단위 테스트. (1.10→1.11)
- 문서: `docs/api/gateway.md`(op 3 구현 현황), `decisions.md` D42 seam 갱신.

---

## [1.41.0] - 2026-06-16
### 새 기능 — WebAuthn/Passkeys (Phase 5, D19)
- **암호 없는 로그인(FIDO2 공개키 자격증명)** — D19의 "Passkeys 스트레치"를 구현. **P6: 크립토는 검증 크레이트 `webauthn-rs`(0.5.5)** — 어테스테이션·서명·challenge·counter 전부 위임(수제 금지).
- **흐름**: 등록(인증된 유저) `POST /auth/webauthn/register/{start,finish}` → `Passkey` 저장. 로그인 `POST /auth/webauthn/login/{start,finish}` — start는 username으로 자격증명 로드, finish는 서명 검증 후 **access+refresh 발급(암호 없는 로그인)**. counter 진전 시 재저장(클론 탐지 토대).
- **저장**: `webauthn_credentials`(V18) — `credential_id BYTEA UNIQUE`(exclude/조회) + **`passkey JSONB`**(webauthn-rs `Passkey` 직렬화, 공개키+counter 캡슐화). domain은 opaque `passkey_json`으로만 알아 webauthn-rs 무의존(P2).
- **ceremony 상태**: register/auth 중간 상태(`PasskeyRegistration`/`PasskeyAuthentication`)는 **휘발(DB-D5)** — rest-api `AppState`의 인메모리 맵(ceremony_id 키, 5분 만료, 서버측 보관 = 위변조 차단). RP = env `WEBAUTHN_RP_ID`/`WEBAUTHN_RP_ORIGIN`(없으면 webauthn 엔드포인트 404).
- crate 변경:
  - `domain`: `webauthn`(NewWebAuthnCredential/WebAuthnCredential, opaque) + `WebAuthnRepository`(add/list/update) → Store 합류. (1.15→1.16)
  - `storage`: **마이그레이션 V18** `webauthn_credentials` + PgStore impl(passkey jsonb↔text) + DB 테스트 +1. (1.16→1.17)
  - `auth`: **`webauthn`**(`WebauthnService` — webauthn-rs 래퍼, P6) + webauthn-rs 의존 + **SoftPasskey 통합 테스트**(register→auth 서명검증 라운드트립, 헤드리스). (1.2→1.3)
  - `rest-api`: `routes/webauthn`(4 엔드포인트) + `AppState`에 webauthn/ceremony 주입 + MemStore impl + **HTTP 통합 테스트**(SoftPasskey로 register→암호없는 login 전 계층 통과). (1.17→1.18)
  - `server`: env로 `WebauthnService` 구성·주입. (1.6→1.7)
  - `cli`: `webauthn-demo`(가입→passkey 등록→암호없는 로그인, SoftPasskey 헤드리스) + `webauthn-authenticator-rs` 의존. (1.16→1.17)
- 문서(R2/P1): decisions **D19에 WebAuthn 구현 노트**, `02-schema.md` webauthn_credentials(passkey JSONB), `api/rest.md` 엔드포인트. TODO 체크.
- **라이브 검증**: 단일노드 서버(RP=localhost) + `cli webauthn-demo` → 가입→passkey 등록(실 Postgres V18)→**암호 없는 로그인 → PASETO 토큰 발급** 확인.
- seam: usernameless(discoverable) 로그인·어테스테이션 정책(현재 None)·멀티노드 ceremony 공유(start한 노드로 finish)는 후속.
- 전 crate 테스트 합계 **163** (auth 19, rest-api 32[28+4], storage 19, domain 18, node 37, protocol 13, transport 11, cluster-config 4, gateway 8, actor-rt 2).

## [1.40.0] - 2026-06-16
### 새 기능 — SWIM 하드닝 (D45 후속, Phase 5)
- **robust join (H1)** — `run_swim`이 startup에 `SwimJoin`을 1회만 보내 seed 핸드셰이크 전이면 유실되던 seam을 닫음. `Swim.bootstrapped`(seed의 SwimGossip/Ack 수신 시 set) 전까지 **매 tick join 재전송**. → 신규 노드가 **seed보다 먼저 기동해도** seed 등장 시 합류. **라이브 검증**(node3 동적 노드를 seed node1보다 먼저 띄움 → node1 등장 ~2초 후 풀메시 자가구성).
- **round-robin probe (H2)** — 임의 probe 대상 선택을 **셔플 라운드로빈**으로 — 한 라운드에 각 멤버 정확히 1회 탐지(임의 선택의 탐지시간 변동 제거, SWIM 정석).
- **주기적 membership anti-entropy (H3)** — 감염형 전파(bounded λ·logN) 누락 대비 — `anti_entropy_ticks`마다 **full-snapshot SwimGossip**을 임의 멤버에 push해 view 수렴 보강.
- **SwimConfig env 노출 (H4)** — `SWIM_PING_TIMEOUT_MS`/`SWIM_PROBE_PERIOD_MS`/`SWIM_SUSPICION_TIMEOUT_MS`/`SWIM_INDIRECT_K`/`SWIM_GOSSIP_FANOUT`/`SWIM_DISSEMINATION_COUNT`/`SWIM_MAX_PIGGYBACK`/`SWIM_ANTI_ENTROPY_TICKS`/`SWIM_PROBE_INTERVAL_MS` 운영 튜닝.
- crate: `node` swim 모듈(bootstrapped·probe_order·tick_count + `next_probe_target`/`anti_entropy`/`full_snapshot`) + 단위 테스트 +3(bootstrap·round-robin 1라운드 전멤버·anti-entropy push). (1.7→1.8) · `server` `swim_config_from_env` 헬퍼. (1.5→1.6)
- 문서(R2): decisions D45 하드닝 노트 + seam 갱신(멤버 테이블 = 의도적 휘발 DB-D5 명시). 멤버 영속화는 **비대상**(DB-D5 위반).
- 전 crate 테스트 합계 **159** (node 31+2+4 = 37, 나머지 동일).

## [1.39.0] - 2026-06-16
### 새 기능 — Phase 5 착수: SWIM gossip discovery (D45) + presence anti-entropy (D46)
- **동적 클러스터 멤버십 (D45)** — 정적 config(D5) + 단순 PING/PONG(D23)을 **SWIM**으로 일반화. Q11의 마지막 갈래를 닫음.
  - **3상태 + incarnation**: `Alive`/`Suspect`/`Dead` + incarnation 단조 카운터. 충돌 해소 = 높은 incarnation 우선, 같으면 `Dead>Suspect>Alive`. 자기에 대한 Suspect/Dead를 보면 incarnation을 올려 **Alive 반박(refute)**.
  - **direct + indirect 탐침**: 주기 ping → 타임아웃 시 k명에게 `ping-req` 위임(간접 탐침, 오탐 감소) → 직접·간접 모두 실패해야 Suspect → suspicion 타임아웃 후 Dead.
  - **감염형 전파(infection-style)**: 멤버 델타를 ping/ack/gossip에 피기백, 각 업데이트 유한 횟수(≈λ·logN)만 재전파.
  - **동적 합류**: 신규 노드가 config의 **seed(introducer)**에게 `SwimJoin` → 전체 테이블 회신 + Alive 감염 전파 → 전 노드가 학습하고 **멤버 addr로 런타임 dial**(풀메시 자가구성).
  - **동적 해시링**: `Router.ring`을 `RwLock`로 전환 — Alive 학습 시 `add_node`, Dead 확정 시 `remove_node`. Suspect는 신규 소유권 부여에서 제외(membership down-set 재사용). 일관 해싱이 그 노드 몫 Realm만 재배치(D6/D23).
  - **경계(P2/P5)**: `node::swim::Swim`은 now_ms/rng 주입 **순수 상태머신** — `SwimAction`(송신/링변형/dial)으로 반환, 드라이버(`run_swim`)·server inbound가 IO 실행. → DST 결정론 재현.
- **presence anti-entropy (D46)** — D42의 델타-only seam을 닫음. 신규 노드 합류를 SWIM이 감지하면 기존 노드가 **자기 호스팅 유저 presence 스냅샷**을 `PRESENCE_GOSSIP`로 push(신규 wire 불필요). `Presence::snapshot_local` 추가.
- crate 변경:
  - `protocol`: SWIM 5종 메시지(`SwimJoin`/`SwimPing`/`SwimAck`/`SwimPingReq`/`SwimGossip`) + `SwimMember` 델타, msg_type 0x0301~0x0305, 라운드트립 테스트 +4. (1.4→1.5)
  - `node`: 신규 **`swim`** 모듈(상태머신 + `run_swim` 드라이버 + `apply_swim_actions`) + `Router` 동적 ring(`add/remove_ring_node`) + `Presence::snapshot_local`. 단위 테스트 +7, **DST 통합 `swim_dst.rs` +4**(동적 합류 수렴·재현성·파티션→Dead·재합류 멱등). (1.6→1.7)
  - `transport`: `TcpTransport::connect_peer`/`is_connected`(런타임 동적 dial, 중복 방지) +1. (1.1→1.2)
  - `cluster-config`: `NodeConfig.dynamic` 플래그(seed 합류 모드) +1. (1.0→1.1)
  - `server`: `Swim` 배선 + `run_swim`(정적 `run_failure_detector` 대체) + `on_member_up` 훅(동적 dial + presence push) + run_inbound SWIM 처리. (1.4→1.5)
- 문서(R2/P1): decisions **D45/D46 신설**, D5/Q11/§4-R/§6 갱신, `protocol/node-wire.md`(0x03xx SWIM 바디 명세). TODO 체크.
- **라이브 검증(멀티노드 mTLS)**: node1·2 정적 mesh → **node3(dynamic, seed=node1) 합류** → node1 학습 → gossip으로 node2가 node3 발견·dial → **풀메시 자가구성** 확인. 동적 합류 node3에서 **CLI scenario PASS**(전 메시징 파이프라인 + 동적 ring 라우팅). node3 종료 시 피어가 이탈 감지·재연결.
- seam: 클러스터 전체 동시 재시작 시 seed 부재면 부트스트랩 불가(seed 다중화로 완화)·멤버 테이블 휘발(DB-D5)·파티션 양쪽 상호 Dead(합의 부재 D33)·완전 anti-entropy(주기 digest)는 후속.
- 전 crate 테스트 합계 **156** (protocol 13, node 28+2+4, transport 11, cluster-config 4, gateway 8, rest-api 30, auth 18, domain 18, actor-rt 2, storage 18 — DB 라이브 별도).

## [1.38.0] - 2026-06-16
### 새 기능
- **메시지 시간 RANGE 파티셔닝 (Phase 4, D28)** — `messages`를 `PARTITION BY RANGE (id)`로 전환(드롭&재생성, 로컬 데이터 폐기).
  - **왜 RANGE(id)**: Snowflake id 상위 비트=시간 → "최근=핫" 지역성(04 §2). realm 해시는 런타임 라우팅이 이미 담당.
  - **파티션**: 월별(id 경계 = `(month_start_ms - EPOCH) << 22`) — `messages_2026_06`/`_07` + `messages_default` 캐치올. 인덱스(`ix_messages_channel`, FTS GIN)는 부모에 정의 → 파티션 상속.
  - **D34 변경(중요)**: nonce 멱등성을 **DB 부분 유니크 → 앱레벨 dedup**으로 이전. Postgres는 파티션 테이블 유니크 인덱스에 파티션 키(id) 포함을 강제 → `uq(channel,author,nonce)` 불가. `create_message`가 가드 INSERT(`WHERE NOT EXISTS (동일 channel,author,nonce)`)로 dedup — dispatch 드라이버가 **단일 직렬 소비자**라 레이스 없음(D24). 사용자 승인 결정.
  - **첨부 FK 유지**: PG 12+는 파티션 부모 참조 FK + ON DELETE CASCADE 지원 → 04 §2의 앱레벨 완화(a) 불필요, `attachments.message_id → messages(id)` CASCADE 복원.
  - `storage`: **마이그레이션 V17**(FK 제거→DROP→파티션 부모 재생성→월별/DEFAULT 파티션→FK 복원) + `create_message` 앱레벨 dedup. (1.15→1.16)
- 문서(R2): decisions **D28 구현 + D34 갱신**(앱레벨 dedup), `database/04-partitioning-and-distributed.md` §2(V17 구현, FK 유지), `database/02-schema.md`(messages 파티션 노트). TODO 체크.
- **라이브 검증**: 파티션 라우팅(id 3.4e17 → `messages_2026_06`) + 전체 파이프라인 e2e(전송→파티션 적재, **같은 nonce 2회→1건 dedup**) + storage 18 테스트 전부 파티션 테이블에서 통과(nonce 멱등·FTS·첨부 CASCADE·read_state).
- seam: 신규 월 파티션 사전 생성은 운영 작업(04 §6, 현재 DEFAULT가 흡수) · 과거 파티션 아카이브/detach는 후속.
- 전 crate 테스트 합계 **138** (storage 18, 파티션 전환으로 테스트 수 불변).

## [1.37.0] - 2026-06-16
### 새 기능
- **감사 로그 (Phase 4)** — 길드 관리 행위 시간순 기록 + 조회.
  - **모델**: `audit_log_entries`(V16: id Snowflake=시간순, realm/actor/action_type i16/target/changes jsonb). `changes`는 **불투명 JSON 문자열**(생산 엣지 조립, storage는 jsonb↔text 캐스트로 serde 무의존).
  - **기록(best-effort)**: 채널 생성(10)·역할 생성(30)·역할 부여(25)·닉 수정(24)·추방(20)·웹훅 생성(50)/삭제(52) 라우트가 mutation 후 `record()` 헬퍼로 남긴다(실패해도 주 동작 진행).
  - **조회**: `GET /guilds/:id/audit-logs?before=&limit=`(VIEW_AUDIT_LOG, 최신순 Snowflake 커서).
  - `domain`: 신규 **`audit`** 모듈(`AuditAction` 코드/`AuditEntry`/`NewAuditEntry`) + `AuditRepository`(log/list) + Store 합류. (1.14→1.15)
  - `storage`: PgStore 구현 + **마이그레이션 V16**(`audit_log_entries`). 종단 통합 +1. (1.14→1.15)
  - `rest-api`: `routes/audit`(조회 + `record` 헬퍼) + 5개 mutation 라우트에 기록 삽입. MemStore audit + 통합 +1. (1.16→1.17)
  - `cli`: `audit-logs`. (1.15→1.16)
- 문서(R2): decisions(감사 기록 = best-effort 사후 기록), `api/rest.md`(audit-logs 라우트), `database/02-schema.md`(audit V16). TODO 체크.
- **라이브 e2e**: 단일노드 — create-channel/create-role/create-webhook → `audit-logs`(**action 10/30/50, actor·target, 최신순 3건**). rest-api 통합(기록·최신순·changes·비권한 403).
- seam: 길드/멤버/채널 update·delete·밴 등 일부 mutation 미기록(대표 집합) · reason 필드·동일 트랜잭션 보장은 후속.
- 전 crate 테스트 합계 **138** (storage 17→18 · rest-api 29→30 등).

## [1.36.0] - 2026-06-16
### 새 기능
- **웹훅 (Phase 4)** — 토큰으로 채널에 메시지를 게시하는 외부 진입점.
  - **토큰**: opaque 랜덤 + SHA-256 해시 저장(`auth::generate_refresh`/`hash_refresh` 재사용 — 크립토는 검증 크레이트 한 곳에, P6/D14). 원본은 생성 시 **1회만** 반환, DB엔 해시(`webhooks.token_hash`)만.
  - **실행**: `POST /webhooks/:id/:token`(Bearer 없음, URL 토큰 인증). 메시지를 **rest-api가 직접 persist + `MESSAGE_CREATE` emit**(Realm 액터 우회 seam) → Realm 구독자에 팬아웃. author=웹훅 creator, payload에 `webhook_id`. 틀린 토큰 401.
  - **관리**: `POST/GET /channels/:cid/webhooks`(생성·목록, MANAGE_WEBHOOKS) · `DELETE /webhooks/:id`(MANAGE_WEBHOOKS). 목록은 토큰 비노출.
  - `domain`: 신규 **`webhook`** 모듈 + `WebhookRepository`(create/list/get/delete) + Store 합류. (1.13→1.14)
  - `storage`: PgStore 구현 + **마이그레이션 V15**(`webhooks`). 종단 통합 +1. (1.13→1.14)
  - `rest-api`: `routes/webhook` + `events::webhook_message_payload`. MemStore webhook + 통합 +1. (1.15→1.16)
  - `cli`: `create-webhook`/`list-webhooks`/`execute-webhook`/`delete-webhook`. (1.14→1.15)
- 문서(R2): decisions(웹훅 = 토큰 게시 + 액터 우회 seam), `api/rest.md`(웹훅 라우트), `database/02-schema.md`(webhooks V15). TODO 체크.
- **라이브 e2e**: 단일노드 — create(토큰 반환) → execute(Bearer 없음) → 히스토리에 메시지 게시 확인 → 틀린 토큰 401 → delete → 실행 404. rest-api 통합(생성/목록 토큰 비노출/멤버 403/실행/틀린토큰 401/빈내용 400/삭제 후 404).
- seam: 웹훅 메시지는 Realm 액터 우회(persist-then-emit, id 단조성은 노드 generator로 유지) · 웹훅 avatar/username override·실행 rate limit은 후속.
- 전 crate 테스트 합계 **136** (storage 16→17 · rest-api 28→29 등).

## [1.35.0] - 2026-06-16
### 새 기능
- **파일 첨부 (Phase 4, D37) — 로컬 FS** — 메타데이터와 바이트를 분리한 헥사고날 첨부.
  - **분리**: 메타(`attachments` 테이블 V14: filename/size/content_type/url)는 `AttachmentRepository`(PgStore), 바이트는 **`BlobStore` 포트**(domain) 뒤로 — 로컬 FS 구현(`LocalFsBlobStore`), MinIO/S3는 같은 포트의 후속 adapter(D37). 둘 다 domain은 포트로만 안다(P2/P6).
  - **흐름**: 우리 전송은 비동기(gateway→Router→actor라 업로드 시점에 message_id 없음) → **이미 존재하는 메시지에 사후 첨부**로 단순화. `POST /channels/:cid/messages/:mid/attachments`(멀티파트, 작성자 + ATTACH_FILES, 8 MiB 상한) → 바이트는 BlobStore(key=첨부 Snowflake id), 메타는 DB. `GET .../attachments`(목록, VIEW_CHANNEL) · `GET /attachments/:id`(다운로드, 채널 VIEW_CHANNEL + content-type/disposition).
  - `domain`: 신규 **`attachment`**(`Attachment`/`NewAttachment`) + **`blob`**(`BlobStore` 포트, dyn 박스 future) 모듈 + `AttachmentRepository` + `AttachmentId`/`WebhookId` id + Store 합류. (1.12→1.13)
  - `storage`: PgStore 구현 + **`LocalFsBlobStore`**(tokio fs, 경로 탈출 키 차단) + **마이그레이션 V14**(`attachments`, message FK CASCADE). 통합 +1, blob 유닛 +1. (1.12→1.13)
  - `rest-api`: `routes/attachment`(업로드/목록/다운로드) + `AppState`에 `Arc<dyn BlobStore>` 주입 + axum `multipart` 기능. MemStore attachment + MemBlobStore + 통합 +1. (1.14→1.15)
  - `cli`: `upload-attachment`/`attachments`/`download-attachment` + reqwest `multipart`. (1.13→1.14)
  - server: `LocalFsBlobStore::new(env ATTACHMENT_DIR, 기본 ./attachments)` 주입. (1.3→1.4)
- 문서(R2): decisions **D37** 구현 노트, `api/rest.md`(첨부 라우트), `database/02-schema.md`(attachments V14 구현 표시). TODO 체크.
- **라이브 e2e**: 단일노드 — send → 히스토리에서 mid 확보 → upload(22 bytes) → 목록 → download → **바이트 diff 일치** + 로컬 FS에 blob 파일 기록 확인. rest-api 통합(업로드/목록/다운로드/비작성자403/비멤버403/없는첨부404).
- seam: 메시지 전송 시 동시 첨부(현재 사후 첨부)·이미지 width/height 추출·MinIO·바이트 스트리밍(현재 메모리 적재 8 MiB 상한)은 후속.
- 전 crate 테스트 합계 **134** (storage 14→16 · rest-api 27→28 등).

## [1.34.0] - 2026-06-16
### 새 기능
- **스레드 / 포럼 채널 (Phase 4, D44)** — Realm 통일 추상(P4)의 또 한 번의 배당.
  - **핵심**: 스레드 = 부모 채널과 **같은 Realm의 `channels`(kind='thread', parent_id) 한 행** + `thread_meta` 보강. 메시징·팬아웃(D12 구독자표)·권한·자동구독(D13)을 **길드 채널과 무변경 공유** — gateway/node/protocol 코드 추가 0. 라이브에서 스레드 메시지가 기존 send 경로로 그대로 흐름 확인.
  - **포럼**: `channels.kind='forum'` 채널을 일반 채널 생성(`?kind=forum`)으로. 스레드는 text/announcement/forum 아래 생성(thread/dm 아래 금지).
  - **message_count**: thread_meta 컬럼은 스키마 충실성용, 실제 카운트는 **조회 시 messages 집계**(쓰기 경로 비결합).
  - `domain`: 신규 **`thread`** 모듈(`Thread`/`NewThread`) + `ThreadRepository`(create/get/list/archive) + Store 합류. `default_everyone`에 **CREATE_PUBLIC_THREADS·SEND_MESSAGES_IN_THREADS** 추가(스레드 기본 사용 가능, Discord 정렬). (1.11→1.12)
  - `storage`: PgStore 구현(채널+메타 트랜잭션, 집계 SELECT) + **마이그레이션 V13**(`thread_meta`). 종단 통합 +1. (1.11→1.12)
  - `rest-api`: `POST`/`GET /channels/:id/threads`(생성 CREATE_PUBLIC_THREADS·목록 VIEW_CHANNEL), `PATCH /channels/:id/thread`(아카이브, 소유자/MANAGE_THREADS) + 길드 채널 생성에 `kind` 지원. `THREAD_CREATE`/`THREAD_UPDATE` 팬아웃(D39). MemStore thread + 통합 +1. (1.13→1.14)
  - `cli`: `create-thread`/`list-threads`/`archive-thread` + `create-channel --kind`. (1.12→1.13)
- 문서(R2): decisions **D44**(스레드=채널 P4 재사용), `api/rest.md`(스레드 라우트 구현), `api/gateway.md`(THREAD_* 이벤트), `database/02-schema.md`(thread_meta V13 구현 표시). TODO 체크.
- **라이브 e2e**: 단일노드 — forum 채널 생성 → 스레드 생성(parent/owner) → **스레드에 send ×2(기존 경로)** → list-threads(msgs=2 집계) → 스레드 히스토리 → archive true/false. rest-api 통합(생성/목록/비멤버403/아카이브 소유자·MANAGE_THREADS·owner단축/잘못된 kind 400).
- seam: 스레드 join(명시 구독)은 realm 자동구독으로 대체(P4) · SEND_MESSAGES_IN_THREADS 분리 강제·포럼 태그(`forum_tags`)·자동 아카이브 만료는 후속.
- 전 crate 테스트 합계 **131** (storage 13→14 · rest-api 26→27 등).

## [1.33.0] - 2026-06-16
### 새 기능
- **메시지 전문검색 (Phase 4, Q10/D28 §5) — Postgres FTS** — 외부 검색엔진 없이 Postgres 내장 전문검색.
  - **모델**: `messages.content_tsv`(tsvector **STORED 생성 컬럼**, config=`simple` — 언어 무관·스테밍 없음, 혼합언어 안전) + **GIN 인덱스**(`ix_messages_fts`). 04 §5 청사진 그대로. 파티셔닝(D28) 전환 시 GIN은 파티션에 상속.
  - **쿼리**: `websearch_to_tsquery('simple', $q)` — 사용자 입력 안전 파싱(따옴표/OR/`-` 지원, 파서 에러 없음). `content_tsv @@` 매칭을 GIN이 가속.
  - **권한**: 길드 단위 검색이되 결과는 **VIEW_CHANNEL 있는 채널로 한정**(채널별 오버라이드 존중, D17). 멤버 아니면 403. 권한 필터는 라우트가, FTS는 storage가.
  - `domain`: `MessageRepository::search_messages(realm, channels, query, limit)` 포트. (1.10→1.11)
  - `storage`: PgStore 구현 + **마이그레이션 V12**(`0012_message_fts.sql`). FTS 종단 통합 테스트 +1. (1.10→1.11)
  - `rest-api`: `GET /guilds/:id/messages/search?content=&limit=`(VIEW_CHANNEL 채널 필터). MemStore search + 권한필터 통합 +1. (1.12→1.13)
  - `cli`: `search --token --guild --content [--limit]`. (1.11→1.12)
- 문서(R2): decisions **Q10 해결**(§5 미결 → 구현), `api/rest.md`(§0 구현현황 + §7 검색), `database/04-partitioning-and-distributed.md` §5(V12 구현 표시). TODO 체크.
- **라이브 e2e**: 단일노드 — register(PoW)→create-guild→send ×3→`search "fox"`(**2건, 최신순**)·`"dog"`(1건)·`"elephant"`(0건). storage 통합(매칭/채널필터/삭제제외/빈채널) + rest-api 통합(매칭/빈결과/빈쿼리400/채널 VIEW_CHANNEL deny 필터/비멤버 403).
- seam: 길드 단위(전역 검색 X) · author/before/after 필터·하이라이트·랭킹(ts_rank)은 후속.
- 전 crate 테스트 합계 **129** (storage 12→13 · rest-api 25→26 등).

## [1.32.0] - 2026-06-15
### 새 기능
- **TOTP MFA (Phase 4, D19)** — 2단계 인증(RFC 6238). 인증/봇방지 묶음 마무리.
  - **흐름**: `enable`(secret+otpauth URI 발급, **미저장**) → `verify`(secret+code 확인 시 저장=활성, **락아웃 방지** — 미확인 secret로 안 잠김) → `disable`(코드 확인 후 제거). 로그인은 MFA 활성 유저면 토큰 대신 `{mfa_required:true}` → `POST /auth/mfa/totp`(비번 재확인+코드)로 토큰.
  - **저장**: secret(raw)은 `users.mfa_totp_secret`(BYTEA, **V1에 이미 존재 → 마이그레이션 0**). 민감값이라 `User` 엔티티엔 안 싣고 전용 포트.
  - `auth`: 신규 **`totp`** 모듈(`totp-rs`, P6 검증 크레이트 — SHA1·6자리·30s·skew1: `new_secret`/`otpauth_uri`/`verify`/`generate`/hex). 유닛 +5. (1.1→1.2)
  - `domain`: `UserRepository`에 `set_totp_secret`/`totp_secret` 포트. (1.9→1.10)
  - `storage`: PgStore 구현(UPDATE/SELECT `mfa_totp_secret`). (1.9→1.10)
  - `rest-api`: `/auth/mfa/totp/{enable,verify,disable}` + `/auth/mfa/totp`(2단계) + login MFA 게이트. MemStore totp + MFA 전체흐름 통합 +1. (1.11→1.12)
  - `cli`: `mfa-enable`/`mfa-verify`/`mfa-login` + `totp-code`(인증앱 대역) + login `mfa_required` 분기. (1.10→1.11)
- 문서(R2): decisions **D19** 구현 노트, `api/rest.md`(mfa 엔드포인트 + login mfa_required). TODO 체크.
- **라이브 e2e**: 단일노드 — register(PoW) → mfa-enable → totp-code → verify → login(**mfa_required**) → mfa-login(**토큰 발급**), 틀린 코드 **401**. otpauth URI(`otpauth://totp/discord-v1:user?secret=…&issuer=discord-v1`) 정상.
- seam: 로그인 2단계는 비번 재제출(ticket 미사용) · 백업코드(`mfa_backup_codes`)·WebAuthn은 후속(Phase 5).
- 전 crate 테스트 합계 **127** (auth 18 · rest-api 21+4 · domain 18 · storage 12 · protocol 9 · node 20+2 · gateway 8 등). 마이그레이션 V1~V11(무DB 변경 — mfa 컬럼은 V1 기존).

## [1.31.0] - 2026-06-15
### 새 기능
- **Rate limiting (Phase 4, D32/D18) — Token Bucket per-node** — 봇/폭주 방지. PoW(가입 D18)와 상보적인 **휘발 per-node** 한도.
  - **모델**: 순수 `TokenBucket`(용량 + 초당 리필, 연속 토큰) + per-node `RateLimiter`(`rule:identity`별 버킷, **인메모리 DB-D5 휘발**). 분산 근사(D32): 노드마다 독립 버킷.
  - **적용**: REST 미들웨어(전 라우트). 버킷 클래스 — `/auth/*`=노드 전역(가입/로그인) · 인증=**유저별**(토큰 검증) · 미인증=전역 anon. 초과 시 **429** + `X-RateLimit-{Limit,Remaining,Reset}`·`Retry-After` 헤더. 판정 시각은 주입 clock(DST 결정론).
  - `rest-api`: 신규 **`ratelimit`** 모듈(TokenBucket/RateRule/RateLimiter/미들웨어) + `AppState`에 `RateLimiter` 주입 + router 미들웨어. 유닛 +4, 통합 +1(429). (1.10→1.11)
  - server: `RateLimiter::with_defaults`(auth 20 · user 120 · anon 60, refill 5/60/30) 주입. (1.2→1.3)
- 문서(R2): decisions **D32** 구현 노트, `api/rest.md`(rate limit 구현됨). TODO 체크.
- **라이브 검증**: 단일노드 — cli `scenario` 정상 통과(리미터가 정상 흐름 안 깸) + `/auth/pow-challenge` 30회 폭주 시 **정확히 20개 200 → 10개 429**, 429 응답에 `x-ratelimit-limit:20 / remaining:0 / reset`·`retry-after:1` 헤더 확인.
- seam: 노드별 독립(전역 정밀 한도 아님, D32 근사) · 메시지 전송(gateway 서빙)·gateway WS는 미적용(후속) · 유저-해시 소유 노드 승격(b)은 후속.
- 전 crate 테스트 합계 **121** (rest-api 20+4 · auth 13 · protocol 9 · node 20+2 · domain 18 · storage 12 · gateway 8 등). 마이그레이션 V1~V11(무DB 변경).

## [1.30.0] - 2026-06-15
### 새 기능
- **가입 봇방지 PoW (Phase 4 진입, D18)** — Phase 1 잔여 항목이자 Phase 4 첫 항목. 가입 시 Proof-of-Work 해시 퍼즐을 풀어야 계정 생성.
  - **stateless 멀티노드**: 챌린지를 서버에 저장하지 않고(DB-D5 휘발) **PASETO v4.local 토큰**으로 발급 — 난이도를 인증된 claim(`sub`)에 담아 위변조 차단 + 만료 내장. 어느 노드가 발급해도 공유 키(`POW_SECRET`)로 다른 노드가 검증(D14와 동일 철학).
  - **알고리즘**: 클라가 `sha256(challenge || ":" || nonce)`의 **선행 0비트 ≥ 난이도**(기본 18)가 되는 nonce를 찾아 제출. 서버는 토큰 진위·만료 + (토큰에서 디코드한 인증된 난이도로) 해 검증. **퍼즐 해시만 수제(sha2), 챌린지 MAC은 검증 크레이트(pasetors)** — 수제 암호 금지(P6) 준수.
  - `auth`: 신규 **`pow`** 모듈(`PowKeys` issue_challenge/verify + `solve`/`satisfies`/`leading_zero_bits` + `DEFAULT_DIFFICULTY`) + `AuthError::Pow`. 유닛 +6. (1.0→1.1)
  - `rest-api`: **`GET /auth/pow-challenge`** 발급(`{challenge, difficulty}`) + **`POST /auth/register`가 `pow_challenge`+`pow_nonce` 필수 검증**(실패 400). `AppState`에 `PowKeys` 주입. 통합 +3. (1.9→1.10)
  - server: `POW_SECRET` env 로드(없으면 생성, 멀티노드 경고) + 주입, `gen-keys`가 `POW_SECRET`도 발급. (1.1→1.2)
  - `cli`: register가 챌린지 받기→풀기(`auth::pow::solve` 재사용, 알고리즘 단일 출처)→제출. `auth` 의존 추가. (1.9→1.10)
- 문서(R2): decisions **D18** 구현 노트(stateless PASETO·P6 준수·seam), `api/rest.md`(pow-challenge + register PoW 필수), TODO PoW 항목 체크(Phase 1·4).
- **e2e 검증**: 단일노드 `cli scenario` **PASS** — register가 PoW를 풀고 가입 → 길드 → WS READY → 메시지 전송 → MESSAGE_CREATE 수신까지 종단 통과. `GET /auth/pow-challenge`가 `v4.local.` 토큰 발급 확인.
- seam: 챌린지 미저장(stateless) → 만료(PASETO 기본 1h)까지 같은 해 replay 가능(비용 게이트=난이도) · 로그인 PoW·rate limit(D32)은 후속.
- 전 crate 테스트 합계 **116** (auth 13 · rest-api 19 · protocol 9 · node 20+2 · domain 18 · storage 12 · gateway 8 등). 마이그레이션 V1~V11(무DB 변경).

## [1.29.0] - 2026-06-15
### 새 기능
- **크로스노드 유저 이벤트 라우팅 (Phase 3, D43)** — D40/D41이 남긴 마지막 seam을 닫는다: 유저 단위 이벤트(`RELATIONSHIP_ADD/_REMOVE`, `MESSAGE_ACK`)가 `UserEmitter`=`Hub`라 **이 노드 로컬 세션에만** 배달됐는데, 대상 유저가 다른 노드에 접속 중이어도 배달되도록 일반화.
  - **디렉터리 재사용**: D42 `node::Presence`의 `user → 호스팅 노드 집합`을 **유저 위치 디렉터리**로 재사용(새 레지스트리 0). `Presence::nodes_for` 신설.
  - **타깃 전송(broadcast 아님)**: presence는 친구 전체로 풀메시 broadcast하지만, 유저 이벤트는 수신자가 특정 유저라 **호스팅 노드에만** 보낸다. 로컬=`Hub::deliver`(detach 버퍼 세션 포함), 원격=노드별로 묶어 `USER_DELIVER`(wire 0x0202) 전송 → 수신 노드가 로컬 세션에 배달. `Hub::deliver`가 세션 없는 유저를 자동 스킵 → stale 디렉터리 무해.
  - **어댑터**: gateway 신규 **`user_route::UserRouter`**(Hub+Presence+Router 결합)가 `UserEmitter` 구현 — `RealmEmitter`=Router(D39)와 대칭. server가 주입(Hub의 옛 UserEmitter 구현은 제거). **포트 시그니처 불변 → rest-api(relationship/read_state) 무변경.**
  - `protocol`: **`USER_DELIVER`(0x0202)** wire(`t, payload, user_ids`) + 라운드트립. (1.3→1.4)
  - `node`: `Presence::nodes_for`(디렉터리 조회) + `Router::send_to`(타깃 전송) + handle_inbound UserDeliver arm. 유닛 +1. (1.5→1.6)
  - `gateway`: 신규 **`user_route`** 모듈(`UserRouter` + `deliver_user`) + Hub의 UserEmitter 구현 이전. 유닛 +2. (1.9→1.10)
  - server: `UserRouter` 주입 + `run_inbound`이 `USER_DELIVER` 분기 처리. (1.0→1.1)
- 문서(R2): decisions **D43**(크로스노드 유저 이벤트 라우팅) + D40/D41/D42/Q2/Q11 seam 갱신, `protocol/node-wire.md`(USER_DELIVER 바디), `api/gateway.md`(유저 emit 크로스노드)·`api/rest.md`·`docs/README.md`(D1~D43/Phase 3 완료) 동기화.
- **라이브 검증 (2노드 mTLS)**: alice@node1·bob@node2 등록 → bob이 node2에서 listen → alice가 node1에서 친구 요청 → **bob이 크로스노드로 `RELATIONSHIP_ADD`(pending_in) 실시간 수신**(D43 이전엔 로컬 한정이라 미배달). mTLS 메시 양방향 연결·presence 디렉터리 전파 확인.
- seam: 디렉터리는 **live(온라인) 세션**만 추적 → 원격 detach-grace 세션은 in-flight 미수신(RESUME/다음 READY로 복구) · 신규 노드 anti-entropy 없음(D42와 동일, Phase 5).
- 전 crate 테스트 합계 **107** (protocol 9 · node 20+2 · domain 18 · storage 12 · rest-api 16 · gateway 8 등). 마이그레이션 V1~V11(무DB 변경).

## [1.28.0] - 2026-06-15
### 새 기능
- **전역 presence (gossip, Phase 3, Q11/D12 → D42)** — 친구 온라인 여부. **D40/D41에서 남긴 크로스노드 유저 라우팅 seam을 닫는다**: Realm 무관 유저 이벤트를 풀메시 gossip broadcast + 로컬 친구 필터로 전 노드에 전파.
  - **모델**: presence = 휘발 상태(DB-D5, 인메모리). user → (status, 그를 호스팅하는 노드 집합) — 노드 집합이 비면 offline("any node hosts → online", 멀티노드 정확). 현재 status는 online/offline(idle/dnd op 3은 후속).
  - **전이**: gateway 세션이 유저의 **첫 live 세션** 연결 시 online, **마지막 live 세션** 종료 시 offline(detach 후 `Hub::live_count`로 판정). 전이 시 `PRESENCE_GOSSIP` 풀메시 브로드캐스트 + 그 유저의 로컬 친구에게 `PRESENCE_UPDATE` 배달.
  - **gossip 수신**: server inbound 루프가 `PRESENCE_GOSSIP`을 받아 view 갱신 + 로컬 친구 통지(재브로드캐스트 없음 — 원본이 전 피어에 이미 전송). **READY 스냅샷에 친구 presence(`presences`) 포함**.
  - `node`: 신규 **`presence::{Presence, Status}`** 레지스트리(노드 레벨 휘발 상태) + `Router::{broadcast, peer_ids}`(풀메시 D4) + `HashRing::node_ids`. 유닛 +2. (1.4→1.5)
  - `protocol`: **`PRESENCE_GOSSIP`(0x0201)** wire(`user_id, node_id, status:u8`) + 라운드트립. (1.2→1.3)
  - `gateway`: 신규 **`presence`** 모듈(set_online/set_offline/apply_gossip/notify_friends/ready_presences) + `Hub::{live_count, session_user}` + 세션 연결/해제 훅 + READY presences + `GatewayState.presence`. (1.8→1.9)
  - server: `Presence` 생성·주입 + `run_inbound`이 `PRESENCE_GOSSIP` 분기 처리(`protocol` 의존 추가). 단일노드는 gossip 피어 0 = 로컬 presence만(정상).
  - 친구 대상 산출은 relationships(D40) 재사용(`list_relationships` filter friend) — 새 repo 메서드 0. domain/storage/rest-api **변경 없음**.
- 문서(R2): decisions **D42**(전역 presence gossip + 크로스노드 유저 라우팅), `protocol/node-wire.md`(PRESENCE_GOSSIP 바디 구체화), `api/gateway.md`(PRESENCE_UPDATE + READY presences), TODO 체크.
- **라이브 검증**: ① 단일노드 — 친구 A 접속→친구 B가 PRESENCE_UPDATE online 수신, A 종료→offline, A READY에 B online. ② **2노드 mTLS — node1의 A 접속이 gossip을 타고 node2의 B에게 PRESENCE_UPDATE로 도달**(크로스노드 seam 닫힘 입증).
- seam: 신규 노드 join 시 과거 presence 동기화(anti-entropy) 없음(델타 only) · idle/dnd(op 3) · presence는 휘발이라 전 노드 재시작 시 리셋 — 모두 후속.
- 전 crate 테스트 합계 **103** (protocol 8 · node 19+2 · domain 18 · storage 12 · rest-api 16 · gateway 6 등). 마이그레이션 V1~V11(presence는 무DB).

## [1.27.0] - 2026-06-15
### 새 기능
- **읽음 상태 (read_states, Phase 3, D41)** — 채널별 `last_read_message_id` + 안 읽은 멘션 수(`mention_count`). Discord UX의 미읽음 배지/멘션 카운트 원천.
  - **ack**: `POST /channels/:cid/messages/:mid/ack` → last_read upsert + 그 이후 살아있는 멘션 수 재계산(한 문장). VIEW_CHANNEL 필요(DM은 default_everyone 폴백).
  - **mention_count 유지**: dispatch 드라이버가 멘션 적재(D39) 직후 대상들의 `mention_count` +1(작성자 제외, 존재 유저만). 새 메시지는 항상 최신이라 단순 증가가 정확.
  - **실시간 `MESSAGE_ACK`**: ack 시 본인 세션들에 통지(다른 기기 동기화) — **`UserEmitter`(D40) 재사용**. **READY 스냅샷에 `read_states` 포함**(자동구독 시점 상태 확보).
  - `domain`: 신규 `read_state` 모듈(`ReadState`) + **`ReadStateRepository`** 포트(ack/bump_mentions/list) → `Store` 합류. (1.8→1.9)
  - `storage`: `read_state` 어댑터(ack 재계산·bump upsert) + **V11 `0011_read_states.sql`**. DB 통합 테스트 +1. (1.8→1.9)
  - `rest-api`: `routes/read_state`(`POST .../ack` + `GET /users/@me/read-states`) + `events` MESSAGE_ACK 페이로드. 통합 테스트 +1(ack 멘션 재계산·권한·경계). (1.8→1.9)
  - `gateway`: dispatch가 멘션 카운트 bump + READY가 read_states 포함. (1.7→1.8)
  - `cli`: `ack`/`read-states`. (1.8→1.9)
- 문서(R2): decisions **D41**(읽음상태·mention_count 유지·MESSAGE_ACK via UserEmitter), `api/rest.md`(ack/read-states 엔드포인트)·`api/gateway.md`(MESSAGE_ACK + READY read_states), TODO 체크.
- **라이브 검증**: owner가 bob 멘션 메시지 2개 전송 → bob read-states `mention_count=2`(실 dispatch bump) / bob READY 스냅샷에 read_states 포함 / bob ack → 0 + 본인 listen 세션이 `MESSAGE_ACK` 실시간 수신.
- 전 crate 테스트 합계 **100** (domain 18 · storage 12 · rest-api 16 등). 마이그레이션 V1~V11 적용.

## [1.26.0] - 2026-06-15
### 새 기능
- **친구 · 차단 (relationships, Phase 3, D40)** — Discord식 방향성 행(A↔B = 양쪽 행 2개)으로 친구 요청/수락/취소·거절/삭제 + 차단/해제. 상태 전이의 원자성(두 행)은 storage 트랜잭션.
  - **상태기계**: 요청=내 행 `pending_out`/상대 `pending_in` → 수락 시 양쪽 `friend`. 차단=내 행 `blocked`+상대 행 제거. 제거=친구/대기는 양쪽, 차단은 내 행만.
  - **DM 차단 게이팅 (permissions.md §5 seam 닫힘)**: 어느 한쪽이라도 차단했으면 **1:1 DM 열기 거부**(rest-api) + **1:1 DM 전송 거부**(gateway `can_send` 후단). 그룹DM은 미적용(Discord 동일).
  - **유저 단위 실시간 통지**: 친구·차단은 Realm 무관 이벤트 → 새 **`UserEmitter` 포트**(D12의 "팬아웃 ↔ 전역 presence 분리"). gateway `Hub`가 구현(대상 유저의 이 노드 로컬 세션에 `RELATIONSHIP_ADD/_REMOVE` 배달), server가 rest-api `AppState`에 주입. ⚠ 크로스노드 유저 라우팅은 전역 presence/gossip(Q11) seam.
  - `domain`: 신규 **`relationship`** 모듈(`RelationKind`/`Relationship`/`mirror`) + **`RelationshipRepository`** 포트(list/get/is_blocked_between/friend_request_or_accept/block/remove) → `Store` 합류. emit 모듈에 **`UserEmitter`** 포트. 유닛 +2. (1.7→1.8)
  - `storage`: `relationship` 어댑터(전이 트랜잭션) + **V10 `0010_relationships.sql`**(`relation_kind` enum + `relationships` 테이블). DB 통합 테스트 +1(친구 생애주기·차단). (1.7→1.8)
  - `rest-api`: `routes/relationship`(`GET`/`PUT`/`DELETE /users/@me/relationships[/:uid]`) + `events` 페이로드 + `AppState.user_emitter` + DM 열기 차단 게이팅. 통합 테스트 +2(친구·차단 상태기계 / 차단→DM 거부). (1.7→1.8)
  - `gateway`: `Hub`가 `UserEmitter` 구현(로컬 세션 배달) + `can_send` 1:1 DM 차단 게이팅. (1.6→1.7)
  - `cli`: `add-friend`/`block-user`/`remove-relationship`/`relationships`. (1.7→1.8)
  - server: rest-api에 Hub를 `UserEmitter`로 주입.
- 문서(R2): decisions **D40**(친구·차단 + UserEmitter 분리, Q11 seam), `permissions.md` §5(차단 강제 구현), `api/rest.md`(relationships 엔드포인트)·`api/gateway.md`(`RELATIONSHIP_*` + UserEmitter 경로), TODO 체크.
- **라이브 검증**: alice→bob 친구 요청(bob 실시간 `RELATIONSHIP_ADD` 수신)→수락→friend / alice가 carol 차단 → 양방향 1:1 DM 열기 403 / 전송 경로 차단(블록 후 양쪽 send 403).
- 전 crate 테스트 합계 **98** (domain 18 · storage 11 · rest-api 15 등). 마이그레이션 V1~V10 적용.

## [1.25.0] - 2026-06-15
### 새 기능
- **DM / 그룹DM (D8/DB-D2, Phase 3)** — Realm 통일 추상(P4)의 쇼케이스. DM·그룹DM도 길드와 같은 `realms`+`channels`(+`members`)라서 **메시징·권한·분산 팬아웃 경로를 무변경으로 재사용**한다(gateway/node/protocol/server 변경 0). DM Realm은 @everyone 역할이 없어 권한 계산이 `default_everyone`으로 폴백 → 멤버면 전송·조회가 길드와 동일 경로로 통과.
  - **1:1 DM**: `dm_pairs(user_lo,user_hi)` 중복 방지(find-or-create) — 같은 두 사람은 항상 같은 채널. 1:1 DM도 자기 Snowflake realm_id 발급(라우팅 해시 통일, DB-D2).
  - **그룹DM**: 자체 realm(kind=group_dm, owner_id) + 채널 1개 + 참가자 members. 소유자만 참가자 추가/타인 제거, 본인 탈퇴 가능, 소유자 탈퇴 불가(고아화 방지).
  - `domain`: 신규 **`dm`** 모듈(`RealmKind`/`RealmInfo`/`DmChannel`/`NewDm`/`NewGroupDm`/`order_pair`) + **`DmRepository`** 포트(find_dm/create_dm/create_group_dm/get_realm) → `Store` 합류. 유닛 2. (1.6→1.7)
  - `storage`: `dm` 어댑터(1:1 트랜잭션 = realms+channels+members+dm_pairs / 그룹 트랜잭션) + **V9 `0009_dm_pairs.sql`**. DB 통합 테스트 +1(find-or-create 멱등·Conflict·그룹). (1.6→1.7)
  - `rest-api`: `routes/dm` — `POST /users/@me/channels`(recipient_id=1:1 find-or-create / recipient_ids=그룹) + `PUT`/`DELETE /channels/:id/recipients/:uid`(소유자 추가·제거 / 본인 탈퇴). `events`에 `CHANNEL_RECIPIENT_ADD/_REMOVE` 페이로드. 통합 테스트 +2(1:1 멱등·멤버 게이팅 / 그룹 참가자 관리·소유자 보호). (1.6→1.7)
  - `cli`: `open-dm`/`create-group-dm`/`add-recipient`/`remove-recipient`. (1.6→1.7)
  - gateway/node/protocol/server: **변경 없음** — DM이 길드와 동일한 Realm 라우팅·자동구독(D13)·팬아웃을 그대로 탄다(P4 배당).
- 문서(R2): decisions **D8**에 DM/그룹DM 구현 노트, `api/rest.md`(구현 현황 표에 DM/recipient 엔드포인트)·`api/gateway.md`(`CHANNEL_RECIPIENT_*` 이벤트), TODO 체크.
- **라이브 검증**: 단일노드 서버에서 alice↔bob 1:1 DM 송수신(bob READY 자동구독→MESSAGE_CREATE 수신) + 그룹DM 생성·소유자 추가·비소유자 추가 403·신규 참가자 메시지 수신.
- 권한/seam: 1:1 차단(blocked) 거부는 relationships 도입 후 seam(permissions.md §5). 신규 참가자 통지는 기존 접속 구독자 대상(D39 seam과 동일).
- 전 crate 테스트 합계 **93** (domain 16 · storage 10 · rest-api 13 등). 마이그레이션 V1~V9 적용.

## [1.24.0] - 2026-06-15
### 새 기능
- **메시지 답장 + 멘션 (D39)** (Phase 3) — 메시지 **생성(MESSAGE_CREATE)** 경로에 얹음(persist-then-fanout, D24).
  - **답장**: `messages.reference_message_id`(구조적 입력)를 송신 경로 전체에 관통 — gateway `POST /channels/:id/messages` 바디 → `Router::route_send`/`route_send_local`(+param) → `RealmCommand::SendMessage` → `RealmEvent::MessageCreated` → wire `REALM_SEND`(크로스노드) → `NewMessage` persist → `MESSAGE_CREATE` payload. gateway가 참조 대상 검증(같은 채널의 살아있는 메시지, 아니면 400).
  - **멘션**: content에서 파생 → 파이프라인 무변경. dispatch 드라이버가 persist 후 `domain::mention::parse_mentions`(`<@id>`/`<@!id>`, 중복제거)로 뽑아 **V8 `message_mentions`**(존재 유저만, UNNEST+WHERE EXISTS+멱등)에 적재 + `MESSAGE_CREATE` payload에 `mentions:[id]` 포함.
  - `domain`: `message::{Message,NewMessage}`에 `reference_message_id`, 신규 `mention::parse_mentions`(+유닛 4), `MessageRepository::add_mentions`. (1.5→1.6)
  - `protocol`: `REALM_SEND`에 `reference_message_id: Option<u64>` 관통(라운드트립 갱신). (1.1→1.2)
  - `node`: `RealmCommand::SendMessage`/`RealmEvent::MessageCreated`/`route_send`/`route_send_local`/`handle_inbound`에 reference 관통. (1.3→1.4)
  - `storage`: create/select에 `reference_message_id`, `add_mentions` 구현 + **V8 `0008_message_mentions.sql`**. DB 통합 테스트 +1(답장 reference + 멘션 존재유저 필터·멱등). (1.5→1.6)
  - `gateway`: dispatch가 reference persist + 멘션 파싱·적재·payload, send 라우트가 `reference_message_id` 검증·전달. (1.5→1.6)
  - `cli`: `send --reply <mid>`. (1.5→1.6)
- 문서(R2): `02-schema.md` message_mentions DDL 수정(원안 nullable PK 무효 → 유저 멘션만 단순화, 역할 멘션 Phase 4), decisions **D39**에 답장·멘션 구현 추가, `api/rest.md`(전송 행)·`api/gateway.md`(MESSAGE_CREATE payload), TODO 체크.
- 전 crate 테스트 합계 **88** (domain 14 · storage 9 등). 마이그레이션 V1~V8 적용.

## [1.23.0] - 2026-06-15
### 새 기능
- **메시지 편집·삭제(소프트)·리액션 (D39)** (Phase 3) — D39 범용 envelope를 그대로 타고 `MESSAGE_UPDATE`/`MESSAGE_DELETE`/`MESSAGE_REACTION_ADD`/`_REMOVE` 실시간 통지(비-persist 팬아웃, 진실은 REST 트랜잭션이 DB에 기록).
  - `domain`: `MessageRepository`에 `get_message/edit_message/soft_delete_message` + 신규 **`ReactionRepository`**(add/remove) → `Store` 합류. (1.4→1.5)
  - `storage`: 편집(`edited_at`)·소프트삭제(`deleted_at`, 히스토리 `deleted_at IS NULL` 필터) 구현 + `reaction` 모듈 + **V7 `0007_reactions.sql`**(유니코드 emoji 1컬럼 PK). DB 통합 테스트 +1(편집/삭제/리액션 종단). (1.4→1.5)
  - `rest-api`: `routes/message`에 `PATCH`/`DELETE /channels/:cid/messages/:mid`(편집=작성자 / 삭제=작성자·MANAGE_MESSAGES) + `PUT`/`DELETE .../reactions/:emoji/@me`(추가=ADD_REACTIONS 채널컨텍스트 / 제거=멤버). `events`에 메시지/리액션 페이로드 빌더. 통합 테스트 +2(편집·삭제 권한·소프트삭제 / 리액션 멱등·제거). (1.4→1.5)
  - `cli`: `edit`/`delete-message`/`react`/`unreact`(emoji URL 인코딩, `urlencoding` 의존). (1.4→1.5)
  - gateway/node/protocol/server: **변경 없음** — 편집/삭제/리액션이 D39 envelope·`RealmEmitter`·dispatch 분기를 그대로 재사용(범용화의 배당).
- 문서(R2): `02-schema.md` reactions DDL 수정(원안 nullable PK 무효 → 유니코드 emoji 1컬럼 단순화 노트, 커스텀 이모지 Phase 4), decisions **D39**에 편집·삭제·리액션 구현 추가, `api/rest.md`·`api/gateway.md`(엔드포인트·MESSAGE_* 페이로드), TODO 체크.
- 권한: 편집=작성자 본인, 삭제=작성자 또는 MANAGE_MESSAGES, 리액션 추가=ADD_REACTIONS(채널 컨텍스트)·제거=본인. 소프트 삭제는 히스토리에서 제외.
- 전 crate 테스트 합계 **83** (storage 8 · rest-api 11 등). 마이그레이션 V1~V7 적용.

## [1.22.0] - 2026-06-15
### 새 기능
- **멤버 관리 + 범용 Realm 이벤트 팬아웃 (D39)** (Phase 3) — 멤버 조회/nick수정/추방·탈퇴 REST + `GUILD_MEMBER_ADD/_UPDATE/_REMOVE` 실시간 통지. 메시지 전용이던 팬아웃 경로를 **범용 `(t, payload)` envelope**로 일반화(P4).
  - `domain`: `member::Member` 엔티티 + **`emit::RealmEmitter` 포트**(repo 포트와 같은 자리, `dyn` 주입용 박스 future). `GuildRepository`에 `get_member/list_members/update_member_nick/remove_member` 추가 → `Store`. (1.3→1.4)
  - `storage`: 멤버 4메서드 구현(역할 `array_agg`로 N+1 회피). DB 통합 테스트 +1(목록/닉/역할/제거+CASCADE). (1.3→1.4)
  - `protocol`: `REALM_FANOUT`(0x0103) 바디를 `realm_id,t,payload,user_ids`로 일반화 + `REALM_EMIT`(0x0104) 신설(비소유→소유 위임). 라운드트립 테스트 +1. (1.0→1.1)
  - `node`: `RealmEvent::Broadcast` + `RealmCommand::Broadcast`, `LocalDelivery`를 `(t,payload)` envelope로, `Router::fanout(realm,t,payload,targets)` 일반화 + `route_emit`(route_send 대칭) + `RealmEmitter` 구현. 테스트 +2(액터 Broadcast, 크로스노드 emit→fanout 종단). (1.2→1.3)
  - `gateway`: dispatch 드라이버를 이벤트 종류로 분기(메시지만 persist, 멤버는 비-persist) + `deliver_local` 범용화(payload 1회 역파싱). (1.4→1.5)
  - `rest-api`: `routes/member`(GET 목록/단건, PATCH nick, DELETE 추방·탈퇴, `@me` 지원) + `events`(멤버 페이로드 빌더) + invite redeem이 `GUILD_MEMBER_ADD` emit. `AppState`에 `Arc<dyn RealmEmitter>` 주입. 통합 테스트 +2(목록·셀프닉·탈퇴 emit / 추방 권한·소유자 보호). serde_json 정식 의존. (1.3.1→1.4)
  - `server`: Router를 emit 포트로 rest-api에 주입.
  - `cli`: `members`/`set-nick`/`kick`/`leave`(@me) 명령.
- 권한: 조회=멤버, nick=본인 `CHANGE_NICKNAME`/타인 `MANAGE_NICKNAMES`, 추방 `KICK_MEMBERS`, 소유자 추방·탈퇴 불가(고아화 방지).
- 문서(R2): decisions **D39** 신설, `node-wire.md` §4/§5(REALM_FANOUT 일반화·REALM_EMIT), `api/rest.md`·`api/gateway.md`(멤버 엔드포인트·GUILD_MEMBER 페이로드), TODO 멤버 관리 체크.
- 전 crate 테스트: protocol 7 · node 17+2 · gateway 6 · rest-api 9(+2) · storage 7(+1, DB). 합계 80.

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
