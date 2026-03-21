import { For } from 'solid-js';
import { Badge } from '../../components/ui/badge';

interface Design {
  name: string;
  description: string;
  steps: number;
  lastEdited: string;
  status: 'draft' | 'published' | 'archived';
}

const mockDesigns: Design[] = [
  { name: 'revenue_by_category', description: 'Sales CSV -> filter completed -> revenue by category', steps: 4, lastEdited: '2h ago', status: 'published' },
  { name: 'gold_customer_sales', description: 'Sales + Customers join -> gold tier filter -> revenue', steps: 6, lastEdited: '2h ago', status: 'published' },
  { name: 'user_enrichment', description: 'PG active_users -> join CRM data -> push to Mongo', steps: 5, lastEdited: '1d ago', status: 'draft' },
  { name: 'anomaly_detection', description: 'InfluxDB metrics -> threshold filter -> NATS alerts', steps: 3, lastEdited: '3d ago', status: 'draft' },
];

const statusVariant = (s: Design['status']) =>
  s === 'published' ? 'success' as const : s === 'draft' ? 'default' as const : 'muted' as const;

export function DesignsPage() {
  return (
    <div>
      <div class="flex items-center justify-between mb-6">
        <div>
          <h1 class="text-2xl font-bold">Designs</h1>
          <p class="text-sm text-muted mt-1">Visual pipeline builder and DAG designer</p>
        </div>
        <button
          type="button"
          class="bg-primary hover:bg-primary-hover text-white text-sm font-medium px-4 py-2 rounded-md transition-colors"
        >
          + New Design
        </button>
      </div>

      <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
        <For each={mockDesigns}>
          {(design) => (
            <div class="bg-surface border border-border rounded-lg p-4 hover:border-slate-600 transition-colors cursor-pointer">
              <div class="flex items-center justify-between mb-2">
                <h3 class="font-medium text-slate-100">{design.name}</h3>
                <Badge variant={statusVariant(design.status)}>{design.status}</Badge>
              </div>
              <p class="text-sm text-muted mb-3">{design.description}</p>
              <div class="flex items-center justify-between text-xs text-muted">
                <span>{design.steps} steps</span>
                <span>Edited {design.lastEdited}</span>
              </div>
              <div class="mt-3 h-16 bg-surface-raised rounded border border-border flex items-center justify-center text-xs text-muted">
                Pipeline DAG preview
              </div>
            </div>
          )}
        </For>
      </div>
    </div>
  );
}
