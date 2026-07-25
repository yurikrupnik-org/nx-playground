import {
  createMutation,
  createQuery,
  useQueryClient,
} from '@tanstack/solid-query';
import { createSignal, For, Show } from 'solid-js';
import * as assetsApi from '../lib/assets-api';
import { useAuth } from '../lib/auth-context';

export function AssetsPage() {
  const auth = useAuth();
  const queryClient = useQueryClient();

  const assets = createQuery(() => ({
    queryKey: ['assets'],
    queryFn: assetsApi.listAssets,
  }));

  const [provider, setProvider] = createSignal('aws');
  const [name, setName] = createSignal('');
  const [externalId, setExternalId] = createSignal('');
  const [assetType, setAssetType] = createSignal('ec2/t3.micro');
  const [region, setRegion] = createSignal('us-east-1');
  const [monthlyCost, setMonthlyCost] = createSignal(0);

  const create = createMutation(() => ({
    mutationFn: assetsApi.createAsset,
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ['assets'] });
      setName('');
      setExternalId('');
    },
  }));

  const submit = (e: Event) => {
    e.preventDefault();
    create.mutate({
      provider: provider(),
      external_id: externalId(),
      name: name(),
      asset_type: assetType(),
      region: region(),
      monthly_cost: Number(monthlyCost()),
    });
  };

  return (
    <div class="mx-auto max-w-5xl px-4 py-8">
      <header class="mb-6">
        <h2 class="text-2xl font-semibold">Cloud assets</h2>
        <p class="text-sm text-gray-500">
          Organization {auth.user()?.org_id} · role {auth.user()?.role}
        </p>
      </header>

      <form
        onSubmit={submit}
        class="mb-8 grid grid-cols-2 gap-3 rounded-lg border border-gray-200 bg-white p-4 sm:grid-cols-3"
      >
        <select
          class="rounded border px-2 py-1 text-sm"
          value={provider()}
          onChange={(e) => setProvider(e.currentTarget.value)}
        >
          <option value="aws">aws</option>
          <option value="gcp">gcp</option>
          <option value="azure">azure</option>
        </select>
        <input
          class="rounded border px-2 py-1 text-sm"
          placeholder="name"
          value={name()}
          onInput={(e) => setName(e.currentTarget.value)}
        />
        <input
          class="rounded border px-2 py-1 text-sm"
          placeholder="external id"
          value={externalId()}
          onInput={(e) => setExternalId(e.currentTarget.value)}
        />
        <input
          class="rounded border px-2 py-1 text-sm"
          placeholder="type"
          value={assetType()}
          onInput={(e) => setAssetType(e.currentTarget.value)}
        />
        <input
          class="rounded border px-2 py-1 text-sm"
          placeholder="region"
          value={region()}
          onInput={(e) => setRegion(e.currentTarget.value)}
        />
        <input
          class="rounded border px-2 py-1 text-sm"
          type="number"
          step="0.01"
          placeholder="monthly cost"
          value={monthlyCost()}
          onInput={(e) => setMonthlyCost(Number(e.currentTarget.value))}
        />
        <button
          type="submit"
          disabled={create.isPending}
          class="col-span-2 rounded-md bg-gray-900 px-4 py-2 text-sm font-medium text-white hover:bg-gray-800 disabled:opacity-50 sm:col-span-3"
        >
          {create.isPending ? 'Adding…' : 'Add asset'}
        </button>
      </form>

      <Show
        when={!assets.isLoading}
        fallback={<p class="text-gray-500">Loading…</p>}
      >
        <Show
          when={(assets.data?.length ?? 0) > 0}
          fallback={<p class="text-gray-500">No assets yet.</p>}
        >
          <table class="w-full border-collapse text-sm">
            <thead>
              <tr class="border-b text-left text-gray-500">
                <th class="py-2">Provider</th>
                <th class="py-2">Name</th>
                <th class="py-2">Type</th>
                <th class="py-2">Region</th>
                <th class="py-2">Status</th>
                <th class="py-2 text-right">$/mo</th>
              </tr>
            </thead>
            <tbody>
              <For each={assets.data}>
                {(a) => (
                  <tr class="border-b">
                    <td class="py-2">{a.provider}</td>
                    <td class="py-2">{a.name}</td>
                    <td class="py-2">{a.asset_type}</td>
                    <td class="py-2">{a.region}</td>
                    <td class="py-2">{a.status}</td>
                    <td class="py-2 text-right">{a.monthly_cost.toFixed(2)}</td>
                  </tr>
                )}
              </For>
            </tbody>
          </table>
        </Show>
      </Show>
    </div>
  );
}
