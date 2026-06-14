# Database — Overview & 모델링 결정

> PostgreSQL. `sqlx`(컴파일타임 쿼리 검증). 아키텍처 결정 [D11, D23, D28, D38] 등에 종속.
> 이 문서는 **방향성과 모델링 결정의 근거**를 담는다. 실제 DDL은 [02-schema.md](02-schema.md).

---

## 1. 철학

- **Postgres = 진실의 원천(source of truth).** Realm 액터의 인메모리 상태는 여기서 재구축 가능한 캐시일 뿐 (D23).
- **정규화 우선, 성능을 위한 비정규화는 의도적으로만.** (예: `messages.realm_id` 비정규화 = 파티셔닝/라우팅용)
- **DB는 도메인을 모른다 / 도메인은 DB를 모른다.** 리포지토리는 `domain` crate의 trait(port), 구현은 `storage` crate(adapter) (D22).
- **로컬 study지만 실전 스키마.** 인덱스·제약·파티셔닝을 실제처럼 설계하되, 일부는 Phase 4로 미룬다.

## 2. 전역 규약 (Conventions)

| 항목 | 규칙 |
|---|---|
| 기본키 | **`id BIGINT` = Snowflake** (D11). 중앙 시퀀스 없음 — 각 노드가 worker-id로 생성 (D29) |
| 명명 | `snake_case`, 테이블명 복수형 (`users`, `messages`) |
| 시각 | `TIMESTAMPTZ`. 생성시각은 Snowflake에서 추출 가능하나, 쿼리 편의상 일부 컬럼 보존 |
| 삭제 | **소프트 삭제**(`deleted_at`)를 메시지 등 이력이 필요한 곳에. 그 외는 하드 삭제 |
| 권한 | **`BIGINT` 비트마스크** (D17). 63비트로 현재 범위 충분, 초과 시 `NUMERIC`/`bytea`로 승격 |
| 큰 구조 | embeds 등 중첩 구조는 **`JSONB`** |
| FK | 기본 ON DELETE 정책 명시 (CASCADE/SET NULL/RESTRICT) |
| 마이그레이션 | `sqlx migrate`, Phase 0부터 |

## 3. 핵심 모델링 결정 (자문자답)

### DB-D1. Realm을 DB에서 어떻게 표현하나
런타임에선 길드/DM/그룹DM을 `Realm` 하나로 통일했다 (D8). DB도 통일할까?

- *완전 통일안*: `realms` 한 테이블에 nullable 컬럼 떡칠 → 길드 전용 컬럼(verification_level 등)이 DM 행에 NULL로 남아 지저분.
- *완전 분리안*: `guilds`/`dm_channels`/`group_dms` 따로 → 런타임 통일과 어긋나고 라우팅 조인 복잡.

**결론 — 하이브리드:**
- **`realms`** = 라우팅 가능한 컨테이너의 공통 베이스 (id, kind, created_at).
- **`guilds`** = 길드 전용 메타 확장 테이블 (`realm_id` PK = FK).
- DM/그룹DM은 베이스 + `members`로 충분. 그룹DM의 이름/아이콘/소유자만 `realms`의 nullable 공통 컬럼(name, owner_id, icon)에 둔다.
- → 런타임의 "모두 Realm" 추상과 일치하면서, 길드의 풍부한 속성은 격리.

### DB-D2. 1:1 DM의 id — 합성키 문제
D8은 "DM id = 두 user_id 정렬 합성키"라 했으나, `realms.id`는 64비트 Snowflake라 두 ID를 못 담는다.

**결론 — 재조정:**
- 1:1 DM도 **자기 Snowflake id**를 정상 발급받는다 (라우팅 해시는 `hash(realm_id)`로 통일).
- "이 두 사람의 DM이 이미 있나"는 **`dm_pairs(user_lo, user_hi UNIQUE) → realm_id`** 조회 테이블로 해결 (항상 `user_lo < user_hi`).
- 즉 *합성키는 PK가 아니라 유니크 조회 인덱스*로 강등. → D8의 의도(중복 DM 방지)는 지키고 Snowflake 일관성도 유지.
- ⚠ 이 재조정은 decisions.md D8에 각주로 반영해야 함.

### DB-D3. 채널과 메시지
- **모든 메시지는 채널에 속하고, 모든 채널은 Realm에 속한다.**
- DM Realm = 채널 1개를 가진 Realm (`kind='dm'`). 길드 Realm = 채널 다수.
- → "DM은 곧 채널"인 Discord와 약간 다르지만, 채널→Realm 단일 경로로 통일돼 라우팅/권한이 단순.

### DB-D4. 권한 오버라이드
- `channel_overwrites(channel_id, target_id, target_type[role|member], allow, deny)`.
- 권한 계산은 런타임(domain)에서: `@everyone → 역할 OR → 채널 오버라이드 → 멤버 오버라이드 → Admin 통과` (D17). DB는 raw 비트만 저장.

### DB-D5. 무엇을 DB에 두지 *않나* (중요)
- **Presence(온라인 여부)** — 휘발성 런타임 상태. gossip/인메모리 (D12). DB엔 선택적 `users.last_seen_at`만.
- **Gateway 세션 / RESUME 재생 버퍼** — 인메모리(세션 소유 노드) (D24). DB 아님.
- **Rate limit 토큰 버킷** — per-node 인메모리 (D32). DB 아님.
- **Realm 액터 캐시** — 인메모리 (D35). DB는 원천만.
- → DB는 **영속 진실**만. 휘발 상태는 절대 안 섞는다 (안 그러면 또 꼬임).

### DB-D6. 메시지 멱등성
- `messages.nonce`(클라 제공) + `(channel_id, author_id, nonce)` 유니크로 재전송 dedup (D34).

## 4. 도메인 그룹 (스키마 구성)

[02-schema.md](02-schema.md)는 아래 순서로 정의:

1. **Identity & Auth** — users, refresh_tokens, mfa, webauthn(스트레치)
2. **Realms** — realms, guilds, dm_pairs
3. **Membership & Roles** — members, roles, member_roles
4. **Channels** — channels, channel_overwrites, threads
5. **Messages** — messages, attachments, reactions, mentions
6. **Social** — relationships, bans, invites
7. **Assets** — emojis, stickers
8. **Ops** — audit_log, webhooks, read_states

## 5. Phase 연계

- **Phase 0**: 마이그레이션 셋업, users/realms/guilds/channels/messages 등 코어 테이블.
- **Phase 3**: roles/permissions/overrides, relationships, invites, members 풍부화.
- **Phase 4**: 메시지 **시간 RANGE 파티셔닝**(04 문서), 검색(Postgres FTS), audit_log, webhooks.
