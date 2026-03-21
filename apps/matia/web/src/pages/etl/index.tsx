import { For } from 'solid-js';
import { Badge } from '../../components/ui/badge';

interface Pipeline {
  name: string;
  source: string;
  destination: string;
  schedule: string;
  lastRun: string;
  status: 'running' | 'success' | 'failed' | 'idle';
  rows: number;
}

const mockPipelines: Pipeline[] = [
  { name: 'revenue_by_category', source: 'sales (CSV)', destination: 'Catalog', schedule: 'On demand', lastRun: '2 min ago', status: 'success', rows: 4 },
  { name: 'active_users_sync', source: 'PostgreSQL', destination: 'Catalog', schedule: '*/15 * * * *', lastRun: '12 min ago', status: 'success', rows: 1240 },
  { name: 'tasks_snapshot', source: 'MongoDB', destination: 'Catalog', schedule: '0 * * * *', lastRun: '45 min ago', status: 'running', rows: 856 },
  { name: 'metrics_rollup', source: 'InfluxDB', destination: 'Catalog', schedule: '0 */6 * * *', lastRun: '3h ago', status: 'failed', rows: 0 },
  { name: 'embedding_refresh', source: 'PostgreSQL', destination: 'Qdrant', schedule: '0 2 * * *', lastRun: '18h ago', status: 'idle', rows: 2400 },
];

const statusVariant = (s: Pipeline['status']) => {
  switch (s) {
    case 'running': return 'default' as const;
    case 'success': return 'success' as const;
    case 'failed': return 'error' as const;
    default: return 'muted' as const;
  }
};

export function EtlPage() {
  return (
    <div>
      <div class="flex items-center justify-between mb-6">
        <div>
          <h1 class="text-2xl font-bold">ETL Pipelines</h1>
          <p class="text-sm text-muted mt-1">Ingest data from sources into the catalog</p>
        </div>
        <button
          type="button"
          class="bg-primary hover:bg-primary-hover text-white text-sm font-medium px-4 py-2 rounded-md transition-colors"
        >
          + New Pipeline
        </button>
      </div>

      <div class="grid gap-4">
        <For each={mockPipelines}>
          {(pipeline) => (
            <div class="bg-surface border border-border rounded-lg p-4 hover:border-slate-600 transition-colors cursor-pointer">
              <div class="flex items-center justify-between">
                <div class="flex items-center gap-3">
                  <h3 class="font-medium text-slate-100">{pipeline.name}</h3>
                  <Badge variant={statusVariant(pipeline.status)}>{pipeline.status}</Badge>
                </div>
                <span class="text-xs text-muted">{pipeline.lastRun}</span>
              </div>
              <div class="mt-2 flex items-center gap-2 text-sm text-muted">
                <span>{pipeline.source}</span>
                <span class="text-slate-600">\u2192</span>
                <span>{pipeline.destination}</span>
                <span class="ml-auto text-xs">{pipeline.schedule}</span>
              </div>
              <div class="mt-2 text-xs text-muted">
                {pipeline.rows > 0 ? `${pipeline.rows.toLocaleString()} rows processed` : 'No data'}
              </div>
            </div>
          )}
        </For>
      </div>
    </div>
  );
}
