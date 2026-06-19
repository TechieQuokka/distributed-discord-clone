// 길드 채널 사이드바 — 텍스트/음성 채널 + 초대/생성 + 하단 유저 패널.
import { useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useChannels, useRealms } from "../hooks/queries.ts";
import { useNameResolver } from "../hooks/names.ts";
import { createChannel, createInvite } from "../api/guilds.ts";
import { qk } from "../api/queryKeys.ts";
import { useUi } from "../store/ui.ts";
import { useAuth } from "../store/auth.ts";
import { useVoice } from "../store/voice.ts";
import { getGatewayClient } from "../gateway/RealtimeProvider.tsx";
import { UserPanel } from "./UserPanel.tsx";
import { GuildSettingsModal } from "./GuildSettingsModal.tsx";
import type { Snowflake } from "../lib/ids.ts";
import type { ChannelView } from "../api/types.ts";

const TEXT_KINDS = new Set(["text", "announcement", "forum"]);

export function ChannelSidebar({ realmId }: { realmId: Snowflake }) {
  const qc = useQueryClient();
  const { data: realms } = useRealms();
  const { data: channels } = useChannels(realmId);
  const selectedChannelId = useUi((s) => s.selectedChannelId);
  const selectChannel = useUi((s) => s.selectChannel);
  const [invite, setInvite] = useState<string | null>(null);
  const [settings, setSettings] = useState(false);

  const realm = realms?.find((r) => r.id === realmId);
  const textChannels = (channels ?? []).filter((c) => TEXT_KINDS.has(c.kind));
  const voiceChannels = (channels ?? []).filter((c) => c.kind === "voice");

  useEffect(() => {
    if (!channels || channels.length === 0) return;
    const exists = channels.some((c) => c.id === selectedChannelId && TEXT_KINDS.has(c.kind));
    if (!exists) {
      const first = channels.find((c) => TEXT_KINDS.has(c.kind));
      if (first) selectChannel(first.id);
    }
  }, [channels, selectedChannelId, selectChannel]);

  async function onCreateChannel(kind: "text" | "voice") {
    const name = prompt(kind === "voice" ? "새 음성 채널 이름" : "새 텍스트 채널 이름");
    if (!name?.trim()) return;
    await createChannel(realmId, name.trim(), kind);
    qc.invalidateQueries({ queryKey: qk.channels(realmId) });
  }

  async function onInvite() {
    const inv = await createInvite(realmId);
    setInvite(inv.code);
  }

  return (
    <div className="flex w-60 flex-col bg-bg-sidebar">
      <header className="flex h-12 items-center justify-between border-b border-black/20 px-4 shadow-sm">
        <span className="truncate font-semibold text-text-bright">{realm?.name ?? "서버"}</span>
        <div className="flex items-center gap-2">
          <button onClick={onInvite} title="초대 코드 생성" className="text-text-muted hover:text-text-bright">
            ＋친구
          </button>
          <button onClick={() => setSettings(true)} title="서버 관리" className="text-text-muted hover:text-text-bright">
            ⚙
          </button>
        </div>
      </header>
      {settings && <GuildSettingsModal realmId={realmId} onClose={() => setSettings(false)} />}

      {invite && (
        <div className="bg-bg-server px-3 py-2 text-xs text-text-muted">
          초대 코드: <span className="select-all font-mono text-online">{invite}</span>
          <button onClick={() => setInvite(null)} className="float-right hover:text-text-bright">✕</button>
        </div>
      )}

      <div className="flex-1 overflow-y-auto px-2 pt-3">
        <SectionHeader label="텍스트 채널" onAdd={() => onCreateChannel("text")} />
        {textChannels.map((c) => (
          <ChannelRow
            key={c.id}
            channel={c}
            active={c.id === selectedChannelId}
            onClick={() => selectChannel(c.id)}
          />
        ))}
        {textChannels.length === 0 && <p className="px-2 py-1 text-xs text-text-muted">채널이 없습니다.</p>}

        <div className="mt-4" />
        <SectionHeader label="음성 채널" onAdd={() => onCreateChannel("voice")} />
        {voiceChannels.map((c) => (
          <VoiceChannelRow key={c.id} channel={c} realmId={realmId} />
        ))}
        {voiceChannels.length === 0 && (
          <p className="px-2 py-1 text-xs text-text-muted">음성 채널이 없습니다.</p>
        )}
      </div>

      <UserPanel />
    </div>
  );
}

function SectionHeader({ label, onAdd }: { label: string; onAdd: () => void }) {
  return (
    <div className="flex items-center justify-between px-1 pb-1">
      <span className="text-xs font-bold uppercase text-text-muted">{label}</span>
      <button onClick={onAdd} title="채널 생성" className="text-text-muted hover:text-text-bright">＋</button>
    </div>
  );
}

function ChannelRow({
  channel,
  active,
  onClick,
}: {
  channel: ChannelView;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={`flex w-full items-center gap-1.5 rounded px-2 py-1.5 text-left text-sm ${
        active ? "bg-bg-selected text-text-bright" : "text-text-muted hover:bg-bg-hover hover:text-text-normal"
      }`}
    >
      <span className="text-lg leading-none text-text-muted">#</span>
      <span className="truncate">{channel.name ?? channel.id}</span>
    </button>
  );
}

function VoiceChannelRow({ channel, realmId }: { channel: ChannelView; realmId: Snowflake }) {
  const myId = useAuth((s) => s.userId);
  const participants = useVoice((s) => s.byChannel[channel.id]);
  const myChannel = useVoice((s) => (myId ? s.userChannel[myId] : undefined));
  const nameOf = useNameResolver(realmId);
  const joinedHere = myChannel === channel.id;

  function toggle() {
    const gw = getGatewayClient();
    if (!gw) return;
    gw.setVoiceState(realmId, joinedHere ? null : channel.id);
  }

  const members = Object.values(participants ?? {});
  return (
    <div>
      <button
        onClick={toggle}
        className={`flex w-full items-center gap-1.5 rounded px-2 py-1.5 text-left text-sm ${
          joinedHere ? "bg-bg-selected text-text-bright" : "text-text-muted hover:bg-bg-hover hover:text-text-normal"
        }`}
      >
        <span className="leading-none">🔊</span>
        <span className="truncate">{channel.name ?? channel.id}</span>
        {joinedHere && <span className="ml-auto text-xs text-online">연결됨</span>}
      </button>
      {members.length > 0 && (
        <div className="ml-5 space-y-0.5 py-0.5">
          {members.map((m) => (
            <div key={m.user_id} className="flex items-center gap-1 text-xs text-text-muted">
              <span className="h-1.5 w-1.5 rounded-full bg-online" />
              <span className="truncate">{nameOf(m.user_id)}</span>
              {m.self_mute && <span title="음소거">🔇</span>}
              {m.self_deaf && <span title="헤드셋 끔">🎧</span>}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
