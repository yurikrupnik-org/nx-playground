import { For } from 'solid-js';
import { Badge } from '../../components/ui/badge';

interface Issue {
  id: string;
  title: string;
  dataset: string;
  severity: 'critical' | 'warning' | 'info';
  type: string;
  detected: string;
  assignee: string | null;
}

const mockIssues: Issue[] = [
  { id: 'ISS-001', title: 'Null values in customers.email', dataset: 'customers', severity: 'critical', type: 'Quality', detected: '1h ago', assignee: null },
  { id: 'ISS-002', title: 'metrics_rollup pipeline failed', dataset: 'metrics_1h', severity: 'critical', type: 'Pipeline', detected: '3h ago', assignee: null },
  { id: 'ISS-003', title: 'Schema drift detected in tasks', dataset: 'tasks', severity: 'warning', type: 'Schema', detected: '45 min ago', assignee: null },
  { id: 'ISS-004', title: 'tasks freshness SLA breached', dataset: 'tasks', severity: 'warning', type: 'Freshness', detected: '45 min ago', assignee: null },
  { id: 'ISS-005', title: 'reverse-etl user_segments sync error', dataset: 'active_users', severity: 'critical', type: 'Sync', detected: '1h ago', assignee: null },
];

const severityVariant = (s: Issue['severity']) =>
  s === 'critical' ? 'error' as const : s === 'warning' ? 'warning' as const : 'muted' as const;

export function IssuesPage() {
  const openCount = () => mockIssues.length;
  const criticalCount = () => mockIssues.filter((i) => i.severity === 'critical').length;

  return (
    <div>
      <div class="flex items-center justify-between mb-6">
        <div>
          <h1 class="text-2xl font-bold">Issues</h1>
          <p class="text-sm text-muted mt-1">
            {openCount()} open issues, {criticalCount()} critical
          </p>
        </div>
      </div>

      <div class="space-y-2">
        <For each={mockIssues}>
          {(issue) => (
            <div class="bg-surface border border-border rounded-lg p-4 hover:border-slate-600 transition-colors cursor-pointer">
              <div class="flex items-center justify-between">
                <div class="flex items-center gap-3">
                  <span class="text-xs text-muted font-mono">{issue.id}</span>
                  <h3 class="font-medium text-slate-100">{issue.title}</h3>
                </div>
                <Badge variant={severityVariant(issue.severity)}>{issue.severity}</Badge>
              </div>
              <div class="mt-2 flex items-center gap-4 text-xs text-muted">
                <span>Dataset: <span class="text-primary">{issue.dataset}</span></span>
                <span>Type: {issue.type}</span>
                <span>Detected: {issue.detected}</span>
                <span>{issue.assignee ?? 'Unassigned'}</span>
              </div>
            </div>
          )}
        </For>
      </div>
    </div>
  );
}
