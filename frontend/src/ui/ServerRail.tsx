// 좌측 서버 레일 — 내 길드(realm kind=guild) 아이콘 + 길드 추가 버튼.
import { useState } from "react";
import { useRealms } from "../hooks/queries.ts";
import { useUi } from "../store/ui.ts";
import { initials } from "../lib/display.ts";
import { CreateServerModal } from "./CreateServerModal.tsx";
import type { RealmView } from "../api/types.ts";

export function ServerRail() {
  const { data: realms } = useRealms();
  const selectedRealmId = useUi((s) => s.selectedRealmId);
  const selectRealm = useUi((s) => s.selectRealm);
  const [showModal, setShowModal] = useState(false);

  // 서버(길드)와 DM은 분리 표시. 레일엔 길드만.
  const guilds = (realms ?? []).filter((r) => r.kind === "guild");

  return (
    <nav className="flex w-[72px] flex-col items-center gap-2 bg-bg-server py-3">
      <RailButton
        label="다이렉트 메시지"
        active={selectedRealmId === null}
        onClick={() => selectRealm(null)}
        accent
      >
        DM
      </RailButton>
      <div className="my-1 h-0.5 w-8 rounded bg-bg-sidebar" />

      <div className="flex flex-1 flex-col items-center gap-2 overflow-y-auto">
        {guilds.map((g) => (
          <ServerIcon
            key={g.id}
            realm={g}
            active={g.id === selectedRealmId}
            onClick={() => selectRealm(g.id)}
          />
        ))}
      </div>

      <RailButton label="서버 추가" onClick={() => setShowModal(true)} addBtn>
        ＋
      </RailButton>

      {showModal && <CreateServerModal onClose={() => setShowModal(false)} />}
    </nav>
  );
}

function ServerIcon({
  realm,
  active,
  onClick,
}: {
  realm: RealmView;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <RailButton label={realm.name ?? realm.id} active={active} onClick={onClick}>
      {initials(realm.name, "G")}
    </RailButton>
  );
}

function RailButton({
  children,
  label,
  active,
  accent,
  addBtn,
  onClick,
}: {
  children: React.ReactNode;
  label: string;
  active?: boolean;
  accent?: boolean;
  addBtn?: boolean;
  onClick: () => void;
}) {
  const base =
    "relative flex h-12 w-12 items-center justify-center text-sm font-semibold transition-all duration-150";
  const shape = active ? "rounded-2xl" : "rounded-3xl hover:rounded-2xl";
  const color = active
    ? "bg-accent text-white"
    : addBtn
      ? "bg-bg-sidebar text-online hover:bg-online hover:text-white"
      : accent
        ? "bg-bg-sidebar text-accent hover:bg-accent hover:text-white"
        : "bg-bg-sidebar text-text-bright hover:bg-accent hover:text-white";
  return (
    <button onClick={onClick} title={label} className={`${base} ${shape} ${color}`}>
      {active && (
        <span className="absolute -left-3 h-8 w-1 rounded-r bg-white" aria-hidden />
      )}
      {children}
    </button>
  );
}
