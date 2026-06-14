# TODO

> 분산 Discord 클론 — 작업 추적. 설계 출처: [docs/architecture/decisions.md](docs/architecture/decisions.md).
> 현재 단계: **DB 설계 + 문서화** (코딩 전). 로드맵 = D §4-R.

범례: `[ ]` 미착수 · `[~]` 진행중 · `[x]` 완료

> ⚠️ **이 문서는 체크용입니다.** 항목 내용을 임의로 확장·구체화하지 마세요.
> 수정/추가/삭제가 필요하면 **사용자 승인 후** 변경합니다. (CLAUDE.md R5)
> 작업 순서: **backend+API → CLI → web UI(후순위)** (CLAUDE.md R4)

---

## 📄 문서화 (Docs) — 진행중

- [x] 아키텍처 결정 원장 (decisions.md, D1~D38)
- [x] DB 설계 문서군 (overview / schema / erd / partitioning)
- [x] 설계 토론 기록 (design-discussion.md)
- [x] 문서 인덱스 (docs/README.md)
- [x] **API 설계 문서** — REST 엔드포인트 카탈로그 (`docs/api/rest.md`)
- [x] **Gateway 이벤트/명령 카탈로그** (`docs/api/gateway.md`)
- [x] **노드 간 프로토콜 명세** — 수제 바이트 레이아웃 (`docs/protocol/node-wire.md`)
- [x] 권한 비트 레이아웃 정의 (`docs/architecture/permissions.md`)
- [x] DB 스키마 리뷰/보강 (빠진 테이블 점검)

---

## Phase 0 — 뼈대 (인프라 stub)

- [x] 독립 crate 골격 생성 (D22, R7 — umbrella 워크스페이스 없음)
- [x] crate 의존성 규칙 검증 (domain이 IO crate import 불가)
- [~] `domain` — 코어 엔티티 + 리포지토리 trait(port)
- [x] `actor-rt` — 수제 액터 런타임 (mailbox/spawn 루프)
- [x] `transport` — `trait NodeTransport` + **in-process stub** 구현
- [x] `protocol` — 프레임 헤더(version 바이트, D36) + 수제 인코딩 스켈레톤
- [~] Snowflake 생성기 (worker-id from config, D11/D29)
- [x] `storage` — Postgres 연결 + `sqlx` 셋업
- [x] DB 마이그레이션 V1 (코어 테이블: users/realms/guilds/channels/messages)  # 적용 완료 + 라운드트립 테스트 통과
- [x] `tracing` 셋업 + span 기본 (D26)
- [x] 클러스터 config 포맷 (정적 노드목록, worker-id, D5/D29)  # cluster-config crate

---

## Phase 1 — MVP 메시징 (단일노드)

- [x] 가입/로그인 — Argon2id(D15) + PASETO access + refresh(D14)  # auth crate 프리미티브 완료, REST/storage 흐름 배선 TODO
- [ ] PoW 가입 챌린지 기초 (D18) *(또는 Phase 4로)*
- [x] WS Gateway — 연결 수명주기(IDENTIFY→READY→HEARTBEAT)  # RESUME 재생버퍼는 Phase 2
- [x] 길드 생성 / 텍스트 채널 생성
- [x] 메시지 송수신 (persist-then-fanout, 단일노드, D24)
- [x] 메시지 멱등성 nonce (D34)
- [x] CLI 클라이언트 — 대화형 + **헤드리스 시나리오 모드** (D1)
- [x] 메시지 히스토리 페이지네이션 (Snowflake 커서, D38)

---

## Phase 2 — 분산 활성화

- [x] raw TCP + mTLS 전송 구현체 (rustls, D3/D16) → stub 교체  # TcpTransport: 2노드 mTLS 교환 + 크로스노드 팬아웃 라이브 검증
- [x] 풀메시 노드 연결 (정적 config, D4/D5)  # listen/dial(작은→큰 id) + 재연결 + 크로스노드 inbound 루프
- [x] Consistent Hashing 링 (D6) — vnode 포함
- [x] Realm 2단 라우팅 (세션 소유 vs Realm 소유, D9)  # router: 소유판정+로컬/원격 subscribe·send + 크로스노드 fanout
- [x] Realm-local 구독자 추적 + 팬아웃 (D12)  # 구독자표 + RealmFanout 와이어 크로스노드 배달 (in-process stub 상에서)
- [x] Gateway RESUME — per-session seq + 재생 버퍼 (D24)  # Hub 영속 세션상태(seq+bounded버퍼+CSPRNG resume_token), detach/grace/replay/RESUMED, gap→INVALID
- [x] Realm 상태 rehydrate (노드 재배치, D23)  # PING/PONG 생사판정(Membership)+owner_excluding failover; 새 소유 노드 actor fresh-spawn(Postgres 진실 보존). D35 캐시 warmup은 후속
- [x] Backpressure — bounded 채널 + 느린 클라 끊기 (D27)  # Hub::push_live가 채널 가득 시 live drop→pump 종료·소켓 close; 버퍼 남아 RESUME 복구. 노드간/액터 메일박스 bounded
- [x] **DST 하네스** — SimTransport + SimClock (D25)  # transport::sim(가상시간+시드 카오스: 지연/유실/파티션); Router/RealmActor에 Clock 주입→id 결정론; node/tests/dst.rs(재현성+파티션). 액터 가상실행기는 후속

---

## Phase 3 — Discord 본체

- [x] 역할/권한 비트마스크 + 계산순서 (D17)  # roles/member_roles(V5)+@everyone 기본, RoleRepository, REST 역할관리, 강제(SEND_MESSAGES/MANAGE_CHANNELS/CREATE_INVITE/MANAGE_ROLES) 2유저 라이브
- [x] 채널 권한 오버라이드  # channel_overwrites(V6)+ChannelOverwriteRepository, effective_channel_permissions 배선, REST PUT /channels/:id/permissions/:tid, gateway can_send 채널컨텍스트화. 2유저 라이브(deny@everyone→member allow)
- [ ] DM / 그룹DM (Realm + dm_pairs, DB-D2)
- [ ] 멤버 관리 (nick/joined/roles)
- [x] 초대 (invites)  # domain Invite/port + storage(트랜잭션 redeem)+V4 마이그레이션 + REST(생성/redeem) + CLI(create-invite/join). 2유저 라이브 검증(초대→합류→자동구독→크로스유저 팬아웃)
- [ ] 리액션 / 편집·삭제(소프트) / 멘션 / 답장
- [ ] 친구·차단 (relationships)
- [ ] 읽음 상태 / 미읽음 카운트 (read_states)
- [ ] 전역 presence (gossip, Q2 후속/Q11)

---

## Phase 4 — 살붙이기

- [ ] 스레드 / 포럼 채널
- [ ] 웹훅
- [ ] 감사 로그 (audit_log)
- [ ] 검색 — Postgres FTS (Q10)
- [ ] 파일 첨부 — 로컬 FS (D37)
- [ ] TOTP MFA (D19)
- [ ] PoW 봇방지 정식 (D18)
- [ ] Rate limit — Token Bucket per-node (D32)
- [ ] **메시지 시간 RANGE 파티셔닝** 전환 (D28, 04 문서)

---

## Phase 5 — 스트레치

- [ ] WebAuthn/Passkeys (D19)
- [ ] CRDT 오프라인 동기화
- [ ] gossip discovery (SWIM, Q11)
- [ ] 이벤트 소싱 (선택, D23)
- [ ] Voice 시그널링 (미디어 제외, D21)
- [ ] MinIO 첨부 저장소 업그레이드 (D37)

---

## ❓ 미결 디테일 (Phase 진입 시 결정)

- [ ] Q7. 액터 supervisor/재시작 전략 (Phase 2)
- [x] Q9. CLI 시나리오 스크립트 포맷 (Phase 1) — `scenario` 서브커맨드로 해결
- [ ] Q10. 검색 구현 세부 (Phase 4, Postgres FTS 유력)
- [ ] Q11. gossip discovery + 전역 presence (Phase 3/5)
