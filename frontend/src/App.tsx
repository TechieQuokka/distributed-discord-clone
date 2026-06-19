// 라우팅 + 인증 게이트. 인증되면 RealtimeProvider(WS 연결)로 감싼 Home, 아니면 Login.
import { Navigate, Route, Routes } from "react-router-dom";
import { useAuth } from "./store/auth.ts";
import { RealtimeProvider } from "./gateway/RealtimeProvider.tsx";
import { Login } from "./ui/Login.tsx";
import { Home } from "./ui/Home.tsx";

export function App() {
  const isAuthed = useAuth((s) => s.isAuthed);

  return (
    <Routes>
      <Route path="/login" element={isAuthed ? <Navigate to="/" replace /> : <Login />} />
      <Route
        path="/*"
        element={
          isAuthed ? (
            <RealtimeProvider>
              <Home />
            </RealtimeProvider>
          ) : (
            <Navigate to="/login" replace />
          )
        }
      />
    </Routes>
  );
}
