# Protocol — 노드 간 와이어 프로토콜 (수제 바이트)

> Gateway 노드 ↔ Gateway 노드. **raw TCP + mTLS + 수제 바이트 직렬화 + 길이접두사 프레이밍**.
> 관련 결정: D3(raw TCP/수제), D4(풀메시), D5(정적config), D16(mTLS), D26(trace), D31(노드는 바이트), D36(버전).
> 크레이트: 타입/인코딩은 `protocol`, 전송은 `transport`(stub/raw-TCP 구현) (D22).

---

## 0. 설계 원칙

- **모든 정수 = 빅엔디언(network order), 패킹(정렬 패딩 없음).**
- **수제 인코딩**: serde/bincode/protobuf 미사용. `protocol` crate가 `encode()/decode()`를 직접 구현.
- **stub과 wire의 분리** (P2/P3): in-process stub 전송은 Rust 구조체를 *그대로* 전달(직렬화 안 함). raw-TCP 전송만 이 바이트 포맷을 사용. → 둘 다 동일한 `enum NodeMessage`를 주고받음.
- 전송 자체는 **mTLS(rustls)** 위에서 (D16). 즉 이 바이트 스트림은 이미 인증·암호화된 채널 안에 있음 → 프레임별 HMAC 불필요.

---

## 1. 프레이밍 (길이 접두사)

TCP는 바이트 스트림이라 메시지 경계가 없음 → 4바이트 길이 접두사로 구분.

```
+------------------+----------------------------+
| length : u32 BE  |  payload (length 바이트)   |
+------------------+----------------------------+
```
- `length` = 뒤따르는 payload 바이트 수 (length 필드 자신은 제외).
- **최대 프레임 = 16 MiB** (안전/백프레셔 상한, D27). 초과 시 연결 종료.
- 수신측: 4바이트 읽어 length 확보 → length만큼 더 읽어 payload 완성 → 디코드.

---

## 2. Payload = 공통 헤더 + 바디

```
offset  size  field
------  ----  ---------------------------------------------
0       1     version      u8     # 프로토콜 버전 (D36, 현재 = 1)
1       2     msg_type     u16    # 메시지 종류 (§4)
3       1     flags        u8     # 비트플래그 (§3)
4       16    trace_id     u128   # 분산 트레이싱 전파 (D26)
20      8     src_node_id  u64    # 보낸 노드 id
28      ...   body                # msg_type별 가변 (§5)
```
- 고정 헤더 = **28바이트**. 이후 바디.
- 알 수 없는 `version` → 연결 거부/종료 (핸드셰이크에서 사전 합의, §6).
- 알 수 없는 `msg_type` → 해당 프레임 무시 + 경고 로그(전방호환), 단 치명적이면 종료.

### flags (u8)
| 비트 | 의미 |
|---|---|
| 0x01 | `COMPRESSED` — 바디가 압축됨 (후속, 기본 0) |
| 0x02 | `REQUIRES_ACK` — 수신측 ACK 기대 |
| 나머지 | 예약 |

---

## 3. 원시 타입 인코딩 규칙

`protocol` crate의 인코더가 따르는 규칙 (수제):

| 타입 | 인코딩 |
|---|---|
| `u8/u16/u32/u64` | 빅엔디언 고정폭 |
| `i64` | 빅엔디언 2의 보수 |
| `bool` | 1바이트 (0/1) |
| `Snowflake` | `u64` |
| `String` | `[len: u32][utf8 bytes]` |
| `Bytes` | `[len: u32][raw]` |
| `Option<T>` | `[present: u8][T if present==1]` |
| `Vec<T>` | `[count: u32][T × count]` |
| `enum` | `[tag: u16][variant fields]` |
| `Timestamp` | `i64` (epoch millis) 또는 Snowflake에서 추출 |

---

## 4. 메시지 종류 (msg_type)

| 범위 | 카테고리 |
|---|---|
| `0x00xx` | 제어/핸드셰이크 |
| `0x01xx` | Realm 라우팅/팬아웃 |
| `0x02xx` | Presence/gossip |
| `0x03xx` | 클러스터/링 |

| msg_type | 이름 | 방향 | 설명 |
|---|---|---|---|
| 0x0001 | `HELLO` | 양방향 | 연결 직후 노드 식별 (mTLS 핸드셰이크 후) |
| 0x0002 | `HELLO_ACK` | 양방향 | HELLO 응답 |
| 0x0003 | `PING` | 양방향 | 생존 확인 (failure detection) |
| 0x0004 | `PONG` | 양방향 | PING 응답 |
| 0x0101 | `REALM_COMMAND` | →소유노드 | 클라 동작을 Realm 소유 노드로 전달 (메시지 전송 등) |
| 0x0102 | `REALM_COMMAND_RESULT` | →출발노드 | 명령 결과/ACK 회신 |
| 0x0103 | `REALM_FANOUT` | 소유노드→ | 구독자 보유 노드로 이벤트 푸시 지시 (D12) |
| 0x0104 | `REALM_EMIT` | →소유노드 | 비-메시지 이벤트(멤버 변동 등) 팬아웃을 소유 노드에 위임 (D39) |
| 0x0110 | `SUBSCRIBE` | →소유노드 | 유저 U(노드 N)가 Realm R 구독 등록 (D12) |
| 0x0111 | `UNSUBSCRIBE` | →소유노드 | 구독 해제 |
| 0x0201 | `PRESENCE_GOSSIP` | 양방향 | presence 델타 전파 (Phase 3, D12) |
| 0x0301 | `RING_UPDATE` | 양방향 | 해시링 멤버십 변경 (Phase 5 동적, gossip) |

---

## 5. 주요 바디 레이아웃

### `SUBSCRIBE` (0x0110)
```
realm_id : u64
user_id  : u64
node_id  : u64     # 이 유저의 세션이 붙은 노드 (= src_node_id와 보통 동일)
```

### `REALM_COMMAND` (0x0101)
```
realm_id          : u64
origin_node_id    : u64
origin_session_id : u64       # 결과 회신 라우팅용
command           : enum(u16) # 액터 명령
   0x01 SendMessage { channel_id:u64, author_id:u64, nonce:Option<String>,
                      content:String, reference_message_id:Option<u64> }
   0x02 EditMessage { channel_id:u64, message_id:u64, content:String }
   0x03 DeleteMessage { channel_id:u64, message_id:u64 }
   0x04 AddReaction { channel_id:u64, message_id:u64, emoji:String }
   ...
```
- 소유 노드의 Realm 액터가 검증·persist(Postgres) 후 `REALM_FANOUT`을 발사 (persist-then-fanout, D24).

### `REALM_FANOUT` (0x0103)
```
realm_id      : u64
event_type    : u16            # gateway.md의 DISPATCH 이벤트에 대응
event_body    : Bytes          # 직렬화된 이벤트 내용 (seq/JSON 미포함)
target_users  : Vec<u64>       # 이 노드에서 푸시할 대상 유저 id
```
- **seq와 JSON 인코딩은 세션 노드가 담당**: 수신 노드가 각 세션에 per-session `s`(D24)를 부여하고 JSON(D31)으로 변환해 클라에 push. → seq 소유권이 세션 노드에 유지됨.

> **구현 현황 (Phase 3, D39 — 범용 envelope).** 위 `REALM_COMMAND`/`REALM_FANOUT`은 edit/delete/react까지 포함한 목표 설계다. 현재 `protocol` crate는 다음을 구현한다 — 메시지 전송은 전용 명령으로, 그 외 이벤트는 **범용 `(t, payload)` envelope**(D39)로:
> - `REALM_SEND`(0x0101): `realm_id:u64, channel_id:u64, author:u64, content:String, nonce:Option<String>` — `SendMessage` 단일 액션(소유 노드에서 persist+ID 확정).
> - `REALM_EMIT`(0x0104): `realm_id:u64, t:String, payload:String` — 비-메시지 이벤트(멤버 변동 등) 팬아웃을 소유 노드에 위임. `payload`=클라에 나갈 JSON을 미리 직렬화한 불투명 문자열(하위 계층은 파싱 안 함, D39/P2).
> - `REALM_FANOUT`(0x0103): `realm_id:u64, t:String, payload:String, user_ids:Vec<u64>` — **모든 DISPATCH 이벤트 공용**. `t`=이벤트 이름(`MESSAGE_CREATE`/`GUILD_MEMBER_ADD` …), `payload`=직렬화된 JSON. (이전의 메시지 전용 평탄 필드는 payload 안으로 흡수.)
> 즉 §5의 `event_type:u16 + event_body:Bytes`를 **`t:String + payload:String(JSON)`**로 구체화했다(가독·디버깅 우선, 압축은 후속 flags COMPRESSED). edit/delete/react도 같은 `REALM_FANOUT`에 `t`만 바꿔 실어 보낸다.

### `PRESENCE_GOSSIP` (0x0201) — 구현됨 (Phase 3, Q11/D42)
```
user_id : u64
node_id : u64     # 이 유저를 호스팅(해제)한 노드
status  : u8      # user_status: 0=offline 1=online 2=idle 3=dnd
```
- 전역 presence 델타. 전이(첫/마지막 live 세션) 시 **모든 피어에 broadcast**(풀메시 D4). 수신 노드는 view 갱신 후 그 유저의 **로컬 친구**(relationships, D40)에게 `PRESENCE_UPDATE`(gateway JSON) 배달하고 **재브로드캐스트하지 않는다**(원본이 이미 전 피어 전송 → 루프 방지). Realm 무관 유저 라우팅 = D40/D41의 크로스노드 seam 해소.

### `REALM_COMMAND_RESULT` (0x0102)
```
origin_session_id : u64
ok                : bool
result            : enum(u16)  # MessageCreated{message_id:u64,...} | Error{code:u32, msg:String}
```

### `HELLO` (0x0001)
```
capabilities : u32     # 기능 비트(향후 확장용)
epoch        : u64     # 노드 시작 시각(ms) — 재시작 감지/failure detection
```
(노드 id는 헤더 `src_node_id`로 전달)

---

## 6. 연결 수립 & 풀메시 (D4/D5)

```
1. 부팅 시 정적 config에서 피어 목록 로드 (D5)
2. 중복 연결 방지: "id 작은 노드가 id 큰 노드에게 dial" (쌍당 1연결)
3. TCP connect → mTLS 핸드셰이크(rustls, 상호 인증서 검증, D16)
4. HELLO(version, src_node_id, epoch) 교환 → HELLO_ACK
   - version 불일치 → 연결 종료
5. 이후 PING/PONG로 생존 확인
```
- **failure detection**: PONG 미수신 N회 → 피어 down 판정 → 해당 노드 소유 Realm을 링에서 재배치(D23) → 새 소유 노드가 Postgres에서 rehydrate.
- (Phase 5) 동적 합류는 `RING_UPDATE` + gossip(SWIM, Q11).

---

## 7. 워크드 예시 — `SUBSCRIBE`

노드 7이 "유저 0xA가 Realm 0x100을 노드 7에서 구독" 등록:

```
바디(24B): realm_id=0x100, user_id=0xA, node_id=7
페이로드 = 헤더(28B) + 바디(24B) = 52B = 0x34

[length]        00 00 00 34
[version]       01
[msg_type]      01 10                  # 0x0110 SUBSCRIBE
[flags]         00
[trace_id]      00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
[src_node_id]   00 00 00 00 00 00 00 07
[body.realm_id] 00 00 00 00 00 00 01 00
[body.user_id]  00 00 00 00 00 00 00 0A
[body.node_id]  00 00 00 00 00 00 00 07
```
총 4(length) + 52 = 56바이트 전송.

---

## 8. 에러 처리 & 불변식
- 프레임 > 16 MiB → 연결 종료.
- 디코드 실패(길이 부족/태그 미상) → 연결 종료 + `tracing` 에러 로그(trace_id 포함).
- mTLS 검증 실패 → 연결 거부.
- 헤더 28바이트 미만 payload → 잘못된 프레임, 종료.
- **불변식**: 한 Realm은 한 시점에 한 노드만 소유(D9). FANOUT은 항상 소유 노드만 발사.

## 9. 미정/후속
- 압축(zstd) flag 활성 (후속 최적화).
- `RING_UPDATE`/gossip 상세 (Phase 5, Q11).
- ACK/재전송 정책 세부 (REQUIRES_ACK flag 운용).
