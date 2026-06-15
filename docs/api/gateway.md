# API — Gateway (WebSocket) 이벤트/명령 카탈로그

> 실시간 양방향 계층 (`gateway` crate). 클라가 1개 영구 WS 연결로 이벤트를 push 받음. **Discord Gateway 모델 차용.**
> 관련 결정: D2(WS), D13(자동구독), D24(seq/RESUME), D31(JSON 페이로드). REST는 [rest.md](rest.md).

---

## 0. 규약

- 엔드포인트: `wss://<node>/gateway?v=1&encoding=json`
- **페이로드 = JSON** (클라 엣지, D31). 노드↔노드의 수제 바이트와 별개.
- 공통 프레임:
```json
{ "op": 0, "d": { ... }, "s": 42, "t": "MESSAGE_CREATE" }
```
| 필드 | 의미 |
|---|---|
| `op` | opcode (아래 표) |
| `d` | 데이터 페이로드 |
| `s` | 시퀀스 번호 (DISPATCH에만, RESUME 재생용, D24) |
| `t` | 이벤트 이름 (DISPATCH에만) |

- ID는 JSON 문자열 (REST와 동일 규약).

---

## 1. Opcodes

| op | 이름 | 방향 | 설명 |
|---|---|---|---|
| 0 | DISPATCH | S→C | 이벤트 전달 (`t`,`s` 포함) |
| 1 | HEARTBEAT | C→S | 하트비트 (마지막 `s` 포함) |
| 2 | IDENTIFY | C→S | 최초 인증 (access 토큰) |
| 3 | PRESENCE_UPDATE | C→S | 상태 변경 (online/idle/dnd) |
| 6 | RESUME | C→S | 끊긴 세션 재개 (session_id + last seq) |
| 7 | RECONNECT | S→C | 재연결 요청 |
| 9 | INVALID_SESSION | S→C | 세션 무효 (재IDENTIFY 필요) |
| 10 | HELLO | S→C | 연결 직후, `heartbeat_interval` 안내 |
| 11 | HEARTBEAT_ACK | S→C | 하트비트 응답 |

---

## 2. 연결 수명주기

```
C ──연결──▶ S
S ──HELLO(heartbeat_interval)──▶ C
C ──IDENTIFY(token)──▶ S
S ──READY(user, realms, session_id)──▶ C      # 초기 상태 스냅샷
   (이후) C ──HEARTBEAT(s)──▶ S ──HEARTBEAT_ACK──▶ C   # 주기 반복
   (이후) S ──DISPATCH(t, d, s)──▶ C            # 이벤트 푸시

[끊김 후 재연결]
C ──RESUME(session_id, token, last_s)──▶ S
S ──(놓친 이벤트 재생: last_s+1 ~ 현재, 원래 seq 보존)──▶ C
S ──RESUMED(s=현재 last seq)──▶ C
  └ 토큰 불일치 / 버퍼 밖 gap / 미지·만료 세션 ──INVALID_SESSION──▶ C → 재IDENTIFY + REST 재조회 (D24)
```

- **IDENTIFY** 성공 시 입장한 Realm들을 **자동 구독**(D13) → 해당 Realm 이벤트가 push됨.
- 세션 소유 노드(Hub)가 **per-session seq + bounded 재생 버퍼**(기본 256 프레임)를 들고 있음 (D24).
  소켓이 끊겨도 버퍼·구독·seq는 **유예(grace, 기본 90s)** 동안 유지 → RESUME이 그 버퍼에서 재생. 유예 후 purge.
- **RESUME 자격증명 = `resume_token`**(CSPRNG 256-bit, READY에서 1회 발급, D20). 추측불가 — session_id만으론 재개 불가.
- RESUME은 **세션을 보유한 동일 노드**로만 가능(버퍼가 노드 로컬). 다른 노드면 INVALID_SESSION → 재IDENTIFY.
- `RESUMED`는 별도 opcode가 아니라 **DISPATCH 형태**(`t="RESUMED"`, `s`=현재 마지막 seq, 새 이벤트 아님).

### RESUME 페이로드
```json
{ "op": 6, "d": { "session_id": "<READY의 session_id>", "token": "<READY의 resume_token>", "seq": 42 } }
```

### IDENTIFY 페이로드
```json
{ "op": 2, "d": { "token": "<PASETO access>", "properties": { "os": "linux", "client": "cli" } } }
```

### READY 페이로드 (초기 스냅샷)
```json
{ "op": 0, "t": "READY", "s": 1, "d": {
    "session_id": "abc",
    "resume_token": "<CSPRNG hex, RESUME 자격증명, D20>",
    "user": { "id": "...", "username": "..." },
    "realms": [ { "id": "...", "kind": "guild", "channels": [...], "roles": [...] } ],
    "relationships": [...],
    "read_states": [...]
} }
```

---

## 3. DISPATCH 이벤트 (`t`)

### 메시지
| 이벤트 | 트리거 |
|---|---|
| `MESSAGE_CREATE` | 메시지 전송 (persist 후 팬아웃, D24) |
| `MESSAGE_UPDATE` | 편집 |
| `MESSAGE_DELETE` | 삭제 (소프트) |
| `MESSAGE_REACTION_ADD` / `_REMOVE` | 리액션 |
| `TYPING_START` | 타이핑 |

### 채널 / 길드
| 이벤트 | 트리거 |
|---|---|
| `CHANNEL_CREATE` / `_UPDATE` / `_DELETE` | 채널 변경 |
| `THREAD_CREATE` / `_UPDATE` | 스레드 생성/아카이브 (D44) |
| `GUILD_CREATE` | 길드 입장/로드 |
| `GUILD_UPDATE` / `_DELETE` | 길드 변경/삭제 |
| `GUILD_MEMBER_ADD` / `_UPDATE` / `_REMOVE` | 멤버 변동 |
| `GUILD_ROLE_CREATE` / `_UPDATE` / `_DELETE` | 역할 변동 |
| `GUILD_BAN_ADD` / `_REMOVE` | 밴 |

### 유저 / 상태
| 이벤트 | 트리거 |
|---|---|
| `PRESENCE_UPDATE` | 친구/멤버 상태 변경 (전역 presence, gossip, D12) |
| `RELATIONSHIP_ADD` / `_REMOVE` | 친구/차단 변동 |
| `USER_UPDATE` | 내 프로필 변경 |

> 이벤트 페이로드 `d`는 해당 엔티티의 JSON (스키마 [../database/02-schema.md](../database/02-schema.md) 대응).

### 구현 현황 (Phase 3 D39~D43 + Phase 4 D44)
- 구현된 DISPATCH: `MESSAGE_CREATE` / `MESSAGE_UPDATE` / `MESSAGE_DELETE`, `MESSAGE_REACTION_ADD` / `_REMOVE`, `MESSAGE_ACK`(읽음, D41), `GUILD_MEMBER_ADD` / `_UPDATE` / `_REMOVE`, `CHANNEL_RECIPIENT_ADD` / `_REMOVE`(그룹DM 참가자 변동, D8), `THREAD_CREATE` / `THREAD_UPDATE`(스레드, D44), `RELATIONSHIP_ADD` / `_REMOVE`(친구·차단, D40), `PRESENCE_UPDATE`(전역 presence, D42), `READY`/`RESUMED`.
- **THREAD_CREATE/_UPDATE**(D44): 스레드 = 부모와 같은 Realm의 `channels`(kind='thread') 행이라 Realm 구독자표(D12)로 그대로 팬아웃. payload: `{ "id", "realm_id", "parent_id", "name", "owner_id", "archived", "auto_archive", "message_count" }`. 트리거: `POST /channels/:id/threads`(CREATE) · `PATCH /channels/:id/thread`(UPDATE=아카이브). 스레드 메시지는 별도 이벤트 없이 일반 `MESSAGE_CREATE`(채널=스레드 id).
- **PRESENCE_UPDATE**(D42): 친구가 온/오프라인 전이하면 그 친구를 둔 노드가 배달. payload `{ "user": { "id": "..." }, "status": "online"|"offline" }`. 크로스노드는 `PRESENCE_GOSSIP`(node-wire 0x0201) 풀메시 broadcast로 전파 → 각 노드가 로컬 친구에게 배달. **READY 스냅샷에 `presences`**(현재 온라인인 친구 목록) 포함. (idle/dnd 설정용 C→S op 3은 후속.)
- **READY 스냅샷에 `read_states` 포함**(D41): `[{ "channel_id", "last_read_message_id"|null, "mention_count" }]`. `MESSAGE_ACK`(유저 emit, 본인 다른 기기 동기화) payload: `{ "channel_id", "message_id", "mention_count" }`.
- **이벤트 emit 경로 2종**: ① Realm 단위(구독자표 D12, `RealmEmitter`) — 메시지/멤버/recipient. ② **유저 단위**(`UserEmitter`, D40) — 친구·차단·읽음 등 Realm 무관 이벤트. **D43부터 크로스노드**: 로컬 세션은 `Hub`로, 다른 노드에 접속한 대상 유저는 `Presence` 디렉터리(D42)로 호스팅 노드를 찾아 `USER_DELIVER`(node-wire 0x0202)로 타깃 배달. (포트 시그니처 불변 → REST 라우트 무변경.)
- `RELATIONSHIP_ADD` payload: `{ "user": { "id": "...", "username": "..." }, "kind": "pending_in"|"pending_out"|"friend"|"blocked" }` (수신자 관점). `RELATIONSHIP_REMOVE`: `{ "user": { "id": "..." } }`.
- DM/그룹DM(D8)은 길드와 **동일한 자동구독(D13)·팬아웃 경로**를 탄다 — DM Realm에 입장(=멤버)이면 READY가 그 realm을 자동구독해 `MESSAGE_CREATE` 등을 그대로 수신(별도 분기 없음, P4). `CHANNEL_RECIPIENT_*`는 멤버 이벤트와 같은 범용 envelope·RealmEmitter로 팬아웃:
```json
// CHANNEL_RECIPIENT_ADD
{ "op": 0, "t": "CHANNEL_RECIPIENT_ADD", "d": {
    "realm_id": "...", "channel_id": "...", "user": { "id": "...", "username": "..." } } }
// CHANNEL_RECIPIENT_REMOVE
{ "op": 0, "t": "CHANNEL_RECIPIENT_REMOVE", "d": { "realm_id": "...", "channel_id": "...", "user": { "id": "..." } } }
```
- 메시지 편집·삭제·리액션도 범용 envelope(D39)로 같은 구독자표 위에서 팬아웃(비-persist; 진실은 REST 트랜잭션이 이미 기록). 페이로드:
```json
// MESSAGE_UPDATE (편집)
{ "op": 0, "t": "MESSAGE_UPDATE", "d": {
    "id": "...", "channel_id": "...", "author": { "id": "..." }, "content": "새 내용", "edited": true } }
// MESSAGE_DELETE (소프트)
{ "op": 0, "t": "MESSAGE_DELETE", "d": { "id": "...", "channel_id": "..." } }
// MESSAGE_REACTION_ADD / _REMOVE
{ "op": 0, "t": "MESSAGE_REACTION_ADD", "d": {
    "message_id": "...", "channel_id": "...", "user_id": "...", "emoji": "👍" } }
```
- 트리거: `PATCH`/`DELETE /channels/:cid/messages/:mid`(편집/삭제) · `PUT`/`DELETE .../reactions/:emoji/@me`(리액션).
- `MESSAGE_CREATE`는 답장·멘션 필드를 포함(D39):
```json
{ "op": 0, "t": "MESSAGE_CREATE", "d": {
    "id": "...", "channel_id": "...", "author": { "id": "..." }, "content": "<@10> 안녕",
    "nonce": null, "reference_message_id": "456", "mentions": ["10"] } }
```
- `reference_message_id`=답장 대상(없으면 null), `mentions`=content에서 파싱한 유저 id(`<@id>`/`<@!id>`, 존재 유저로 한정).
- 멤버 이벤트는 범용 envelope(D39)로 같은 구독자표(D12) 위에서 팬아웃된다. 페이로드:
```json
// GUILD_MEMBER_ADD / GUILD_MEMBER_UPDATE
{ "op": 0, "t": "GUILD_MEMBER_ADD", "s": 7, "d": {
    "realm_id": "...", "user": { "id": "...", "username": "..." },
    "nick": null, "roles": ["..."]
} }
// GUILD_MEMBER_REMOVE
{ "op": 0, "t": "GUILD_MEMBER_REMOVE", "s": 8, "d": {
    "realm_id": "...", "user": { "id": "..." }
} }
```
- 트리거: `POST /invites/{code}`(ADD) · `PATCH /guilds/{id}/members/{uid}`(UPDATE) · `DELETE .../members/{uid}`(REMOVE). 대상 = 그 Realm의 **현재 접속 구독자**(신규 합류자 본인은 redeem 응답/다음 READY로 상태 확보, D13).

---

## 4. 팬아웃 경로 (요약)
```
작성자 → 소유 노드의 Realm 액터 → persist(Postgres) → 구독자표(D12)로 대상 노드 산출
   → 각 노드의 세션으로 DISPATCH(JSON) push → 세션 seq 증가 + 재생버퍼 적재
```
- 느린 클라는 backpressure로 끊김(D27) → RESUME/재조회로 복구.

## 5. 미정 디테일
- Gateway intents(구독 범위 최적화) — Discord식, 후속 검토.
- 압축(zstd) — 후속 최적화.
- 시나리오 스크립트 포맷(CLI 테스트 하네스, Q9) — Phase 1.
