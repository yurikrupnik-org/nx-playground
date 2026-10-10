import { createMemo, createSignal, For, Show } from 'solid-js';
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '../components/ui/card';
import {
  KIND_LABELS,
  OCI_REGISTRIES,
  type RegistryKind,
} from '../lib/oci-registries';

const KIND_BADGE_CLASSES: Record<RegistryKind, string> = {
  cloud: 'bg-blue-100 text-blue-800',
  saas: 'bg-purple-100 text-purple-800',
  'self-hosted': 'bg-green-100 text-green-800',
};

const KIND_FILTERS: Array<'all' | RegistryKind> = [
  'all',
  'cloud',
  'saas',
  'self-hosted',
];

export function RegistriesPage() {
  const [search, setSearch] = createSignal('');
  const [kind, setKind] = createSignal<'all' | RegistryKind>('all');

  const filtered = createMemo(() => {
    const query = search().trim().toLowerCase();
    const currentKind = kind();
    return OCI_REGISTRIES.filter((registry) => {
      if (currentKind !== 'all' && !registry.kinds.includes(currentKind)) {
        return false;
      }
      if (!query) return true;
      return [registry.name, registry.vendor, registry.host]
        .join(' ')
        .toLowerCase()
        .includes(query);
    });
  });

  return (
    <div class="container mx-auto p-4 max-w-6xl space-y-6">
      <div>
        <h2 class="text-2xl font-bold">OCI Registries</h2>
        <p class="text-sm text-gray-600 mt-1">
          Where you can push and pull OCI images and artifacts — managed cloud
          registries, hosted SaaS, and self-hosted options.
        </p>
      </div>

      <div class="flex flex-wrap items-center gap-3">
        <input
          type="search"
          placeholder="Search by name, vendor, or host…"
          class="border rounded-md px-3 py-1.5 text-sm w-72"
          value={search()}
          onInput={(e) => setSearch(e.currentTarget.value)}
        />
        <div class="flex gap-1">
          <For each={KIND_FILTERS}>
            {(value) => (
              <button
                type="button"
                class={`px-3 py-1 rounded-full text-xs font-medium ${
                  kind() === value
                    ? 'bg-gray-900 text-white'
                    : 'bg-gray-100 text-gray-700 hover:bg-gray-200'
                }`}
                onClick={() => setKind(value)}
              >
                {value === 'all' ? 'All' : KIND_LABELS[value]}
              </button>
            )}
          </For>
        </div>
        <span class="text-xs text-gray-500 ml-auto">
          {filtered().length} of {OCI_REGISTRIES.length} registries
        </span>
      </div>

      <div class="grid gap-4 sm:grid-cols-2">
        <For each={filtered()}>
          {(registry) => (
            <Card>
              <CardHeader class="pb-3">
                <div class="flex items-start justify-between gap-2">
                  <CardTitle class="text-base">{registry.name}</CardTitle>
                  <div class="flex gap-1 shrink-0">
                    <For each={registry.kinds}>
                      {(k) => (
                        <span
                          class={`px-2 py-0.5 rounded-full text-xs font-medium ${KIND_BADGE_CLASSES[k]}`}
                        >
                          {KIND_LABELS[k]}
                        </span>
                      )}
                    </For>
                  </div>
                </div>
                <CardDescription>{registry.vendor}</CardDescription>
              </CardHeader>
              <CardContent class="space-y-3 text-sm">
                <div>
                  <div class="text-xs font-semibold text-gray-500 uppercase">
                    Host
                  </div>
                  <code class="text-xs break-all">{registry.host}</code>
                </div>
                <div>
                  <div class="text-xs font-semibold text-gray-500 uppercase">
                    Login
                  </div>
                  <pre class="bg-gray-50 border rounded-md p-2 text-xs whitespace-pre-wrap break-all">
                    {registry.login}
                  </pre>
                </div>
                <div>
                  <div class="text-xs font-semibold text-gray-500 uppercase">
                    Example
                  </div>
                  <code class="text-xs break-all">{registry.example}</code>
                </div>
                <Show when={registry.notes}>
                  <p class="text-gray-600">{registry.notes}</p>
                </Show>
                <div class="flex items-center justify-between pt-1">
                  <span
                    class={`text-xs ${
                      registry.ociArtifacts ? 'text-green-700' : 'text-gray-400'
                    }`}
                  >
                    {registry.ociArtifacts
                      ? '✓ OCI artifacts (Helm, SBOM, signatures)'
                      : 'Images only'}
                  </span>
                  <a
                    href={registry.docsUrl}
                    target="_blank"
                    rel="noreferrer"
                    class="text-xs text-blue-600 hover:underline"
                  >
                    Docs ↗
                  </a>
                </div>
              </CardContent>
            </Card>
          )}
        </For>
      </div>

      <Show when={filtered().length === 0}>
        <p class="text-center text-sm text-gray-500 py-8">
          No registries match your search.
        </p>
      </Show>
    </div>
  );
}
