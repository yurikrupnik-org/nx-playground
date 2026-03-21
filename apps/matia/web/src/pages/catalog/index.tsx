import { createSignal, For } from 'solid-js';
import { Badge } from '../../components/ui/badge';

interface Dataset {
  name: string;
  source: string;
  rows: number;
  columns: number;
  freshness: 'fresh' | 'stale' | 'unknown';
  tags: string[];
}

const mockDatasets: Dataset[] = [
  { name: 'sales', source: 'CSV', rows: 30, columns: 8, freshness: 'fresh', tags: ['revenue', 'orders'] },
  { name: 'customers', source: 'CSV', rows: 16, columns: 5, freshness: 'fresh', tags: ['users', 'crm'] },
  { name: 'active_users', source: 'PostgreSQL', rows: 1240, columns: 12, freshness: 'fresh', tags: ['users'] },
  { name: 'tasks', source: 'MongoDB', rows: 856, columns: 9, freshness: 'stale', tags: ['operations'] },
  { name: 'metrics_1h', source: 'InfluxDB', rows: 50000, columns: 6, freshness: 'fresh', tags: ['timeseries'] },
  { name: 'embeddings', source: 'Qdrant', rows: 2400, columns: 3, freshness: 'unknown', tags: ['vectors', 'ai'] },
];

const freshnessVariant = (f: Dataset['freshness']) =>
  f === 'fresh' ? 'success' as const : f === 'stale' ? 'warning' as const : 'muted' as const;

export function CatalogPage() {
  const [search, setSearch] = createSignal('');

  const filtered = () => {
    const q = search().toLowerCase();
    if (!q) return mockDatasets;
    return mockDatasets.filter(
      (d) => d.name.includes(q) || d.tags.some((t) => t.includes(q)),
    );
  };

  return (
    <div>
      <div class="flex items-center justify-between mb-6">
        <div>
          <h1 class="text-2xl font-bold">Data Catalog</h1>
          <p class="text-sm text-muted mt-1">Discover, organize, and govern datasets</p>
        </div>
        <input
          type="text"
          placeholder="Filter datasets..."
          class="bg-surface border border-border rounded-md px-3 py-1.5 text-sm text-slate-100 placeholder:text-muted outline-none focus:border-primary w-64"
          value={search()}
          onInput={(e) => setSearch(e.currentTarget.value)}
        />
      </div>

      <div class="bg-surface border border-border rounded-lg overflow-hidden">
        <table class="w-full text-sm">
          <thead>
            <tr class="border-b border-border text-left text-muted">
              <th class="px-4 py-3 font-medium">Name</th>
              <th class="px-4 py-3 font-medium">Source</th>
              <th class="px-4 py-3 font-medium text-right">Rows</th>
              <th class="px-4 py-3 font-medium text-right">Columns</th>
              <th class="px-4 py-3 font-medium">Freshness</th>
              <th class="px-4 py-3 font-medium">Tags</th>
            </tr>
          </thead>
          <tbody>
            <For each={filtered()}>
              {(dataset) => (
                <tr class="border-b border-border hover:bg-surface-raised transition-colors cursor-pointer">
                  <td class="px-4 py-3 font-medium text-slate-100">{dataset.name}</td>
                  <td class="px-4 py-3 text-muted">{dataset.source}</td>
                  <td class="px-4 py-3 text-right text-muted">{dataset.rows.toLocaleString()}</td>
                  <td class="px-4 py-3 text-right text-muted">{dataset.columns}</td>
                  <td class="px-4 py-3">
                    <Badge variant={freshnessVariant(dataset.freshness)}>{dataset.freshness}</Badge>
                  </td>
                  <td class="px-4 py-3">
                    <div class="flex gap-1">
                      <For each={dataset.tags}>
                        {(tag) => <Badge variant="muted">{tag}</Badge>}
                      </For>
                    </div>
                  </td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
    </div>
  );
}
