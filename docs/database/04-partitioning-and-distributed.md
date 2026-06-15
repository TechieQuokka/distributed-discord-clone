# Database — 파티셔닝 & 분산 연계

> 메시지 규모 확장과, DB가 분산 런타임(노드/Realm/복구)과 어떻게 맞물리나.
> 관련 결정: D11(Snowflake), D23(복구), D28(sqlx/파티셔닝), D29(worker-id), D32~D35.

---

## 1. Snowflake ID & 분산 생성

- 모든 PK = 64bit Snowflake: `[타임스탬프 41b | worker-id 10b | 시퀀스 12b]`.
- **중앙 시퀀스 없음.** 각 노드가 클러스터 config의 **고유 worker-id**(D29)로 생성 → DB 라운드트립 0, 충돌 0.
- 시간순 정렬 가능 → `id DESC` 인덱스가 곧 시간 정렬. `created_at`은 편의 중복 컬럼.
- ⚠ **클럭 스큐**: 노드 간 시계가 틀어지면 ID의 전역 시간순이 약간 어긋남. Realm 내 순서는 액터가 보장(D24)하므로 무해. NTP 권장 수준.

## 2. 메시지 파티셔닝 (구현됨 Phase 4, v1.38, 마이그레이션 V17)

메시지는 압도적 다수 행 → 유일하게 파티셔닝이 필요한 테이블. **V17에서 `PARTITION BY RANGE (id)` 전환**(드롭&재생성, 로컬 데이터 폐기). 월별 파티션(`messages_2026_06`/`_07`) + `messages_default` 캐치올, 인덱스는 부모 정의→상속.

> **nonce 멱등(D34) 영향**: 파티션 테이블 유니크 인덱스는 파티션 키(id) 포함 강제 → `uq(channel,author,nonce)` 불가 → nonce dedup을 **앱레벨**(create_message 가드 INSERT, dispatch 단일 직렬 소비자라 레이스 없음)로 이전. decisions D34 참조.
> **첨부 FK**: PG 12+는 파티션 부모 참조 FK + CASCADE 지원 → 아래 (a) 앱레벨 완화는 **불필요**(`attachments.message_id → messages(id)` CASCADE 유지). reactions/message_mentions는 원래 FK 없음.

### 전략: Snowflake 시간 기준 RANGE 파티셔닝 (월별)
```sql
-- Phase 4 전환. id의 상위 비트(타임스탬프)로 범위 분할
CREATE TABLE messages (...) PARTITION BY RANGE (id);

CREATE TABLE messages_2026_06 PARTITION OF messages
    FOR VALUES FROM (<snowflake_min_2026_06>) TO (<snowflake_min_2026_07>);
-- ... 월별 파티션, 신규 월은 사전 생성(스케줄)
```
- **왜 RANGE(시간)**: 채팅은 "최근=핫, 과거=콜드". 최근 파티션만 자주 접근 → 캐시·인덱스 효율 ↑, 오래된 파티션은 분리/아카이브 용이.
- **왜 hash(realm_id) 아님**: 히스토리 조회가 "최근 N개"라 시간 지역성이 핵심. realm 해시는 라우팅(런타임)에서 이미 처리.
- Phase 1~3은 단순 단일 테이블 + `(channel_id, id DESC)` 인덱스. **조기 파티셔닝 금지**(복잡도만 증가).

### 파티션 테이블 FK 주의
- Postgres에서 **파티션 테이블을 참조하는 FK**(`attachments.message_id → messages.id`)는 제약이 있음.
- 결론: 파티셔닝 전환 시 `attachments`/`reactions`/`message_mentions`의 메시지 FK는
  - (a) **앱 레벨 무결성**으로 대체(권장, 단순), 또는
  - (b) 동일 파티션 키(`realm_id` 또는 시간)를 함께 들고 가 합성 FK 구성.
- Phase 1~3엔 정식 FK 유지 → Phase 4 전환 시 (a)로 완화.

## 3. DB ↔ 분산 런타임 연계

### 진실/캐시 경계 (D23/D35)
```
쓰기:  클라 → Realm 액터 → [Postgres 영속(persist)] → 팬아웃 (persist-then-fanout, D24)
읽기:  최근 → Realm 액터 인메모리 캐시 (D35)
       과거 → Postgres 직격 (커서 페이지네이션, D38)
```
- DB는 **영속 진실만**. presence/session/ratelimit/액터캐시는 DB 밖 (DB-D5).

### 노드 사망 → 복구 (D23)
```
노드 죽음 → consistent hashing이 해당 Realm 재배치
         → 새 소유 노드가 Postgres에서 Realm 상태 rehydrate
           (guild/channels/roles/members/최근 messages 로드)
         → 죽은 노드의 WS 세션은 끊김 → 클라가 다른 노드로 RESUME 재연결
```
- 즉 **인메모리 상태 유실은 곧 캐시 미스**일 뿐, 데이터 손실 아님. Postgres가 받쳐줌.

### Realm 소유와 DB 접근
- 한 Realm은 한 노드가 소유(액터 단일소유, D7/D9) → 그 Realm의 쓰기는 **한 노드에서 직렬화** → DB 경합/락 거의 없음.
- 읽기(과거 히스토리)는 어느 노드서든 가능(무상태 조회). 동일 Realm 쓰기는 소유 노드 경유라 일관.

## 4. 인덱싱 요약

| 테이블 | 주 인덱스 | 목적 |
|---|---|---|
| messages | `(channel_id, id DESC)` | 히스토리 페이지네이션 |
| messages | `uq(channel_id, author_id, nonce)` | 멱등성 dedup (D34) |
| members | PK`(realm_id,user_id)` + `(user_id)` | 멤버 조회 / "내 서버 목록" |
| channels | `(realm_id) WHERE not deleted` | Realm 채널 목록 |
| relationships | PK`(user_id,target_id)` | 친구/차단 조회 |
| read_states | PK`(user_id,channel_id)` | 미읽음 카운트 |
| refresh_tokens | `uq(token_hash)`, `(user_id) WHERE active` | 인증/폐기 |
| audit_log | `(realm_id, id DESC)` | 감사 로그 시간순 |

## 5. 검색 (Q10) — 구현됨 (Phase 4, v1.33, 마이그레이션 V12)
- **Postgres 전문검색(FTS)**: `tsvector` 생성 컬럼 + GIN 인덱스. 외부 검색엔진(Elasticsearch) 회피 — 로컬 study 범위에 적합.
```sql
-- 0012_message_fts.sql (적용됨)
ALTER TABLE messages ADD COLUMN content_tsv tsvector
    GENERATED ALWAYS AS (to_tsvector('simple', content)) STORED;
CREATE INDEX ix_messages_fts ON messages USING GIN (content_tsv);
```
- 쿼리: `content_tsv @@ websearch_to_tsquery('simple', $q)` (사용자 입력 안전 파싱). REST `GET /guilds/:id/messages/search`(멤버 + VIEW_CHANNEL 채널 한정, D17).
- 파티셔닝(D28) 전환 시 각 파티션에 GIN 인덱스가 상속됨 — 파티션 재생성 마이그레이션이 `content_tsv` 생성 컬럼 + GIN을 함께 정의한다.

## 6. 마이그레이션 운영
- `sqlx migrate` — 순번 SQL 파일. Phase 0부터 버전 관리.
- 파티션 신규 월 생성은 별도 유지 작업(런처/스케줄). 로컬 study에선 수동/스크립트로 충분.
