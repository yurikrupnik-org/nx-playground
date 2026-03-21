import { For } from 'solid-js';
import { Badge } from '../../components/ui/badge';

interface Sync {
  name: string;
  source: string;
  destination: string;
  schedule: string;
  lastSync: string;
  status: 'active' | 'paused' | 'error';
  records: number;
}

const mockSyncs: Sync[] = [
  { name: 'daily_summary_pg', source: 'revenue_by_category', destination: 'PostgreSQL (analytics.daily_summary)', schedule: '0 6 * * *', lastSync: '18h ago', status: 'active', records: 4 },
  { name: 'anomaly_alerts', source: 'metrics_rollup', destination: 'NATS (alerts.anomalies)', schedule: 'Real-time', lastSync: '5 min ago', status: 'active', records: 12 },
  { name: 'task_enrichment', source: 'enriched_tasks', destination: 'MongoDB (tasks_enriched)', schedule: '*/30 * * * *', lastSync: '25 min ago', status: 'paused', records: 856 },
  { name: 'user_segments', source: 'active_users', destination: 'Dapr pub/sub (db.tasks.pg)', schedule: '0 * * * *', lastSync: '1h ago', status: 'error', records: 0 },
];

const statusVariant = (s: Sync['status']) =>
  s === 'active' ? 'success' as const : s === 'error' ? 'error' as const : 'muted' as const;

export function ReverseEtlPage() {
  return (
    <div>
      <div class="flex items-center justify-between mb-6">
        <div>
          <h1 class="text-2xl font-bold">Reverse ETL</h1>
          <p class="text-sm text-muted mt-1">Push processed data back to operational systems</p>
        </div>
        <button
          type="button"
          class="bg-primary hover:bg-primary-hover text-white text-sm font-medium px-4 py-2 rounded-md transition-colors"
        >
          + New Sync
        </button>
      </div>

      <div class="grid gap-4">
        <For each={mockSyncs}>
          {(sync) => (
            <div class="bg-surface border border-border rounded-lg p-4 hover:border-slate-600 transition-colors cursor-pointer">
              <div class="flex items-center justify-between">
                <div class="flex items-center gap-3">
                  <h3 class="font-medium text-slate-100">{sync.name}</h3>
                  <Badge variant={statusVariant(sync.status)}>{sync.status}</Badge>
                </div>
                <span class="text-xs text-muted">{sync.lastSync}</span>
              </div>
              <div class="mt-2 flex items-center gap-2 text-sm text-muted">
                <span class="text-primary">{sync.source}</span>
                <span class="text-slate-600">\u2192</span>
                <span>{sync.destination}</span>
              </div>
              <div class="mt-2 flex items-center justify-between text-xs text-muted">
                <span>{sync.schedule}</span>
                <span>{sync.records > 0 ? `${sync.records.toLocaleString()} records synced` : 'Sync failed'}</span>
              </div>
            </div>
          )}
        </For>
      </div>
    </div>
  );
}
