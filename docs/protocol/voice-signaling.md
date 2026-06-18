# Protocol — Voice 시그널링 (설계 전용, 미디어 제외)

> **D21 경계**: 음성/영상 **미디어**(WebRTC/SFU/코덱/UDP·SRTP)는 범위 밖. 이 문서는 **시그널링 경로**(Gateway를 통과하는 제어 평면)만 설계한다. 실제 오디오 전송은 구현하지 않는다.
> 관련 결정: **D47**(이 문서의 출처), D21(미디어 제외), D8(Realm 통일), D12(구독자표 팬아웃), D17(권한), D31(JSON 엣지), DB-D5(휘발 상태).
> 상태: **제어 평면 구현됨 (v1.48, fanout-only)** — gateway op 4 → 권한 CONNECT → 기존 emit 경로로 `VOICE_STATE_UPDATE` 팬아웃 + `VOICE_SERVER_UPDATE`(endpoint=null stub). 미디어 평면은 D21대로 영구 제외. **미구현 seam**(아래 §8 갱신): 액터 voice_states 맵(§1)·READY 스냅샷(§6)·서버 모더레이션(§5).

---

## 0. 왜 시그널링만

Discord 음성은 **두 평면**으로 나뉜다:

1. **시그널링(제어 평면)** — "누가 어느 음성 채널에 있나, mute/deaf 상태, 어느 미디어 서버에 붙어야 하나"를 Gateway WS로 주고받음. **JSON, 우리 아키텍처와 동형**(메시지 팬아웃과 같은 Realm 경로).
2. **미디어(데이터 평면)** — 실제 Opus 오디오 패킷을 별도 UDP voice 서버(SFU)와 WebRTC/SRTP로 교환. **별세계**(코덱·NAT 트래버설·지터버퍼·암호화) → D21로 **제외**.

이 프로젝트의 테마(분산 텍스트 인프라)는 (1)과 정확히 겹친다 — 음성 상태도 결국 **Realm 안의 실시간 상태**다. 그래서 (1)은 **기존 추상으로 흡수**(P4)해 설계하고, (2)는 경계로 남긴다.

---

## 1. 데이터 모델 (휘발, DB-D5)

음성 상태(voice state)는 **세션 수명에 묶인 휘발 상태**다 — presence(D42)·구독자표(D12)와 같은 부류. 따라서 **DB에 두지 않는다**(DB-D5). Discord도 voice state를 Gateway 메모리로 관리한다.

```
VoiceState {
    user_id     : u64
    realm_id    : u64        // 음성 채널이 속한 Realm(길드/그룹DM)
    channel_id  : u64        // 음성 채널(channel_kind='voice'). null = 채널 떠남
    session_id  : u64        // 이 음성 세션을 연 WS 세션
    self_mute   : bool       // 클라가 자기 마이크 끔
    self_deaf   : bool       // 클라가 자기 스피커 끔
    server_mute : bool       // 모더레이터가 강제 mute (MUTE_MEMBERS)
    server_deaf : bool       // 모더레이터가 강제 deaf (DEAFEN_MEMBERS)
}
```

- **소유 위치**: 음성 채널은 어느 Realm에 속하므로(D8), 그 **Realm 소유 노드의 Realm 액터**가 `{user_id → VoiceState}` 맵을 들고 있다 — 구독자표(D12)와 같은 자리·같은 라이프사이클. 전역 조회 0(D12 철학 재사용).
- **휘발성**: 노드 사망 → voice state 유실 = 캐시 미스. 클라가 RESUME 후 voice state를 재선언(op 4 재전송)으로 복구. 메시지처럼 Postgres 진실이 받쳐주지 않는다(음성은 영속 대상 아님).
- `channels.kind='voice'`는 이미 존재(domain `ChannelKind::Voice`). 권한 비트 `CONNECT(1<<20)`/`SPEAK(1<<21)`/`MUTE_MEMBERS`/`DEAFEN_MEMBERS`/`MOVE_MEMBERS`도 permissions.md에 **예약됨** — 이 설계가 그 비트의 소비처.

---

## 2. Gateway 시그널링 (JSON, D31)

### Opcodes (gateway.md §1 확장)

| op | 이름 | 방향 | 설명 |
|---|---|---|---|
| 4 | `VOICE_STATE_UPDATE` | C→S | 음성 채널 입장/이동/퇴장 + self mute/deaf. (Discord op 4와 동일) |

### DISPATCH 이벤트 (S→C)

| 이벤트 | 트리거 |
|---|---|
| `VOICE_STATE_UPDATE` | 누군가의 voice state 변경(입장/퇴장/mute) → 같은 Realm 구독자에 팬아웃 |
| `VOICE_SERVER_UPDATE` | 입장 확정 후 **붙을 미디어 서버 정보** 전달 (endpoint + token) — **미디어 서버가 없으므로 설계상 stub**(D21 경계) |

### C→S op 4 페이로드

```json
{ "op": 4, "d": {
    "realm_id": "...",
    "channel_id": "...",     // null이면 채널 떠남
    "self_mute": false,
    "self_deaf": false
} }
```

### S→C VOICE_STATE_UPDATE (팬아웃)

```json
{ "op": 0, "t": "VOICE_STATE_UPDATE", "d": {
    "realm_id": "...", "channel_id": "...", "user_id": "...",
    "self_mute": false, "self_deaf": false,
    "server_mute": false, "server_deaf": false
} }
```

### S→C VOICE_SERVER_UPDATE (미디어 경계 — stub)

```json
{ "op": 0, "t": "VOICE_SERVER_UPDATE", "d": {
    "realm_id": "...",
    "endpoint": null,        // 실제론 SFU "host:port" — 미디어 서버 없음(D21) → null
    "token": "..."           // 미디어 인증 토큰 — 설계만(PASETO 재사용 가능)
} }
```
> **D21 경계 명시**: `endpoint=null`은 "여기서 시그널링이 끝나고 미디어 평면이 시작되지만 그 평면은 구현하지 않는다"는 표식. 클라는 이 이벤트를 받아도 실제 오디오 연결을 맺지 않는다.

---

## 3. 라우팅 — 메시지 경로 재사용 (P4)

voice state 변경은 **메시지 전송과 동형**으로 흐른다 (D9 2단 라우팅 + D12 구독자표):

```
클라(노드 A) ──op 4──▶ A의 Gateway
   → 권한 검사: CONNECT(채널 컨텍스트, D17). self_mute는 무권한, server_mute는 MUTE_MEMBERS.
   → Realm 소유 노드 B = hash(realm_id)  (D9)
   → A가 raw TCP로 B에 위임 (신규 wire VOICE_STATE_SET, §4)
   → B의 Realm 액터가 voice state 맵 갱신(입장/이동/퇴장)
   → B가 그 Realm 구독자(D12)에게 VOICE_STATE_UPDATE 팬아웃 (REALM_FANOUT 범용 envelope D39 재사용!)
   → 각 세션 노드가 JSON으로 변환해 클라에 push
```

- **핵심**: 팬아웃은 **D39 범용 envelope `(t, payload)`를 그대로 탄다** — `t="VOICE_STATE_UPDATE"`, payload=위 JSON. 즉 **node/protocol에 팬아웃용 신규 코드 0**(메시지·멤버 이벤트와 같은 길). 새로 필요한 건 "voice state **set**을 소유 노드에 위임"하는 명령 하나뿐.
- **VOICE_SERVER_UPDATE**는 입장 op 4를 보낸 **그 세션에만** 회신(팬아웃 아님) — 미디어 endpoint는 본인만 받음.

---

## 4. Wire (node-wire.md §4, 향후 0x01xx 확장 — 설계만)

메시지 라우팅과 같은 `0x01xx` 대역. 구현 시 추가될 타입(예약):

```
0x0120 VOICE_STATE_SET   →소유노드   realm_id:u64, channel_id:Option<u64>, user_id:u64,
                                       session_node:u64, self_mute:bool, self_deaf:bool
   # 비소유 노드가 음성 상태 변경을 Realm 소유 노드에 위임. REALM_EMIT(0x0104)와 대칭.
   # 소유 노드가 voice state 맵 갱신 후 VOICE_STATE_UPDATE를 REALM_FANOUT(0x0103)로 팬아웃.
```
- 퇴장(`channel_id=None`)·이동(다른 voice 채널 id)도 같은 메시지로 표현(특수 케이스 흡수, P4).
- 팬아웃은 `REALM_FANOUT`(0x0103) 재사용 → **voice 전용 팬아웃 wire 불필요**.

---

## 5. 권한 (D17)

- **입장**(`channel_id` 설정): 그 음성 채널 컨텍스트에서 `CONNECT`. 없으면 거부(403/무시).
- **말하기**: `SPEAK` (미디어 평면이라 시그널링은 상태만 표기 — `server_mute`로 게이팅 표현).
- **강제 mute/deaf/이동**(타인 voice state 변경): `MUTE_MEMBERS`/`DEAFEN_MEMBERS`/`MOVE_MEMBERS`.
- self mute/deaf: 무권한(본인 상태).
- DM/그룹DM voice: 참가자면 통과(길드 권한 폴백, D8/permissions.md §5).

---

## 6. READY 스냅샷 (gateway.md §2 확장 — 설계만)

READY에 현재 음성 상태를 포함해 클라가 "지금 누가 어느 음성 채널에 있나"를 즉시 그림:

```json
"voice_states": [ { "realm_id":"...", "channel_id":"...", "user_id":"...", "self_mute":false, ... } ]
```
- 자동구독(D13) 시점에 그 Realm의 voice state 스냅샷을 같이 실어 보냄(presence·read_states와 같은 패턴).

---

## 7. 구현되지 않는 것 (D21 경계 = 이 문서의 핵심 명시)

- **미디어 전송 일체**: WebRTC/ICE/STUN/TURN, SFU(selective forwarding), Opus 인코딩, SRTP 암호화, UDP voice 소켓, RTP 지터버퍼.
- **VOICE_SERVER_UPDATE의 실제 endpoint**: 미디어 서버가 없으므로 `null` stub.
- **음성 활동(speaking) 표시**: 미디어 평면 신호(RTP)에서 파생 → 제외.
- 포트폴리오 서술: *"음성 시그널링(제어 평면)은 텍스트 메시징과 동형이라 같은 Realm 라우팅·구독자표·범용 팬아웃 envelope로 흡수된다. 미디어 평면은 의도적으로 경계 밖에 둔다(왜 안 넣었는지 안다)."*

---

## 8. 구현 체크리스트 (v1.48 진행 상태)

- [x] **2. gateway op 4 핸들러 → 권한(CONNECT) → 팬아웃** — `handle_voice_state`(session). `route_voice_state` 신규 명령 대신 **기존 `Router::route_emit` 재사용**(스펙 §3 "거의 코드 0" 실현 — 멤버 이벤트와 동형, 신규 와이어 0).
- [x] **3'. `REALM_FANOUT`(t="VOICE_STATE_UPDATE") 팬아웃** — `route_emit`이 로컬/원격(`REALM_EMIT`/`REALM_FANOUT`) 자동 처리. **`VoiceStateSet`(0x0120) 신규 와이어 불필요**(emit 경로가 흡수).
- [x] **4. VOICE_SERVER_UPDATE를 입장 세션에 회신** — endpoint=null stub(D21 경계). `Hub::dispatch_one`(seq 부여, RESUME-safe).
- [ ] **1/5/6 (seam)**: `node::realm` 액터 `voice_states` 맵(§1)·READY `voice_states` 스냅샷(§6)·서버 모더레이션 MUTE/DEAFEN/MOVE_MEMBERS(§5). 현재는 **fanout-only**(클라가 스트림으로 상태 재구성). 액터 상태화는 후속 증분(맵 + `VoiceSnapshot` 명령 + READY 주입).
