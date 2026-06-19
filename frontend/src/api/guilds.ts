// 길드/채널/멤버/초대 API (CLI `rest.rs` 미러 + v1.51 신규 목록 엔드포인트).
import { api } from "./http.ts";
import type { Snowflake } from "../lib/ids.ts";
import type {
  ChannelView,
  GuildView,
  InviteView,
  JoinView,
  MemberView,
  RealmView,
  RoleView,
} from "./types.ts";

// ── 디스커버리(v1.51) ─────────────────────────────────────────────────────
/** 내가 멤버인 Realm 목록(서버/DM, 이름·종류 포함). 웹 UI 좌측 레일의 출처. */
export function listMyRealms(): Promise<RealmView[]> {
  return api.get<RealmView[]>("/users/@me/realms");
}

/** 길드(Realm)의 채널 목록. 채널 트리의 출처. */
export function listChannels(realmId: Snowflake): Promise<ChannelView[]> {
  return api.get<ChannelView[]>(`/guilds/${realmId}/channels`);
}

// ── 생성/관리 ─────────────────────────────────────────────────────────────
export function createGuild(name: string): Promise<GuildView> {
  return api.post<GuildView>("/guilds", { json: { name } });
}

export function createChannel(realmId: Snowflake, name: string, kind?: string): Promise<ChannelView> {
  return api.post<ChannelView>(`/guilds/${realmId}/channels`, { json: { name, kind } });
}

export function createInvite(realmId: Snowflake, maxUses = 0, maxAge = 0): Promise<InviteView> {
  return api.post<InviteView>(`/guilds/${realmId}/invites`, { json: { max_uses: maxUses, max_age: maxAge } });
}

export function joinInvite(code: string): Promise<JoinView> {
  return api.post<JoinView>(`/invites/${code}`);
}

export function listMembers(realmId: Snowflake): Promise<MemberView[]> {
  return api.get<MemberView[]>(`/guilds/${realmId}/members`);
}

export function setNick(realmId: Snowflake, userId: Snowflake, nick: string | null): Promise<MemberView> {
  return api.patch<MemberView>(`/guilds/${realmId}/members/${userId}`, { json: { nick } });
}

/** 추방(타인) 또는 탈퇴(userId="@me"). */
export function removeMember(realmId: Snowflake, userId: Snowflake): Promise<void> {
  return api.del<void>(`/guilds/${realmId}/members/${userId}`);
}

export function listRoles(realmId: Snowflake): Promise<RoleView[]> {
  return api.get<RoleView[]>(`/guilds/${realmId}/roles`);
}
