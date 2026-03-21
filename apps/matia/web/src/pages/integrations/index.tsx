import { createMemo, createSignal, For, Show } from 'solid-js';
import { Badge } from '../../components/ui/badge';
import {
  CONNECTOR_CATALOG,
  USE_CASE_LABELS,
  type ConnectorCatalogEntry,
  type ConnectorDirection,
  type ConnectorUseCase,
} from '@matia/types';

type DirectionFilter = 'all' | 'source' | 'destination';

const directionBadgeVariant = (d: ConnectorDirection) =>
  d === 'source' ? 'default' as const : d === 'sink' ? 'warning' as const : 'success' as const;

const directionLabel = (d: ConnectorDirection) =>
  d === 'source' ? 'source' : d === 'sink' ? 'destination' : 'source + destination';

function matchesDirection(entry: ConnectorCatalogEntry, filter: DirectionFilter): boolean {
  if (filter === 'all') return true;
  if (filter === 'source') return entry.direction === 'source' || entry.direction === 'both';
  return entry.direction === 'sink' || entry.direction === 'both';
}

const useCaseKeys = Object.keys(USE_CASE_LABELS) as ConnectorUseCase[];

export function IntegrationsPage() {
  const [search, setSearch] = createSignal('');
  const [directionFilter, setDirectionFilter] = createSignal<DirectionFilter>('all');
  const [useCaseFilter, setUseCaseFilter] = createSignal<ConnectorUseCase | 'all'>('all');

  const filtered = createMemo(() => {
    const q = search().toLowerCase();
    const dir = directionFilter();
    const uc = useCaseFilter();

    return CONNECTOR_CATALOG.filter((c) => {
      if (q && !c.name.toLowerCase().includes(q)) return false;
      if (!matchesDirection(c, dir)) return false;
      if (uc !== 'all' && c.useCase !== uc) return false;
      return true;
    });
  });

  const counts = createMemo(() => {
    const q = search().toLowerCase();
    const uc = useCaseFilter();
    const base = CONNECTOR_CATALOG.filter((c) => {
      if (q && !c.name.toLowerCase().includes(q)) return false;
      if (uc !== 'all' && c.useCase !== uc) return false;
      return true;
    });
    return {
      all: base.length,
      source: base.filter((c) => c.direction === 'source' || c.direction === 'both').length,
      destination: base.filter((c) => c.direction === 'sink' || c.direction === 'both').length,
    };
  });

  const clearFilters = () => {
    setSearch('');
    setDirectionFilter('all');
    setUseCaseFilter('all');
  };

  const hasActiveFilters = createMemo(
    () => search() !== '' || directionFilter() !== 'all' || useCaseFilter() !== 'all',
  );

  return (
    <div>
      <div class="flex items-center justify-between mb-6">
        <div>
          <h1 class="text-2xl font-bold">All Connectors</h1>
          <p class="text-sm text-muted mt-1">
            Browse {CONNECTOR_CATALOG.length} available source and destination connectors
          </p>
        </div>
      </div>

      {/* Search */}
      <div class="mb-4">
        <input
          type="text"
          placeholder="Search connectors..."
          value={search()}
          onInput={(e) => setSearch(e.currentTarget.value)}
          class="w-full max-w-md bg-surface border border-border rounded-md px-3 py-2 text-sm text-slate-100 placeholder:text-slate-500 focus:outline-none focus:border-primary"
        />
      </div>

      {/* Filters */}
      <div class="flex flex-wrap items-center gap-4 mb-6">
        {/* Direction filter */}
        <div class="flex items-center gap-2">
          <span class="text-sm text-muted">Connector Type</span>
          <div class="flex gap-1">
            <FilterButton
              active={directionFilter() === 'all'}
              onClick={() => setDirectionFilter('all')}
            >
              All ({counts().all})
            </FilterButton>
            <FilterButton
              active={directionFilter() === 'source'}
              onClick={() => setDirectionFilter('source')}
            >
              Source ({counts().source})
            </FilterButton>
            <FilterButton
              active={directionFilter() === 'destination'}
              onClick={() => setDirectionFilter('destination')}
            >
              Destination ({counts().destination})
            </FilterButton>
          </div>
        </div>

        {/* Use case filter */}
        <div class="flex items-center gap-2">
          <span class="text-sm text-muted">Use Case</span>
          <select
            value={useCaseFilter()}
            onChange={(e) => setUseCaseFilter(e.currentTarget.value as ConnectorUseCase | 'all')}
            class="bg-surface border border-border rounded-md px-2 py-1.5 text-sm text-slate-100 focus:outline-none focus:border-primary"
          >
            <option value="all">All</option>
            <For each={useCaseKeys}>
              {(key) => <option value={key}>{USE_CASE_LABELS[key]}</option>}
            </For>
          </select>
        </div>

        <Show when={hasActiveFilters()}>
          <button
            type="button"
            onClick={clearFilters}
            class="text-sm text-slate-400 hover:text-slate-200 underline"
          >
            Clear all
          </button>
        </Show>
      </div>

      {/* Results count */}
      <p class="text-sm text-muted mb-4">
        Showing {filtered().length} connector{filtered().length !== 1 ? 's' : ''}
      </p>

      {/* Connector grid */}
      <div class="grid grid-cols-1 sm:grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-4">
        <For each={filtered()}>
          {(connector) => (
            <div class="bg-surface border border-border rounded-lg p-4 hover:border-slate-600 transition-colors cursor-pointer">
              <h3 class="font-medium text-slate-100 mb-2">{connector.name}</h3>
              <div class="flex flex-wrap gap-1">
                <Show when={connector.direction === 'source' || connector.direction === 'both'}>
                  <Badge variant={directionBadgeVariant('source')}>source</Badge>
                </Show>
                <Show when={connector.direction === 'sink' || connector.direction === 'both'}>
                  <Badge variant={directionBadgeVariant('sink')}>destination</Badge>
                </Show>
              </div>
              <p class="text-xs text-slate-500 mt-2">
                {USE_CASE_LABELS[connector.useCase]}
              </p>
            </div>
          )}
        </For>
      </div>

      <Show when={filtered().length === 0}>
        <div class="text-center py-12 text-slate-400">
          No connectors match your filters.
        </div>
      </Show>
    </div>
  );
}

function FilterButton(props: { active: boolean; onClick: () => void; children: any }) {
  return (
    <button
      type="button"
      onClick={props.onClick}
      class={`px-3 py-1.5 text-sm rounded-md transition-colors ${
        props.active
          ? 'bg-primary text-white'
          : 'bg-surface border border-border text-slate-400 hover:text-slate-200'
      }`}
    >
      {props.children}
    </button>
  );
}
