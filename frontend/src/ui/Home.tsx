// 메인 레이아웃 — 서버 레일 + (DM | 길드) 사이드바 + 채팅 + 멤버.
import { useEffect } from "react";
import { useRealms, useChannels, useMembers } from "../hooks/queries.ts";
import { useNameResolver } from "../hooks/names.ts";
import { useUi } from "../store/ui.ts";
import { useAuth } from "../store/auth.ts";
import { ServerRail } from "./ServerRail.tsx";
import { ChannelSidebar } from "./ChannelSidebar.tsx";
import { DmSidebar } from "./DmSidebar.tsx";
import { ChatArea } from "./ChatArea.tsx";
import { MemberList } from "./MemberList.tsx";
import type { Snowflake } from "../lib/ids.ts";
import type { RealmView } from "../api/types.ts";

export function Home() {
  const { data: realms } = useRealms();
  const selectedRealmId = useUi((s) => s.selectedRealmId);
  const realm = realms?.find((r) => r.id === selectedRealmId);

  return (
    <div className="flex h-full">
      <ServerRail />
      {selectedRealmId === null ? (
        <>
          <DmSidebar />
          <Welcome />
        </>
      ) : realm?.kind === "guild" ? (
        <GuildView realmId={selectedRealmId} />
      ) : (
        <>
          <DmSidebar />
          <DmChat realmId={selectedRealmId} realm={realm} />
        </>
      )}
    </div>
  );
}

function GuildView({ realmId }: { realmId: Snowflake }) {
  const { data: channels } = useChannels(realmId);
  const selectedChannelId = useUi((s) => s.selectedChannelId);
  const channel = channels?.find((c) => c.id === selectedChannelId);

  return (
    <>
      <ChannelSidebar realmId={realmId} />
      {selectedChannelId && channel ? (
        <ChatArea channelId={selectedChannelId} channelName={channel.name ?? "채널"} realmId={realmId} />
      ) : (
        <Welcome />
      )}
      <MemberList realmId={realmId} />
    </>
  );
}

function DmChat({ realmId, realm }: { realmId: Snowflake; realm?: RealmView }) {
  const { data: channels } = useChannels(realmId);
  const { data: members } = useMembers(realm?.kind === "dm" ? realmId : null);
  const nameOf = useNameResolver(realm?.kind === "dm" ? realmId : null);
  const myId = useAuth((s) => s.userId);
  const selectedChannelId = useUi((s) => s.selectedChannelId);
  const selectChannel = useUi((s) => s.selectChannel);

  // DM realm의 단일 채널 자동 선택.
  useEffect(() => {
    if (!channels || channels.length === 0) return;
    if (!channels.some((c) => c.id === selectedChannelId)) selectChannel(channels[0].id);
  }, [channels, selectedChannelId, selectChannel]);

  // 헤더 표시명: 그룹은 그룹명, 1:1은 상대 이름.
  const otherId = realm?.kind === "dm" ? (members ?? []).find((m) => m.user_id !== myId)?.user_id : undefined;
  const title =
    realm?.kind === "group_dm"
      ? (realm.name ?? "그룹 DM")
      : otherId
        ? nameOf(otherId)
        : "다이렉트 메시지";

  if (!selectedChannelId) return <Welcome />;
  return <ChatArea channelId={selectedChannelId} channelName={title} realmId={null} />;
}

function Welcome() {
  return (
    <div className="flex flex-1 items-center justify-center bg-bg-app text-center">
      <div>
        <div className="mb-2 text-4xl">💬</div>
        <p className="text-text-muted">서버나 채널을 선택해 대화를 시작하세요.</p>
      </div>
    </div>
  );
}
