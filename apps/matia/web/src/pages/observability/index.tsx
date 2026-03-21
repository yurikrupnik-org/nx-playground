import { For } from 'solid-js';
import { Badge } from '../../components/ui/badge';

interface QualityCheck {
  dataset: string;
  check: string;
  status: 'pass' | 'warn' | 'fail';
  value: string;
  threshold: string;
  lastChecked: string;
}

const mockChecks: QualityCheck[] = [
  { dataset: 'sales', check: 'Not null: order_id', status: 'pass', value: '0 nulls', threshold: '0', lastChecked: '2 min ago' },
  { dataset: 'sales', check: 'Unique: order_id', status: 'pass', value: '30 unique / 30 rows', threshold: '100%', lastChecked: '2 min ago' },
  { dataset: 'sales', check: 'Range: unit_price', status: 'pass', value: '9.99 - 199.99', threshold: '0 - 10000', lastChecked: '2 min ago' },
  { dataset: 'active_users', check: 'Freshness', status: 'pass', value: '12 min ago', threshold: '< 1h', lastChecked: '12 min ago' },
  { dataset: 'tasks', check: 'Freshness', status: 'warn', value: '45 min ago', threshold: '< 30 min', lastChecked: '45 min ago' },
  { dataset: 'metrics_1h', check: 'Row count', status: 'fail', value: '0 rows', threshold: '> 0', lastChecked: '3h ago' },
  { dataset: 'tasks', check: 'Schema drift', status: 'warn', value: '+1 column (priority)', threshold: 'No changes', lastChecked: '45 min ago' },
  { dataset: 'customers', check: 'Not null: email', status: 'fail', value: '3 nulls', threshold: '0', lastChecked: '1h ago' },
];

const statusVariant = (s: QualityCheck['status']) =>
  s === 'pass' ? 'success' as const : s === 'warn' ? 'warning' as const : 'error' as const;

export function ObservabilityPage() {
  const passCount = () => mockChecks.filter((c) => c.status === 'pass').length;
  const warnCount = () => mockChecks.filter((c) => c.status === 'warn').length;
  const failCount = () => mockChecks.filter((c) => c.status === 'fail').length;

  return (
    <div>
      <div class="mb-6">
        <h1 class="text-2xl font-bold">Observability</h1>
        <p class="text-sm text-muted mt-1">Data quality, freshness, and schema drift detection</p>
      </div>

      <div class="grid grid-cols-3 gap-4 mb-6">
        <div class="bg-surface border border-border rounded-lg p-4 text-center">
          <p class="text-3xl font-bold text-emerald-400">{passCount()}</p>
          <p class="text-xs text-muted mt-1">Passing</p>
        </div>
        <div class="bg-surface border border-border rounded-lg p-4 text-center">
          <p class="text-3xl font-bold text-amber-400">{warnCount()}</p>
          <p class="text-xs text-muted mt-1">Warnings</p>
        </div>
        <div class="bg-surface border border-border rounded-lg p-4 text-center">
          <p class="text-3xl font-bold text-red-400">{failCount()}</p>
          <p class="text-xs text-muted mt-1">Failing</p>
        </div>
      </div>

      <div class="bg-surface border border-border rounded-lg overflow-hidden">
        <table class="w-full text-sm">
          <thead>
            <tr class="border-b border-border text-left text-muted">
              <th class="px-4 py-3 font-medium">Dataset</th>
              <th class="px-4 py-3 font-medium">Check</th>
              <th class="px-4 py-3 font-medium">Status</th>
              <th class="px-4 py-3 font-medium">Value</th>
              <th class="px-4 py-3 font-medium">Threshold</th>
              <th class="px-4 py-3 font-medium">Last Checked</th>
            </tr>
          </thead>
          <tbody>
            <For each={mockChecks}>
              {(check) => (
                <tr class="border-b border-border hover:bg-surface-raised transition-colors">
                  <td class="px-4 py-3 font-medium text-slate-100">{check.dataset}</td>
                  <td class="px-4 py-3 text-muted">{check.check}</td>
                  <td class="px-4 py-3">
                    <Badge variant={statusVariant(check.status)}>{check.status}</Badge>
                  </td>
                  <td class="px-4 py-3 text-muted">{check.value}</td>
                  <td class="px-4 py-3 text-muted">{check.threshold}</td>
                  <td class="px-4 py-3 text-xs text-muted">{check.lastChecked}</td>
                </tr>
              )}
            </For>
          </tbody>
        </table>
      </div>
    </div>
  );
}
