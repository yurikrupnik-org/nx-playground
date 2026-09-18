import { createQuery } from '@tanstack/solid-query';
import { For, Show } from 'solid-js';
import * as inventoryApi from '../lib/inventory-api';

const STATUS_CLASS: Record<string, string> = {
  active: 'bg-green-100 text-green-800',
  failed: 'bg-red-100 text-red-800',
};

export function CloudResourcesPage() {
  const resources = createQuery(() => ({
    queryKey: ['cloud-resources'],
    queryFn: inventoryApi.listCloudResources,
  }));

  return (
    <div class="mx-auto max-w-5xl px-4 py-8">
      <header class="mb-6">
        <h2 class="text-2xl font-semibold">Cloud resources</h2>
        <p class="text-sm text-gray-500">
          Read-only inventory observed by Crossplane. Nothing here can be
          changed from this app.
        </p>
      </header>

      <Show
        when={!resources.isLoading}
        fallback={<p class="text-gray-500">Loading…</p>}
      >
        <Show
          when={!resources.error}
          fallback={
            <p class="rounded-md bg-red-50 px-4 py-3 text-sm text-red-800">
              {resources.error?.message}
            </p>
          }
        >
          <Show
            when={(resources.data?.length ?? 0) > 0}
            fallback={
              <p class="text-gray-500">
                No observed resources — create an inventory with{' '}
                <code>just inventory-create</code>.
              </p>
            }
          >
            <table class="w-full border-collapse text-sm">
              <thead>
                <tr class="border-b text-left text-gray-500">
                  <th class="py-2 pr-4">Name</th>
                  <th class="py-2 pr-4">Kind</th>
                  <th class="py-2 pr-4">Namespace</th>
                  <th class="py-2 pr-4">Type</th>
                  <th class="py-2 pr-4">Status</th>
                  <th class="py-2 pr-4">Inventory</th>
                  <th class="py-2">Tags</th>
                </tr>
              </thead>
              <tbody>
                <For each={resources.data}>
                  {(r) => (
                    <tr class="border-b">
                      <td class="py-2 pr-4 font-medium">{r.name}</td>
                      <td class="py-2 pr-4">{r.kind}</td>
                      <td class="py-2 pr-4">{r.namespace ?? '—'}</td>
                      <td class="py-2 pr-4">{r.resource_type}</td>
                      <td class="py-2 pr-4">
                        <span
                          class={`rounded px-2 py-0.5 text-xs font-medium ${
                            STATUS_CLASS[r.status] ??
                            'bg-gray-100 text-gray-800'
                          }`}
                        >
                          {r.status}
                        </span>
                      </td>
                      <td class="py-2 pr-4">{r.claim}</td>
                      <td class="py-2 text-gray-500">
                        {r.tags.map((t) => `${t.key}=${t.value}`).join(', ') ||
                          '—'}
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </Show>
        </Show>
      </Show>
    </div>
  );
}
