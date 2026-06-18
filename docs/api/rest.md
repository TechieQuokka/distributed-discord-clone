# API — REST 엔드포인트 카탈로그

> 무상태 REST 계층 (`rest-api` crate). 인증·CRUD·요청-응답. 실시간은 [gateway.md](gateway.md).
> 관련 결정: D1(API+CLI), D14(PASETO), D17(권한), D34(nonce), D38(커서). 스키마: [../database/02-schema.md](../database/02-schema.md).

---

## 0. 전역 규약

| 항목 | 규칙 |
|---|---|
| Base URL | `/api/v1` |
| 전송 | HTTPS (`wss`/`https`, 로컬 self-signed, D16) |
| 인증 | `Authorization: Bearer <PASETO access token>` (D14) |
| **ID 표현** | Snowflake는 JSON에서 **문자열**로 직렬화 (JS의 53비트 정수 절단 회피) |
| 시각 | ISO-8601 (`TIMESTAMPTZ`) |
| 본문 | `application/json` (멀티파트는 첨부 업로드만) |
| 멱등성 | 메시지 전송 시 `nonce` (D34) |
| 페이지네이션 | Snowflake 커서 `before`/`after`/`around` + `limit` (D38) |
| Rate limit | Token Bucket **per-node**(D32, 구현됨) — `/auth/*` 전역·인증 유저별·미인증 anon 버킷. 초과 429 + 헤더 |

### 표준 에러 형식
```json
{ "code": 40001, "message": "Unauthorized", "errors": { "field": ["detail"] } }
```
| HTTP | 의미 |
|---|---|
| 400 | 잘못된 요청 / 검증 실패 |
| 401 | 인증 없음/만료 |
| 403 | 권한 없음 (비트마스크 계산 실패) |
| 404 | 리소스 없음 |
| 409 | 충돌 (예: 이미 존재) |
| 429 | Rate limited (`Retry-After`, `X-RateLimit-*`) |

### Rate limit 헤더
```
X-RateLimit-Limit, X-RateLimit-Remaining, X-RateLimit-Reset, Retry-After
```

### 구현 현황 (Phase 1–4, v1.32.x)

> 아래 §1~ 표는 **전체 카피 대상 청사진**이다. **현재 실제 구현된 엔드포인트**는 다음과 같다(나머지는 후속 Phase).

| 메서드 | 경로 | 강제 권한 | 비고 |
|---|---|---|---|
| GET | `/auth/pow-challenge` | — | 가입용 PoW 챌린지 발급 `{challenge, difficulty}` (D18, stateless PASETO v4.local) |
| POST | `/auth/{register,login,refresh}` | — | PASETO + refresh 회전/재사용탐지 (D14). **register는 `pow_challenge`+`pow_nonce` 필수** (D18). login은 MFA 활성 시 `{mfa_required:true}` |
| POST | `/auth/mfa/totp/{enable,verify,disable}` | enable/verify/disable=인증 | TOTP 설정(D19): enable=secret+URI 발급 / verify=확인 후 저장(활성) / disable=코드 확인 후 제거 |
| POST | `/auth/mfa/totp` | — | 로그인 2단계: `username`+`password`+`code` → 토큰 (D19) |
| POST | `/guilds` | — | 길드 + @everyone 역할 + 기본 general 채널 |
| POST | `/guilds/{id}/channels` | MANAGE_CHANNELS | `kind` 선택(text/voice/category/announcement/forum, D44) |
| POST / GET | `/channels/{id}/threads` | 생성=CREATE_PUBLIC_THREADS / 목록=VIEW_CHANNEL | 스레드 생성·목록 (D44, 스레드=채널 P4). 생성 시 `THREAD_CREATE` 팬아웃 |
| PATCH | `/channels/{id}/thread` | 소유자 또는 MANAGE_THREADS | 스레드 아카이브/해제 → `THREAD_UPDATE` 팬아웃 (D44) |
| GET / POST | `/guilds/{id}/roles` | GET=멤버 / POST=MANAGE_ROLES (권한상승 방지) | |
| GET | `/guilds/{id}/members` | 멤버 | 멤버 목록(nick/joined/역할) |
| GET | `/guilds/{id}/members/{user_id}` | 멤버 | 멤버 단건 조회 |
| PATCH | `/guilds/{id}/members/{user_id}` | 본인=CHANGE_NICKNAME / 타인=MANAGE_NICKNAMES | nick 수정 → `GUILD_MEMBER_UPDATE` 팬아웃 (D39) |
| DELETE | `/guilds/{id}/members/{user_id}` | 타인=KICK_MEMBERS / 본인=leave | 추방/탈퇴 → `GUILD_MEMBER_REMOVE` 팬아웃 (D39) |
| PUT | `/guilds/{id}/members/{user_id}/roles/{role_id}` | MANAGE_ROLES | 역할 부여 |
| POST | `/guilds/{id}/invites` | CREATE_INVITE | **길드 레벨**(청사진의 채널 레벨 §4와 다름 — 단순화 채택) |
| POST | `/invites/{code}` | (멤버 아님 무관) | redeem → members 추가 → `GUILD_MEMBER_ADD` 팬아웃 (D39) |
| POST | `/users/@me/channels` | (인증) | DM 열기 — `recipient_id`=1:1(`dm_pairs` find-or-create, 기존 있으면 200 재사용)·`recipient_ids`=그룹DM 생성 (D8/DB-D2) |
| PUT | `/channels/{id}/recipients/{user_id}` | 그룹 소유자 | 그룹DM 참가자 추가 → `CHANNEL_RECIPIENT_ADD` 팬아웃 |
| DELETE | `/channels/{id}/recipients/{user_id}` | 소유자(타인) / 본인(탈퇴) | 그룹DM 참가자 제거 → `CHANNEL_RECIPIENT_REMOVE` (소유자 탈퇴 불가) |
| GET | `/users/@me/relationships` | (인증) | 내 친구/대기/차단 목록 (D40) |
| PUT | `/users/@me/relationships/{user_id}` | (인증) | `type:friend`=요청/수락(상대 차단 시 403) · `type:block`=차단 → `RELATIONSHIP_ADD`(유저 emit) |
| DELETE | `/users/@me/relationships/{user_id}` | (인증) | 친구 삭제/요청 취소·거절/차단 해제 → `RELATIONSHIP_REMOVE` |
| POST | `/channels/{id}/messages/{mid}/ack` | VIEW_CHANNEL | 채널을 그 메시지까지 읽음 처리(+멘션수 재계산) → `MESSAGE_ACK`(유저 emit) (D41) |
| GET | `/users/@me/read-states` | (인증) | 내 읽음 상태 목록(채널별 last_read + mention_count). READY 스냅샷과 동일 |
| GET / POST | `/users/@me/sync` | (인증) | CRDT 오프라인 동기화 문서(D49, V21). POST=기기 로컬 엔트리 LWW 병합→병합 문서 회신 · GET=현재 병합 문서. `{entries:[{key,value?,ts,node}]}`(value null=툼스톤). 충돌 없이 수렴 |
| PUT | `/channels/{id}/permissions/{target_id}` | MANAGE_ROLES | 오버라이드 upsert (DELETE는 후속) |
| GET | `/channels/{id}/messages` | VIEW_CHANNEL + READ_MESSAGE_HISTORY | 히스토리 커서 (D38) |
| GET | `/guilds/{id}/messages/search` | 멤버 (결과는 VIEW_CHANNEL 채널 한정) | 전문검색 `?content=&limit=` (Q10, Postgres FTS `websearch_to_tsquery`, V12 `content_tsv`+GIN) |
| POST | `/channels/{id}/messages` | SEND_MESSAGES | **`gateway` crate가 서빙**(D31) — 채널 컨텍스트 권한 계산 후 persist-then-fanout (D24). `reference_message_id`(답장, 같은 채널 검증)·`<@id>` 멘션 파싱 지원 (D39, V8 `message_mentions`) |
| PATCH | `/channels/{id}/messages/{mid}` | 작성자 본인 | 편집 → `edited_at` 갱신 + `MESSAGE_UPDATE` 팬아웃 (D39) |
| DELETE | `/channels/{id}/messages/{mid}` | 작성자 본인 또는 MANAGE_MESSAGES | 소프트 삭제(`deleted_at`) + `MESSAGE_DELETE` 팬아웃 (D39) |
| PUT/DELETE | `/channels/{id}/messages/{mid}/reactions/{emoji}/@me` | ADD_REACTIONS(추가) / 멤버(제거) | 본인 리액션 추가·제거 → `MESSAGE_REACTION_ADD/_REMOVE` 팬아웃 (D39, V7 `reactions`) |
| POST / GET | `/channels/{id}/messages/{mid}/attachments` | 업로드=작성자+ATTACH_FILES / 목록=VIEW_CHANNEL | 파일 첨부 멀티파트 업로드·목록 (D37, V14 `attachments` + BlobStore 로컬 FS, 8 MiB) |
| GET | `/attachments/{id}` | 채널 VIEW_CHANNEL | 첨부 다운로드 (바이트 + content-type, D37) |
| POST / GET | `/channels/{id}/webhooks` | MANAGE_WEBHOOKS | 웹훅 생성(토큰 1회 반환)·목록 (V15) |
| DELETE | `/webhooks/{id}` | MANAGE_WEBHOOKS | 웹훅 삭제 |
| POST | `/webhooks/{id}/{token}` | URL 토큰 (Bearer 없음) | 웹훅 실행 = 채널에 메시지 게시 → `MESSAGE_CREATE` (액터 우회 seam) |
| GET | `/guilds/{id}/audit-logs` | VIEW_AUDIT_LOG | 감사 로그 `?before=&limit=` (V16, 채널/역할/멤버/웹훅 변경 기록) |

> 권한 계산은 채널 오버라이드까지 적용(D17): `@everyone` → 역할 OR → 채널 오버라이드(@everyone/역할/멤버) → owner/Administrator 단축. 미구현 항목(밴/이모지/스레드/감사로그/리액션/편집·삭제 등)은 TODO Phase 3~4.

---

## 1. Auth (`/auth`)

| 메서드 | 경로 | 설명 |
|---|---|---|
| GET | `/auth/pow-challenge` | 가입용 PoW 챌린지 발급 (D18) |
| POST | `/auth/register` | 계정 생성 (PoW 해답 + username/email/password) |
| POST | `/auth/login` | 로그인 → `{ access, refresh }` (MFA 필요 시 `mfa_required`) |
| POST | `/auth/refresh` | refresh 회전 → 새 `{ access, refresh }` (재사용 탐지, D14) |
| POST | `/auth/logout` | refresh 폐기 |
| POST | `/auth/mfa/totp/enable` | TOTP 시크릿 발급(QR) (D19) |
| POST | `/auth/mfa/totp/verify` | TOTP 코드 검증/활성화 |
| POST | `/auth/mfa/totp` | 로그인 2단계 코드 제출 |
| POST | `/auth/webauthn/register/start` | (인증) Passkey 등록 challenge 발급 → `{ceremony_id, options}` (D19) |
| POST | `/auth/webauthn/register/finish` | (인증) `{ceremony_id, credential}` → Passkey 저장 |
| POST | `/auth/webauthn/login/start` | `{username}` → 인증 challenge `{ceremony_id, options}` |
| POST | `/auth/webauthn/login/finish` | `{ceremony_id, credential}` → 서명 검증 후 `{access, refresh}` (암호 없는 로그인) |
| POST | `/auth/webauthn/login/discoverable/start` | (본문 없음) **usernameless** challenge `{ceremony_id, options}` (D19, v1.49) |
| POST | `/auth/webauthn/login/discoverable/finish` | `{ceremony_id, credential}` → user handle로 유저 식별 → 서명 검증 후 `{access, refresh}` |

> **WebAuthn/Passkeys (Phase 5, D19, 구현됨)**: `webauthn-rs`(P6) 기반 FIDO2 공개키 자격증명. ceremony 중간 상태는 휘발 인메모리(DB-D5, ceremony_id 키). 자격증명은 `webauthn_credentials`(V18, `passkey JSONB`). RP = env `WEBAUTHN_RP_ID`/`WEBAUTHN_RP_ORIGIN`(기본 localhost). 미설정 노드는 404. **usernameless(discoverable, v1.49)**: username 없이 인증기가 resident key로 유저 선택 → `identify_discoverable`이 user handle(Uuid)에서 유저 식별(`conditional-ui` feature). 헤드리스 SoftPasskey는 discoverable 탐색 미지원→실제 인증기 영역. seam: 멀티노드 ceremony 공유(start/finish 같은 노드)는 후속.

---

## 2. Users (`/users`)

| 메서드 | 경로 | 설명 |
|---|---|---|
| GET | `/users/@me` | 현재 유저 |
| PATCH | `/users/@me` | 프로필 수정 (global_name, avatar, bio, status) |
| GET | `/users/{user_id}` | 유저 조회 (공개 프로필) |
| GET | `/users/@me/realms` | 내 Realm 목록 (길드 + 그룹DM + DM) |
| POST | `/users/@me/channels` | DM/그룹DM 열기 → DM Realm의 채널 반환 (기존 있으면 재사용, `dm_pairs` DB-D2) |
| GET | `/users/@me/relationships` | 친구/차단/대기 목록 |
| PUT | `/users/@me/relationships/{user_id}` | 친구 요청/수락 또는 차단 (`kind`) |
| DELETE | `/users/@me/relationships/{user_id}` | 친구 삭제/차단 해제 |
| GET/POST | `/users/@me/sync` | CRDT 오프라인 동기화 문서 (D49) — POST=LWW 병합, GET=조회 |

---

## 3. Guilds (`/guilds`) — Realm(kind=guild)

| 메서드 | 경로 | 설명 |
|---|---|---|
| POST | `/guilds` | 길드 생성 (Realm+guild 행, @everyone 역할 생성) |
| GET | `/guilds/{guild_id}` | 길드 조회 |
| PATCH | `/guilds/{guild_id}` | 길드 수정 (MANAGE_GUILD) |
| DELETE | `/guilds/{guild_id}` | 길드 삭제 (소유자) |
| GET | `/guilds/{guild_id}/channels` | 채널 목록 |
| POST | `/guilds/{guild_id}/channels` | 채널 생성 (MANAGE_CHANNELS) |
| GET | `/guilds/{guild_id}/members` | 멤버 목록 (페이지네이션) |
| GET | `/guilds/{guild_id}/members/{user_id}` | 멤버 조회 |
| PATCH | `/guilds/{guild_id}/members/{user_id}` | 닉/역할 수정 (MANAGE_*) |
| DELETE | `/guilds/{guild_id}/members/{user_id}` | 추방 (KICK_MEMBERS) |
| GET/PUT/DELETE | `/guilds/{guild_id}/bans/{user_id}` | 밴 조회/생성/해제 (BAN_MEMBERS) |
| GET/POST | `/guilds/{guild_id}/roles` | 역할 목록/생성 (MANAGE_ROLES) |
| PATCH/DELETE | `/guilds/{guild_id}/roles/{role_id}` | 역할 수정/삭제 |
| GET | `/guilds/{guild_id}/invites` | 초대 목록 |
| GET/POST | `/guilds/{guild_id}/emojis` | 이모지 목록/추가 |
| DELETE | `/guilds/{guild_id}/emojis/{emoji_id}` | 이모지 삭제 |
| GET | `/guilds/{guild_id}/audit-logs` | 감사 로그 (VIEW_AUDIT_LOG, 구현됨 V16 — 채널/역할/멤버/웹훅 변경 기록, 최신순 커서) |

---

## 4. Channels (`/channels`) — 메시징 1차 경로 (DM 포함)

| 메서드 | 경로 | 설명 |
|---|---|---|
| GET | `/channels/{channel_id}` | 채널 조회 |
| PATCH | `/channels/{channel_id}` | 채널 수정 (이름/토픽/슬로우모드) |
| DELETE | `/channels/{channel_id}` | 채널 삭제 |
| GET | `/channels/{channel_id}/messages` | 히스토리 (`before`/`after`/`around`/`limit`, D38) |
| GET | `/channels/{channel_id}/messages/{message_id}` | 단일 메시지 |
| POST | `/channels/{channel_id}/messages` | 메시지 전송 (`content`, `nonce`, `embeds`, 첨부) → persist-then-fanout (D24) |
| PATCH | `/channels/{channel_id}/messages/{message_id}` | 편집 (작성자) |
| DELETE | `/channels/{channel_id}/messages/{message_id}` | 삭제 (소프트, 작성자/MANAGE_MESSAGES) |
| PUT/DELETE | `/channels/{channel_id}/messages/{message_id}/reactions/{emoji}/@me` | 리액션 추가/제거 |
| GET | `/channels/{channel_id}/messages/{message_id}/reactions/{emoji}` | 리액션한 유저 목록 |
| GET/PUT/DELETE | `/channels/{channel_id}/pins/{message_id}` | 고정 목록/추가/해제 |
| PUT/DELETE | `/channels/{channel_id}/permissions/{overwrite_id}` | 권한 오버라이드 설정/제거 (DB-D4) |
| POST | `/channels/{channel_id}/typing` | 타이핑 시작 → TYPING_START 팬아웃 |
| POST | `/channels/{channel_id}/invites` | 채널 초대 생성 |
| POST / GET | `/channels/{channel_id}/threads` | 스레드 생성·목록 (구현됨, D44 — 스레드=channels kind='thread'+parent_id) |
| PATCH | `/channels/{channel_id}/thread` | 스레드 아카이브/해제 (소유자/MANAGE_THREADS) |

### 메시지 전송 본문 예시
```json
POST /api/v1/channels/123/messages
{ "content": "hello", "nonce": "client-uuid", "embeds": [], "reference_message_id": "456" }
```

---

## 5. Invites (`/invites`)

| 메서드 | 경로 | 설명 |
|---|---|---|
| GET | `/invites/{code}` | 초대 정보 (미리보기) |
| POST | `/invites/{code}` | 초대 수락 → 길드 입장 (members 추가) |
| DELETE | `/invites/{code}` | 초대 폐기 (MANAGE_GUILD) |

---

## 6. Webhooks (`/webhooks`) — 구현됨 (Phase 4, v1.36, V15)

| 메서드 | 경로 | 설명 |
|---|---|---|
| POST | `/channels/{channel_id}/webhooks` | 웹훅 생성 (MANAGE_WEBHOOKS) → 토큰 1회 반환 |
| GET | `/channels/{channel_id}/webhooks` | 채널 웹훅 목록 (토큰 비노출) |
| POST | `/webhooks/{webhook_id}/{token}` | 웹훅 실행 (Bearer 없음, URL 토큰) = 채널에 메시지 게시 |
| DELETE | `/webhooks/{webhook_id}` | 삭제 (MANAGE_WEBHOOKS) |

> 토큰 = opaque 랜덤 + SHA-256 해시 저장(원본 1회 반환). 실행은 메시지를 persist 후 `MESSAGE_CREATE` 팬아웃하되 Realm 액터를 우회한다(seam).

---

## 7. 검색 (`/guilds/{guild_id}/messages/search`) — 구현됨 (Phase 4, v1.33, Q10)
- **Postgres FTS** 기반 (Q10). `messages.content_tsv`(tsvector STORED 생성 컬럼, config `simple`) + GIN 인덱스(V12). 쿼리는 `websearch_to_tsquery('simple', content)`(따옴표/OR/`-` 안전 파싱).
- 쿼리 파라미터: **`content`**(필수, 검색어), **`limit`**(기본 25, 1~100). `author_id`/`channel_id`/`before`/`after` 필터는 후속.
- **권한**: 멤버여야 하며(아니면 403), 결과는 **VIEW_CHANNEL 있는 채널로 한정**(채널 오버라이드 존중, D17). 빈 검색어는 400.

---

## 부록 — REST와 분산 런타임의 관계
- REST는 무상태 → **아무 노드나** 처리 가능.
- 쓰기(메시지 전송 등)는 내부적으로 **해당 Realm 소유 노드로 라우팅**되어 액터를 거친다 (D9). REST 핸들러는 `node` crate의 라우팅을 호출할 뿐, 직접 상태를 만지지 않는다.
- 읽기(히스토리)는 Postgres 직격 또는 Realm 캐시 (D35).
