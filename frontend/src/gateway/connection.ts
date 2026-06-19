// Gateway WebSocket 클라이언트 (gateway.md §2, CLI `gateway_client.rs` 확장).
// 핸드셰이크 HELLO→IDENTIFY→READY + 주기 HEARTBEAT + DISPATCH 라우팅 + 끊김 시 RESUME 재연결.
// 프레임워크 무관 — zustand store가 콜백으로 구독한다.
import type { ReadyPayload } from "../api/types.ts";

// opcode (protocol/mod.rs 미러).
const OP = {
  DISPATCH: 0,
  HEARTBEAT: 1,
  IDENTIFY: 2,
  PRESENCE_UPDATE: 3,
  VOICE_STATE_UPDATE: 4,
  RESUME: 6,
  RECONNECT: 7,
  INVALID_SESSION: 9,
  HELLO: 10,
  HEARTBEAT_ACK: 11,
} as const;

export type GatewayState =
  | "connecting"
  | "identifying"
  | "ready"
  | "resuming"
  | "reconnecting"
  | "closed";

interface Frame {
  op: number;
  d?: unknown;
  s?: number;
  t?: string;
}

export interface GatewayCallbacks {
  onReady(d: ReadyPayload): void;
  onResumed(): void;
  /** DISPATCH 이벤트 (t, d, s). READY/RESUMED는 별도 콜백으로 분기되어 여기 안 옴. */
  onDispatch(t: string, d: unknown, s: number): void;
  onStateChange(state: GatewayState): void;
}

/** 같은 origin의 `/gateway` → Vite proxy가 backend WS로 전달. */
function gatewayUrl(): string {
  const proto = location.protocol === "https:" ? "wss:" : "ws:";
  return `${proto}//${location.host}/gateway`;
}

export class GatewayClient {
  private ws: WebSocket | null = null;
  private heartbeatTimer: ReturnType<typeof setInterval> | null = null;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private lastSeq = 0;
  private sessionId: string | null = null;
  private resumeToken: string | null = null;
  private closedByUser = false;
  private backoff = 1000;

  constructor(
    private token: string,
    private cb: GatewayCallbacks,
  ) {}

  connect(): void {
    this.closedByUser = false;
    this.openSocket(false);
  }

  /** 사용자 로그아웃 등 — 영구 종료(재연결 안 함). */
  close(): void {
    this.closedByUser = true;
    this.clearTimers();
    this.ws?.close();
    this.ws = null;
    this.cb.onStateChange("closed");
  }

  /** C→S op 3: idle/dnd/online 상태 변경 (D42). */
  setPresence(status: "online" | "idle" | "dnd"): void {
    this.sendFrame({ op: OP.PRESENCE_UPDATE, d: { status } });
  }

  /** C→S op 4: 음성 채널 입장/이동/퇴장 (D47, 제어 평면). */
  setVoiceState(realmId: string, channelId: string | null, selfMute = false, selfDeaf = false): void {
    this.sendFrame({
      op: OP.VOICE_STATE_UPDATE,
      d: { realm_id: realmId, channel_id: channelId, self_mute: selfMute, self_deaf: selfDeaf },
    });
  }

  // ── 내부 ────────────────────────────────────────────────────────────────
  private openSocket(resuming: boolean): void {
    this.cb.onStateChange(resuming ? "resuming" : "connecting");
    const ws = new WebSocket(gatewayUrl());
    this.ws = ws;

    ws.onmessage = (e) => {
      let frame: Frame;
      try {
        frame = JSON.parse(e.data as string);
      } catch {
        return;
      }
      this.handleFrame(frame, resuming);
    };
    ws.onclose = () => this.handleClose();
    ws.onerror = () => {
      /* onclose가 뒤따른다 — 거기서 재연결 처리. */
    };
  }

  private handleFrame(frame: Frame, resuming: boolean): void {
    switch (frame.op) {
      case OP.HELLO: {
        const interval = (frame.d as { heartbeat_interval?: number })?.heartbeat_interval ?? 30_000;
        this.startHeartbeat(interval);
        if (resuming && this.sessionId && this.resumeToken) {
          this.cb.onStateChange("resuming");
          this.sendFrame({
            op: OP.RESUME,
            d: { session_id: this.sessionId, token: this.resumeToken, seq: this.lastSeq },
          });
        } else {
          this.cb.onStateChange("identifying");
          this.sendFrame({ op: OP.IDENTIFY, d: { token: this.token } });
        }
        break;
      }
      case OP.DISPATCH: {
        if (typeof frame.s === "number") this.lastSeq = frame.s;
        const t = frame.t ?? "";
        if (t === "READY") {
          const d = frame.d as ReadyPayload;
          this.sessionId = d.session_id;
          this.resumeToken = d.resume_token;
          this.backoff = 1000;
          this.cb.onStateChange("ready");
          this.cb.onReady(d);
        } else if (t === "RESUMED") {
          this.backoff = 1000;
          this.cb.onStateChange("ready");
          this.cb.onResumed();
        } else {
          this.cb.onDispatch(t, frame.d, frame.s ?? this.lastSeq);
        }
        break;
      }
      case OP.HEARTBEAT_ACK:
        break;
      case OP.RECONNECT:
        // 서버 요청 재연결 → 소켓 닫고 RESUME 시도.
        this.ws?.close();
        break;
      case OP.INVALID_SESSION:
        // 세션 무효 → 재개 자격 폐기 후 신규 IDENTIFY(다음 연결에서).
        this.sessionId = null;
        this.resumeToken = null;
        this.lastSeq = 0;
        this.ws?.close();
        break;
      default:
        break;
    }
  }

  private handleClose(): void {
    this.clearTimers();
    if (this.closedByUser) return;
    // 재연결: 세션 자격이 있으면 RESUME, 없으면 신규 IDENTIFY.
    const willResume = this.sessionId !== null && this.resumeToken !== null;
    this.cb.onStateChange("reconnecting");
    this.reconnectTimer = setTimeout(() => {
      this.openSocket(willResume);
    }, this.backoff);
    this.backoff = Math.min(this.backoff * 2, 15_000);
  }

  private startHeartbeat(intervalMs: number): void {
    this.clearHeartbeat();
    this.heartbeatTimer = setInterval(() => {
      this.sendFrame({ op: OP.HEARTBEAT, d: this.lastSeq });
    }, intervalMs);
  }

  private sendFrame(frame: Frame): void {
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(frame));
    }
  }

  private clearHeartbeat(): void {
    if (this.heartbeatTimer) {
      clearInterval(this.heartbeatTimer);
      this.heartbeatTimer = null;
    }
  }

  private clearTimers(): void {
    this.clearHeartbeat();
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
  }
}
