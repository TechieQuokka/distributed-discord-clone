# 📚 Documentation Index

Rust 기반 분산 Discord 클론 — 설계 문서 모음.
공부 + 포트폴리오, 로컬 전용, **"공부는 실전처럼"** (수만 동접 감당 구조).

---

## 🧭 어디부터 읽나

| 순서 | 문서 | 내용 |
|---|---|---|
| 1 | [architecture/decisions.md](architecture/decisions.md) | **결정 원장 (D1~D43)** — 무엇을 왜 정했나. 모든 설계의 출처(source of truth) |
| - | [design-discussion.md](design-discussion.md) | **설계 토론 기록** — 어떤 선택지를 두고 어떻게 논쟁해 정했나 (서사) |
| 2 | [database/01-overview.md](database/01-overview.md) | DB 철학·규약·핵심 모델링 결정 |
| 3 | [database/02-schema.md](database/02-schema.md) | 전체 테이블 DDL (도메인별) |
| 4 | [database/03-erd.md](database/03-erd.md) | ER 다이어그램 |
| 5 | [database/04-partitioning-and-distributed.md](database/04-partitioning-and-distributed.md) | 파티셔닝·인덱싱·분산 연계 |

---

## 🗂️ 카테고리

### Architecture (시스템 아키텍처)
- [decisions.md](architecture/decisions.md) — 결정 원장 + 6단계 로드맵 + 열린 질문
- [architecture/permissions.md](architecture/permissions.md) — 권한 비트마스크 레이아웃 & 계산
- [../design-discussion.md](design-discussion.md) — 설계 토론 기록(논쟁 서사)

### Project (작업 추적 / 규칙)
- [../TODO.md](../TODO.md) — Phase별 할 일
- [../CLAUDE.md](../CLAUDE.md) — 개발 규칙(최신 lib, 문서 동기화, 버전 관리) + 핵심 원칙
- [../CHANGELOG.md](../CHANGELOG.md) — 버전 이력 (1.0.0~)

### Database (데이터 설계)
- [01-overview.md](database/01-overview.md) — 철학·규약·모델링 결정
- [02-schema.md](database/02-schema.md) — 스키마 DDL
- [03-erd.md](database/03-erd.md) — 엔티티 관계도
- [04-partitioning-and-distributed.md](database/04-partitioning-and-distributed.md) — 확장·파티셔닝

### API (인터페이스)
- [api/rest.md](api/rest.md) — REST 엔드포인트 카탈로그
- [api/gateway.md](api/gateway.md) — Gateway(WebSocket) 이벤트/명령 카탈로그

### Protocol (노드 간 와이어)
- [protocol/node-wire.md](protocol/node-wire.md) — 수제 바이트 프로토콜 (프레이밍·헤더·메시지·핸드셰이크)

---

## 📌 핵심 요약 (TL;DR)

```
클라(web/CLI) ──JSON/WSS──▶ Gateway노드 ──수제바이트/mTLS──▶ Gateway노드
                              │  (풀메시, consistent hashing)
                              ▼
                        Realm 액터 (수제 tokio+mpsc, 단일소유 → 순서보장)
                         · persist-then-fanout · 최근메시지 캐시 · 구독자표
                              ▼
                         PostgreSQL (진실의 원천, sqlx)

경계: 헥사고날 9-crate (domain 중심) · 검증: DST + tracing
보안: PASETO+refresh / Argon2id / mTLS / TOTP / PoW
비범위(명시): 합의(Raft) 없음, Voice 미디어 없음
```

> 현재 단계: **Phase 3 완료 + Phase 4 진행** — Phase 3(…·크로스노드 유저 라우팅 D43) 완료, **Phase 4 인증/봇방지 묶음: PoW(D18)·rate limit(D32)·TOTP MFA(D19)** 완료. 다음: 스레드/검색·파일첨부·파티셔닝(D28) 등. (이어서 → [RESUME.md](../RESUME.md))
