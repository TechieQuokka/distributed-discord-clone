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
| Rate limit | per-route/per-user Token Bucket (D18/D32). 응답 헤더에 잔량 표기 |

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

> WebAuthn/Passkeys 엔드포인트는 Phase 5 스트레치.

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
| GET | `/guilds/{guild_id}/audit-logs` | 감사 로그 (VIEW_AUDIT_LOG, Phase 4) |

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
| POST | `/channels/{channel_id}/threads` | 스레드 생성 (Phase 4) |

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

## 6. Webhooks (`/webhooks`) — Phase 4

| 메서드 | 경로 | 설명 |
|---|---|---|
| POST | `/channels/{channel_id}/webhooks` | 웹훅 생성 |
| GET | `/channels/{channel_id}/webhooks` | 채널 웹훅 목록 |
| POST | `/webhooks/{webhook_id}/{token}` | 웹훅 실행 (메시지 전송) |
| DELETE | `/webhooks/{webhook_id}` | 삭제 |

---

## 7. 검색 (`/guilds/{guild_id}/messages/search`) — Phase 4
- Postgres FTS 기반 (Q10). 쿼리 파라미터: `content`, `author_id`, `channel_id`, `before`/`after`.

---

## 부록 — REST와 분산 런타임의 관계
- REST는 무상태 → **아무 노드나** 처리 가능.
- 쓰기(메시지 전송 등)는 내부적으로 **해당 Realm 소유 노드로 라우팅**되어 액터를 거친다 (D9). REST 핸들러는 `node` crate의 라우팅을 호출할 뿐, 직접 상태를 만지지 않는다.
- 읽기(히스토리)는 Postgres 직격 또는 Realm 캐시 (D35).
