# CLAUDE.md

Claude Code 작업 지침. 분산 Discord 클론 (Rust backend + web/TS frontend).
공부 + 포트폴리오, 로컬 전용, 철학 **"공부는 실전처럼"**(수만 동접 감당 구조).

> **이어서 작업 시작 → [RESUME.md](RESUME.md)** (다음 세션 온보딩: 문서→코드→재개).
> **먼저 읽을 것**: [docs/README.md](docs/README.md) → [docs/architecture/decisions.md](docs/architecture/decisions.md)(결정 원장, source of truth).

---

## 🔧 개발 규칙 (Development Rules)

### R1. 최신 라이브러리 우선
- 항상 **가능한 최신 안정(stable) 버전**의 crate를 사용한다.
- Rust **edition = 2024** (안정화됨, Rust 1.85+).
- 버전을 직접 추측해 박지 말고 **최신을 탐색해서** 넣을 것:
  - `cargo add <crate>` — crates.io의 최신 호환 버전 자동 추가 (선호).
  - `cargo search <crate>` / crates.io / docs.rs 확인.
  - 의심되면 인터넷에서 최신 릴리스 확인.
- 새 의존성 추가 시 Cargo.toml에 명시적으로 핀(pin)하고, 최신인지 확인.

### R2. 문서를 코드와 함께 업데이트 (중요)
- 개발 도중 **설계를 부분 수정**해야 하는 상황이 생기면, **반드시 관련 문서를 같은 변경에서 업데이트**한다.
- 코드만 바꾸고 문서를 안 고치는 것을 금지 — 문서와 코드가 어긋나면 "코드 꼬임"(지난 실패 원인)이 다시 시작된다.
- 어디를 고치나:
  - 결정이 바뀜 → `docs/architecture/decisions.md`(D번호 갱신/추가) + 필요 시 `docs/design-discussion.md`에 변경 이유 기록.
  - DB 스키마 변경 → `docs/database/*` 갱신 + 마이그레이션.
  - 새 작업/완료 → `TODO.md` 체크.
  - 버전 변동 → `CHANGELOG.md` 기록 (R3).
- 설계와 다르게 구현할 수밖에 없는 경우: **임의로 진행하지 말고, 먼저 문서를 고쳐 합의 상태로 만든 뒤** 구현.

### R3. 버전 관리 (Versioning)
- **1.0.0** 에서 시작.
- **수정/변경(fix)** → PATCH **+0.0.1** (예: 1.0.0 → 1.0.1)
- **새 기능 추가(feature)** → MINOR **+0.1.0** (예: 1.0.1 → 1.1.0)
- (대규모/호환 깨짐은 MAJOR +1.0.0 — 필요 시 사용자 판단)
- 모든 변동은 **`CHANGELOG.md`** 에 한 줄 기록 (버전 + 무엇을).
- 워크스페이스 Cargo.toml의 `version` 도 동기화.

> 참고: 표준 SemVer는 minor를 올릴 때 patch를 0으로 리셋하지만, 이 프로젝트는 사용자 규칙대로 **단순 가산**(예: 1.0.1 + feature = 1.1.1)도 허용. 충돌 시 사용자 규칙 우선.

### R4. 작업 순서 (Workflow Order)
- **순서: ① backend server + API → ② CLI → ③ web UI.**
- **web UI는 후순위.** ①②(backend·API·CLI)를 모두 끝낸 뒤에 web UI 작업에 착수한다.
- 각 단계 착수 전, **필요하면 인터넷에서 최신 스택/라이브러리 정보를 조사**한 뒤 진행 (R1과 연계).

### R5. TODO 규율
- 작업을 진행하면서 **`TODO.md`의 해당 항목을 체크**(`[ ]`→`[~]`→`[x]`)하며 진행한다.
- **`TODO.md`는 "체크용"이다.** 항목 내용을 임의로 **확장·구체화하지 않는다.**
- TODO를 **수정/추가/삭제해야 한다면, 먼저 사용자 승인을 받고** 수정한다.

### R6. 개념 단위 디렉터리/모듈 분리
- 가능한 한 **개념 = 디렉터리(또는 모듈)** 로 잘게 나눈다. 거대한 단일 파일 금지.
- crate 내부도 개념별 모듈로 분리 (예: `domain/src/id/`, `permissions/`, `error/`).

### R7. 독립 빌드·독립 관리 (umbrella 워크스페이스 없음)
- **단일 Cargo 워크스페이스로 한 번에 빌드하지 않는다.** 각 crate = 독립 패키지(자체 `Cargo.toml`·`target`·`Cargo.lock`·버전 1.0.0~).
- 각 crate를 **개별 빌드/테스트/버전 관리** (`cd backend/crates/<x> && cargo build`).
- crate 간 결합은 **path 의존으로만**. 최상위는 `backend/`(rust) · `frontend/`(web) 분리.

---

## 📁 최상위 구조

```
backend/   crates/<독립 crate들> · bins/{server,cli}   # rust, 독립 빌드 (R7)
frontend/  web UI (React+TS, 후순위 R4)
docs/      설계 문서 (프로젝트 공통)
TODO.md · CLAUDE.md · CHANGELOG.md
```

## 🧱 핵심 원칙 (코드 꼬임 방지 — 절대 위반 금지)

지난 유사 프로젝트는 **코드 내부가 꼬여서 실패**했다. 아래는 그 백신이다 (decisions.md §1).

- **P1.** 청사진 우선 — 코드 전에 문서.
- **P2.** 경계는 `trait` 뒤에 — crate로 쪼개 컴파일러가 경계 강제. `domain`은 IO를 모른다.
- **P3.** 어려운 구현은 stub부터 — 기능 먼저, 실제 구현체는 나중에 교체.
- **P4.** 특수 케이스는 추상화로 흡수 — 예: 길드/DM/그룹DM = `Realm` 하나.
- **P5.** 상태는 액터 안에 격리 — 공유 가변상태+락 금지.
- **P6.** 암호화는 직접 짜지 않는다 — crypto는 검증된 크레이트. "수제"는 전송/액터 런타임에만.
- **DB-D5.** 휘발 상태(presence/세션/ratelimit/캐시)는 DB에 두지 않는다.

---

## 🗺️ 빠른 참조

| 알고 싶은 것 | 문서 |
|---|---|
| 무엇을 왜 정했나 | `docs/architecture/decisions.md` |
| 어떻게 논쟁해 정했나 | `docs/design-discussion.md` |
| DB 설계 | `docs/database/` |
| 할 일 / 진행도 | `TODO.md` |
| 버전 이력 | `CHANGELOG.md` |

## 🛠️ 확정 스택 (요약)
- Backend: Rust(edition 2024, tokio), 헥사고날 **독립 crate**(umbrella 워크스페이스 없음, R7)
- DB: PostgreSQL + `sqlx`(컴파일타임 쿼리 검증)
- 노드↔노드: raw TCP + mTLS(rustls) + 수제 바이트 프로토콜, 풀메시, consistent hashing
- 클라↔Gateway: WebSocket + JSON
- 보안: PASETO(Ed25519) + refresh 회전, Argon2id, TOTP, PoW
- 관찰성/테스트: `tracing`, DST(결정론적 시뮬레이션)
- Frontend: React + TS + Vite
- 비범위(명시): 합의(Raft) 없음, Voice 미디어 없음
