import { useMutation, useQuery, useQueryClient } from '@tanstack/solid-query';
import { createSignal, createUniqueId, For, Show } from 'solid-js';
import { orgApi } from '../lib/api-client';
import { useAuth } from '../lib/auth';

export function SettingsPage() {
  const auth = useAuth();
  const queryClient = useQueryClient();

  const org = () => auth.user()?.org;
  const isAdmin = () => org()?.role === 'admin';
  const canManage = () => org()?.is_personal === false;

  const membersQuery = useQuery(() => ({
    queryKey: ['org', 'members'] as const,
    queryFn: orgApi.members,
    get enabled() {
      return canManage();
    },
  }));

  const invitationsQuery = useQuery(() => ({
    queryKey: ['org', 'invitations'] as const,
    queryFn: orgApi.invitations,
    get enabled() {
      return canManage() && isAdmin();
    },
  }));

  // --- Create organization (personal-workspace users only) ------------------
  const [orgName, setOrgName] = createSignal('');
  const orgNameId = createUniqueId();
  const createOrgMutation = useMutation(() => ({
    mutationFn: (name: string) => orgApi.create(name),
    onSuccess: () => {
      // The session was re-minted server-side for the new org. Full-page reload
      // so /me and all queries reflect it (never mutate ['currentUser'] from a
      // mutation callback — reactive-storm hazard, see auth-context.tsx).
      window.location.assign('/settings');
    },
  }));

  // --- Invitations -----------------------------------------------------------
  const [inviteEmail, setInviteEmail] = createSignal('');
  const [inviteRole, setInviteRole] = createSignal('member');
  const inviteEmailId = createUniqueId();
  const inviteRoleId = createUniqueId();
  const inviteMutation = useMutation(() => ({
    mutationFn: (input: { email: string; role: string }) =>
      orgApi.invite(input.email, input.role),
    onSuccess: () => {
      setInviteEmail('');
      queryClient.invalidateQueries({ queryKey: ['org', 'invitations'] });
    },
  }));

  const revokeMutation = useMutation(() => ({
    mutationFn: (id: string) => orgApi.revokeInvitation(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['org', 'invitations'] });
    },
  }));

  const submitInvite = (e: Event) => {
    e.preventDefault();
    if (!inviteEmail().trim()) return;
    inviteMutation.mutate({ email: inviteEmail().trim(), role: inviteRole() });
  };

  return (
    <div class="container mx-auto p-4 max-w-4xl space-y-6">
      <h1 class="text-3xl font-bold">Settings</h1>

      {/* Org card */}
      <Show when={org()} fallback={<div class="py-8">Loading…</div>} keyed>
        {(activeOrg) => (
          <div class="border rounded-lg p-4 bg-white">
            <h2 class="text-xl font-semibold mb-2">Organization</h2>
            <dl class="text-sm space-y-1">
              <div class="flex gap-2">
                <dt class="text-gray-500 w-24">Name</dt>
                <dd class="font-medium">{activeOrg.name}</dd>
              </div>
              <div class="flex gap-2">
                <dt class="text-gray-500 w-24">ID</dt>
                <dd class="font-mono text-xs self-center">
                  {activeOrg.external_id}
                </dd>
              </div>
              <div class="flex gap-2">
                <dt class="text-gray-500 w-24">Your role</dt>
                <dd>{activeOrg.role}</dd>
              </div>
            </dl>
          </div>
        )}
      </Show>

      {/* Personal workspace: self-serve org creation */}
      <Show when={org()?.is_personal}>
        <div class="border rounded-lg p-4 bg-white">
          <h2 class="text-xl font-semibold mb-1">Create an organization</h2>
          <p class="text-sm text-gray-500 mb-4">
            You're in a personal workspace — invitations require an
            organization. Create one to invite teammates; you'll become its
            admin instantly.
          </p>
          <form
            class="flex gap-2 items-end"
            onSubmit={(e) => {
              e.preventDefault();
              if (orgName().trim()) createOrgMutation.mutate(orgName().trim());
            }}
          >
            <div class="flex-1">
              <label class="block text-sm font-medium mb-1" for={orgNameId}>
                Organization name
              </label>
              <input
                id={orgNameId}
                class="w-full border rounded px-3 py-2"
                value={orgName()}
                onInput={(e) => setOrgName(e.currentTarget.value)}
                placeholder="Acme Inc."
                required
              />
            </div>
            <button
              type="submit"
              disabled={createOrgMutation.isPending || !orgName().trim()}
              class="px-4 py-2 rounded bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-50"
            >
              {createOrgMutation.isPending
                ? 'Creating…'
                : 'Create organization'}
            </button>
          </form>
          <Show when={createOrgMutation.isError}>
            <p class="text-sm text-red-600 mt-2">
              {createOrgMutation.error?.message ??
                'Failed to create organization'}
            </p>
          </Show>
        </div>
      </Show>

      {/* Members */}
      <Show when={canManage()}>
        <div class="border rounded-lg p-4 bg-white">
          <h2 class="text-xl font-semibold mb-4">Members</h2>
          <Show
            when={!membersQuery.isLoading}
            fallback={<div class="py-4">Loading members…</div>}
          >
            <table class="w-full text-sm">
              <thead>
                <tr class="text-left text-gray-500 border-b">
                  <th class="py-2">Name</th>
                  <th class="py-2">Email</th>
                  <th class="py-2">Role</th>
                  <th class="py-2">Status</th>
                </tr>
              </thead>
              <tbody>
                <For each={membersQuery.data ?? []}>
                  {(member) => (
                    <tr class="border-b last:border-0">
                      <td class="py-2">{member.name || '—'}</td>
                      <td class="py-2">{member.email}</td>
                      <td class="py-2">{member.role}</td>
                      <td class="py-2">{member.status}</td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </Show>
        </div>

        {/* Invitations (admins only) */}
        <Show
          when={isAdmin()}
          fallback={
            <p class="text-sm text-gray-500">
              Only organization admins can manage invitations.
            </p>
          }
        >
          <div class="border rounded-lg p-4 bg-white">
            <h2 class="text-xl font-semibold mb-4">Invite a teammate</h2>
            <form class="flex gap-2 items-end" onSubmit={submitInvite}>
              <div class="flex-1">
                <label
                  class="block text-sm font-medium mb-1"
                  for={inviteEmailId}
                >
                  Email
                </label>
                <input
                  id={inviteEmailId}
                  type="email"
                  class="w-full border rounded px-3 py-2"
                  value={inviteEmail()}
                  onInput={(e) => setInviteEmail(e.currentTarget.value)}
                  placeholder="teammate@example.com"
                  required
                />
              </div>
              <div>
                <label
                  class="block text-sm font-medium mb-1"
                  for={inviteRoleId}
                >
                  Role
                </label>
                <select
                  id={inviteRoleId}
                  class="border rounded px-3 py-2"
                  value={inviteRole()}
                  onChange={(e) => setInviteRole(e.currentTarget.value)}
                >
                  <option value="member">Member</option>
                  <option value="admin">Admin</option>
                </select>
              </div>
              <button
                type="submit"
                disabled={inviteMutation.isPending || !inviteEmail().trim()}
                class="px-4 py-2 rounded bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-50"
              >
                {inviteMutation.isPending ? 'Sending…' : 'Send invite'}
              </button>
            </form>
            <Show when={inviteMutation.isError}>
              <p class="text-sm text-red-600 mt-2">
                {inviteMutation.error?.message ?? 'Failed to send invitation'}
              </p>
            </Show>

            <h3 class="text-lg font-semibold mt-6 mb-2">Pending invitations</h3>
            <Show
              when={!invitationsQuery.isLoading}
              fallback={<div class="py-2">Loading invitations…</div>}
            >
              <Show
                when={(invitationsQuery.data ?? []).length > 0}
                fallback={
                  <p class="text-sm text-gray-500">No pending invitations.</p>
                }
              >
                <table class="w-full text-sm">
                  <thead>
                    <tr class="text-left text-gray-500 border-b">
                      <th class="py-2">Email</th>
                      <th class="py-2">State</th>
                      <th class="py-2">Expires</th>
                      <th class="py-2" />
                    </tr>
                  </thead>
                  <tbody>
                    <For each={invitationsQuery.data ?? []}>
                      {(invitation) => (
                        <tr class="border-b last:border-0">
                          <td class="py-2">{invitation.email}</td>
                          <td class="py-2">{invitation.state}</td>
                          <td class="py-2">
                            {invitation.expires_at
                              ? new Date(
                                  invitation.expires_at,
                                ).toLocaleDateString()
                              : '—'}
                          </td>
                          <td class="py-2 text-right">
                            <Show when={invitation.state === 'pending'}>
                              <button
                                type="button"
                                class="text-red-500 hover:text-red-700 disabled:opacity-50"
                                disabled={revokeMutation.isPending}
                                onClick={() =>
                                  revokeMutation.mutate(invitation.id)
                                }
                              >
                                Revoke
                              </button>
                            </Show>
                          </td>
                        </tr>
                      )}
                    </For>
                  </tbody>
                </table>
              </Show>
            </Show>
          </div>
        </Show>
      </Show>
    </div>
  );
}
