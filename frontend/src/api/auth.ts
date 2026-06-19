// 인증 API (CLI `rest.rs`의 register/login/mfa/refresh 미러).
import { api } from "./http.ts";
import { solvePow } from "./pow.ts";
import type { AuthResponse, LoginResult, MfaEnableView, PowChallenge } from "./types.ts";

/**
 * 가입: ① PoW 챌린지 GET → ② worker로 풀이 → ③ 해를 실어 register POST (D18).
 * onSolving 콜백으로 "봇 방지 퍼즐 푸는 중" UI를 띄울 수 있다.
 */
export async function register(
  username: string,
  email: string,
  password: string,
  onSolving?: () => void,
): Promise<AuthResponse> {
  const ch = await api.get<PowChallenge>("/auth/pow-challenge", { auth: false });
  onSolving?.();
  const { nonce } = await solvePow(ch.challenge, ch.difficulty);
  return api.post<AuthResponse>("/auth/register", {
    auth: false,
    json: { username, email, password, pow_challenge: ch.challenge, pow_nonce: nonce },
  });
}

/** 로그인. MFA 활성 계정은 `{mfa_required:true}` 반환 → mfaLogin으로 2단계. */
export function login(username: string, password: string): Promise<LoginResult> {
  return api.post<LoginResult>("/auth/login", { auth: false, json: { username, password } });
}

export function isMfaRequired(r: LoginResult): r is { mfa_required: true } {
  return (r as { mfa_required?: boolean }).mfa_required === true;
}

/** MFA 로그인 2단계: 비번 + TOTP 코드 → 토큰. */
export function mfaLogin(username: string, password: string, code: string): Promise<AuthResponse> {
  return api.post<AuthResponse>("/auth/mfa/totp", { auth: false, json: { username, password, code } });
}

/** refresh 회전 → 새 토큰. */
export function refresh(refreshToken: string): Promise<AuthResponse> {
  return api.post<AuthResponse>("/auth/refresh", { auth: false, json: { refresh_token: refreshToken } });
}

/** TOTP MFA 활성화 시작 → secret(hex) + otpauth URI(미저장). */
export function mfaEnable(): Promise<MfaEnableView> {
  return api.post<MfaEnableView>("/auth/mfa/totp/enable");
}

/** TOTP secret+code 확인 → 활성화(저장). */
export function mfaVerify(secret: string, code: string): Promise<void> {
  return api.post<void>("/auth/mfa/totp/verify", { json: { secret, code } });
}
