// REST 호출 기반 (CLI `rest.rs`의 reqwest 헬퍼 미러). 모든 경로는 Vite proxy `/api` 접두 →
// backend 루트로 rewrite된다(예: `/api/auth/login` → `http://127.0.0.1:8080/auth/login`).
// backend는 `/api/v1` 같은 접두가 없다(CLI ground truth).

export const API_BASE = "/api";

// 액세스 토큰 보관소. 인증 store가 갱신하고, 모든 요청이 Bearer로 첨부한다.
// (React Query 호출마다 토큰을 인자로 흘리지 않기 위한 단일 출처.)
let accessToken: string | null = null;
export function setAccessToken(token: string | null): void {
  accessToken = token;
}
export function getAccessToken(): string | null {
  return accessToken;
}

// backend 표준 에러 본문은 평문 또는 `{"error","message"}`. 상태코드 + 메시지를 보존한다.
export class ApiError extends Error {
  constructor(
    public status: number,
    public body: string,
    public payload?: unknown,
  ) {
    super(`${status}: ${body}`);
    this.name = "ApiError";
  }
}

interface RequestOptions {
  /** Bearer 토큰을 붙일지(기본 true). pow-challenge/register/login/webhook 실행은 false. */
  auth?: boolean;
  /** JSON 본문(있으면 content-type 설정). */
  json?: unknown;
  /** 쿼리 파라미터. */
  query?: Record<string, string | number | undefined>;
  signal?: AbortSignal;
}

function buildUrl(path: string, query?: RequestOptions["query"]): string {
  const url = API_BASE + path;
  if (!query) return url;
  const qs = new URLSearchParams();
  for (const [k, v] of Object.entries(query)) {
    if (v !== undefined) qs.set(k, String(v));
  }
  const s = qs.toString();
  return s ? `${url}?${s}` : url;
}

async function parseError(res: Response): Promise<ApiError> {
  const text = await res.text().catch(() => "");
  let message = text;
  let payload: unknown;
  try {
    payload = JSON.parse(text);
    if (payload && typeof payload === "object" && "message" in payload) {
      message = String((payload as { message: unknown }).message);
    }
  } catch {
    /* 평문 본문 — 그대로. */
  }
  return new ApiError(res.status, message || res.statusText, payload);
}

/** 코어 요청. 성공 시 JSON(T)로 파싱. 204/빈 본문은 undefined를 T로 반환. */
export async function request<T>(method: string, path: string, opts: RequestOptions = {}): Promise<T> {
  const headers: Record<string, string> = {};
  const useAuth = opts.auth ?? true;
  if (useAuth && accessToken) headers["authorization"] = `Bearer ${accessToken}`;

  let body: BodyInit | undefined;
  if (opts.json !== undefined) {
    headers["content-type"] = "application/json";
    body = JSON.stringify(opts.json);
  }

  const res = await fetch(buildUrl(path, opts.query), { method, headers, body, signal: opts.signal });
  if (!res.ok) throw await parseError(res);

  if (res.status === 204) return undefined as T;
  const text = await res.text();
  if (!text) return undefined as T;
  return JSON.parse(text) as T;
}

export const api = {
  get: <T>(path: string, opts?: RequestOptions) => request<T>("GET", path, opts),
  post: <T>(path: string, opts?: RequestOptions) => request<T>("POST", path, opts),
  put: <T>(path: string, opts?: RequestOptions) => request<T>("PUT", path, opts),
  patch: <T>(path: string, opts?: RequestOptions) => request<T>("PATCH", path, opts),
  del: <T>(path: string, opts?: RequestOptions) => request<T>("DELETE", path, opts),
};
