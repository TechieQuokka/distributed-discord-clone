// 로그인 / 가입 화면. 가입은 PoW 퍼즐(D18)을 worker로 풀고, 로그인은 MFA 2단계를 처리.
import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { isMfaRequired, login, mfaLogin, register } from "../api/auth.ts";
import { useAuth } from "../store/auth.ts";
import { ApiError } from "../api/http.ts";

type Mode = "login" | "register";

export function Login() {
  const navigate = useNavigate();
  const setSession = useAuth((s) => s.setSession);

  const [mode, setMode] = useState<Mode>("login");
  const [username, setUsername] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [mfaCode, setMfaCode] = useState("");
  const [needMfa, setNeedMfa] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const friendlyError = (e: unknown): string => {
    if (e instanceof ApiError) return e.message || `${e.status}`;
    return e instanceof Error ? e.message : String(e);
  };

  async function onSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    try {
      if (mode === "register") {
        setBusy("계정 생성 중…");
        const res = await register(username, email, password, () =>
          setBusy("봇 방지 퍼즐 푸는 중 (PoW)…"),
        );
        setSession(res, username);
        navigate("/");
        return;
      }

      // login
      if (needMfa) {
        setBusy("2단계 인증 확인 중…");
        const res = await mfaLogin(username, password, mfaCode);
        setSession(res, username);
        navigate("/");
        return;
      }
      setBusy("로그인 중…");
      const res = await login(username, password);
      if (isMfaRequired(res)) {
        setNeedMfa(true);
        setBusy(null);
        return;
      }
      setSession(res, username);
      navigate("/");
    } catch (err) {
      setError(friendlyError(err));
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="flex h-full items-center justify-center bg-bg-server">
      <form
        onSubmit={onSubmit}
        className="w-[440px] rounded-md bg-bg-sidebar p-8 shadow-2xl"
      >
        <h1 className="mb-1 text-center text-2xl font-bold text-text-bright">
          {mode === "login" ? "돌아오신 걸 환영해요!" : "계정 만들기"}
        </h1>
        <p className="mb-6 text-center text-sm text-text-muted">분산 Discord 클론</p>

        <label className="mb-1 block text-xs font-bold uppercase text-text-muted">사용자명</label>
        <input
          value={username}
          onChange={(e) => setUsername(e.target.value)}
          className="mb-4 w-full rounded bg-bg-server px-3 py-2 text-text-bright outline-none focus:ring-2 focus:ring-accent"
          autoComplete="username"
          required
        />

        {mode === "register" && (
          <>
            <label className="mb-1 block text-xs font-bold uppercase text-text-muted">이메일</label>
            <input
              type="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              className="mb-4 w-full rounded bg-bg-server px-3 py-2 text-text-bright outline-none focus:ring-2 focus:ring-accent"
              autoComplete="email"
              required
            />
          </>
        )}

        <label className="mb-1 block text-xs font-bold uppercase text-text-muted">비밀번호</label>
        <input
          type="password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          className="mb-4 w-full rounded bg-bg-server px-3 py-2 text-text-bright outline-none focus:ring-2 focus:ring-accent"
          autoComplete={mode === "login" ? "current-password" : "new-password"}
          required
        />

        {needMfa && (
          <>
            <label className="mb-1 block text-xs font-bold uppercase text-text-muted">
              인증 앱 코드 (TOTP)
            </label>
            <input
              value={mfaCode}
              onChange={(e) => setMfaCode(e.target.value)}
              className="mb-4 w-full rounded bg-bg-server px-3 py-2 tracking-widest text-text-bright outline-none focus:ring-2 focus:ring-accent"
              inputMode="numeric"
              placeholder="123456"
              required
            />
          </>
        )}

        {error && <p className="mb-3 text-sm text-dnd">⚠ {error}</p>}

        <button
          type="submit"
          disabled={busy !== null}
          className="mb-3 w-full rounded bg-accent py-2.5 font-medium text-white transition hover:bg-accent-hover disabled:opacity-60"
        >
          {busy ?? (mode === "login" ? "로그인" : "계속하기")}
        </button>

        <p className="text-sm text-text-muted">
          {mode === "login" ? "계정이 필요한가요? " : "이미 계정이 있나요? "}
          <button
            type="button"
            onClick={() => {
              setMode(mode === "login" ? "register" : "login");
              setNeedMfa(false);
              setError(null);
            }}
            className="text-accent hover:underline"
          >
            {mode === "login" ? "가입하기" : "로그인"}
          </button>
        </p>
      </form>
    </div>
  );
}
