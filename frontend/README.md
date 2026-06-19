# Frontend — 분산 Discord 클론 웹 UI

React 19 + TypeScript + Vite (D30). **backend의 CLI를 검증된 레퍼런스로 삼아 그 REST/WS 계약을 미러링**한다.

## 규칙 (이 프로젝트 frontend 작업 약속)

1. **backend 소스는 수정하지 않는다** (완성됨). dev는 Vite proxy로 같은 backend를 문다.
2. backend 버그/갭 수정이 꼭 필요하면 **승인 후** 진행. (그렇게 추가된 것: `GET /users/@me/realms`, `GET /guilds/:id/channels` — v1.51, 웹 UI 디스커버리용.)
3. frontend는 **완전 독립체** — 언제든 폐기·재작성 가능. 자체 빌드/버전(`package.json`).
4. **CLI ↔ frontend 상호운용**이 가능해야 한다(같은 backend → 한쪽에서 보낸 게 다른 쪽에 실시간 도착).

## 스택

- **React 19 + Vite 8 + TS** · **Tailwind CSS v4**(`@tailwindcss/vite`, `src/index.css`의 `@theme`)
- **TanStack Query** — REST 읽기 캐시(D30) · **네이티브 WebSocket** — gateway 실시간(D30)
- **zustand** — 인증/세션/UI 선택 상태 · **@noble/hashes** — 가입 PoW(D18) sha256

## 구조 (개념=디렉터리, R6)

```
src/
  api/        backend 계약 미러 (CLI rest.rs/gateway_client.rs 대응)
    http.ts        fetch 래퍼(+Bearer, /api proxy 접두)
    types.ts       계약 타입 (id = Snowflake = string!)
    auth.ts guilds.ts messages.ts social.ts   엔드포인트별 호출
    pow.ts pow.worker.ts   가입 PoW 솔버(Web Worker, 서버 알고리즘 동일)
    queryKeys.ts   React Query 키 단일 출처
  gateway/    WS 실시간
    connection.ts        GatewayClient (HELLO/IDENTIFY/READY/HEARTBEAT/RESUME/재연결)
    RealtimeProvider.tsx  DISPATCH → React Query 캐시 + 세션 store 브리지
  store/      zustand (auth 영속 · session 실시간 · ui 선택)
  hooks/      queries.ts (useRealms/useChannels/useMessages/useMembers/useRelationships)
  ui/         Login · ServerRail · ChannelSidebar · DmSidebar · ChatArea · MemberList · Home …
  lib/        ids(Snowflake) · display
```

> **중요**: 모든 id는 backend가 **JSON 문자열**로 직렬화한다(JS 53비트 정수 절단 회피, rest.md §0). 프론트 전 계층에서 `Snowflake = string`으로만 다룬다.

## 실행

```bash
# 1) backend 서버 (단일노드) — 별도 터미널
cd ../backend/bins/server
DATABASE_URL='postgres://david:2147483647@%2Fvar%2Frun%2Fpostgresql:48853/discord_v1' \
  REST_ADDR=127.0.0.1:8080 cargo run

# 2) frontend dev 서버
cd frontend
npm install      # 최초 1회
npm run dev      # http://localhost:5173
```

Vite dev proxy(`vite.config.ts`)가 CORS 없이 backend로 전달한다:
- `/api/*`  → `http://127.0.0.1:8080/*`  (REST)
- `/gateway` → `ws://127.0.0.1:8080/gateway` (WebSocket)

다른 노드/포트로: `BACKEND_URL=http://127.0.0.1:9090 npm run dev`.

```bash
npm run build      # tsc -b && vite build (프로덕션 번들)
npm run typecheck  # 타입만 검사
```

## CLI ↔ frontend 상호운용 테스트 (규칙4)

같은 backend를 물고 양방향 실시간을 눈으로 확인:

1. 브라우저에서 가입/로그인 → 서버 만들기 → 채널 입장.
2. 터미널에서 CLI로 같은 채널에 전송 →
   ```bash
   cd ../backend/bins/cli
   cargo run -- --url http://127.0.0.1:8080 send --token <T> --channel <C> --content "hi from CLI"
   ```
   → **브라우저에 실시간으로 표시**된다.
3. 반대로 브라우저에서 메시지 전송 → CLI `listen --token <T>`에 도착.

## 현재 배선된 기능

- **인증**: 가입(PoW worker)·로그인(MFA 2단계)·로그아웃 · 설정에서 **TOTP MFA 활성화**(enable→verify) · 내 user id 표시(친구 추가 공유용)
- **서버/채널**: 서버 목록/생성/초대·합류 · 텍스트/음성 채널 목록·생성
- **메시지**: 히스토리+실시간(생성/편집/삭제) · **옵티미스틱 전송**(보내는 즉시 표시→WS/REST로 확정, v0.1.2 — WS 프레임 놓쳐도 안 사라짐) · **이전 더보기 페이지네이션** · 답장(전송+**인용 미리보기 표시**, v0.1.4 — 라이브 메시지) · `<@id>` 멘션
- **리액션**: 이모지 픽커 + 칩 토글 + **WS 라이브 카운트**(MESSAGE_REACTION_ADD/_REMOVE)
- **첨부**: 파일 업로드(사후 첨부) + 이미지 인라인/파일 다운로드(인증 blob). id 확정은 WS MESSAGE_CREATE(빠름) **+ REST 히스토리 백업**(v0.1.1) — WS 재연결로 프레임을 놓쳐도 업로드 보장
- **음성**: 음성 채널 입장/퇴장(op4 제어 평면) + 참가자·mute/deaf 표시 (미디어는 D21 제외)
- **검색**: 길드 전문검색(FTS) 모달 → 결과 클릭 시 채널 이동
- **스레드**: 채널별 스레드 목록/생성/보관 + 스레드 열기
- **소셜**: presence(op3 idle/dnd, 친구/멤버 점) · 멤버 목록+액션(DM/닉변경/추방) · 친구(요청/수락/차단/삭제) · DM·그룹DM 열기/대화 · **DM 목록·헤더에 상대 이름**(v0.1.3 — DM realm members 조회로 1:1 상대 해석, 그룹은 그룹명)
- **표시명 디렉터리**: WS 이벤트가 싣는 username을 모아 id→이름 해상도 향상
- **연결**: HEARTBEAT + RESUME 재연결(끊겨도 세션 복구)

## 알려진 seam (backend 계약 한계 — 의도적)

- **username 조회 API 부재** → 타인 표시명은 nick·학습된 username·짧은 id 순. 친구 추가는 **user id**로(설정에서 내 id 확인).
- 리액션 카운트는 **세션 중 라이브 집계**(접속 전 과거 리액션은 미반영 — 집계 엔드포인트 없음).
- 메시지 페이지네이션은 50개씩 "더 보기"(자동 무한스크롤·읽음 위치 점프 후속).
- **미배선(엔드포인트는 존재)**: 웹훅·감사로그·역할 생성/부여·채널 권한 오버라이드 = 관리자 패널 후속.
