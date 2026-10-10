import { useQuery } from '@tanstack/solid-query';
import { Link } from '@tanstack/solid-router';
import { createMemo, createSignal, For, Show } from 'solid-js';
import {
  AddDocument,
  type StoredDocument,
} from '../components/data-map/add-document';
import { KIND_META } from '../components/data-map/kind-meta';
import { MapGraph } from '../components/data-map/map-graph';
import { NodeDetail } from '../components/data-map/node-detail';
import { CatalogError, catalogApi } from '../lib/api-client';
import { databaseSource, documentSource } from '../lib/data-map/derive';
import {
  openApiSources,
  platformSources,
} from '../lib/data-map/static-sources';
import type { MapNode, Source, SourceKind } from '../lib/data-map/types';
import { cn } from '../lib/utils';

const STORAGE_KEY = 'zerg-web:data-map:documents';

function loadDocuments(): StoredDocument[] {
  try {
    const parsed: unknown = JSON.parse(
      localStorage.getItem(STORAGE_KEY) ?? '[]',
    );
    return Array.isArray(parsed) ? (parsed as StoredDocument[]) : [];
  } catch {
    return [];
  }
}

/** Label, description and property names — what a search should hit. */
function matches(node: MapNode, query: string) {
  if (!query) return true;
  const hay = [
    node.label,
    node.description ?? '',
    ...Object.keys(node.schema.properties ?? {}),
  ]
    .join(' ')
    .toLowerCase();
  return hay.includes(query);
}

// Repo documents never change at runtime: parse them once per page load.
const STATIC_SOURCES = {
  openapi: openApiSources(),
  platform: platformSources(),
};

export function DataMapPage() {
  const catalog = useQuery(() => ({
    queryKey: ['catalog', 'databases'] as const,
    queryFn: catalogApi.databases,
    retry: false,
    refetchOnWindowFocus: false,
  }));
  const [documents, setDocuments] = createSignal(loadDocuments());
  const persist = (docs: StoredDocument[]) => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(docs));
    setDocuments(docs);
  };

  const sections = createMemo<{ kind: SourceKind; sources: Source[] }[]>(() => [
    {
      kind: 'database',
      sources: catalog.data?.databases.map(databaseSource) ?? [],
    },
    { kind: 'openapi', sources: STATIC_SOURCES.openapi },
    { kind: 'platform', sources: STATIC_SOURCES.platform },
    {
      kind: 'custom',
      sources: documents().map((d) => {
        try {
          return documentSource(d.id, d.name, d.text);
        } catch (err) {
          return {
            id: d.id,
            kind: 'custom' as const,
            title: d.name,
            nodes: [],
            edges: [],
            error: err instanceof Error ? err.message : String(err),
          };
        }
      }),
    },
  ]);
  const sourceById = createMemo(
    () =>
      new Map(sections().flatMap((s) => s.sources.map((src) => [src.id, src]))),
  );

  const [selectedSource, setSelectedSource] = createSignal<string>();
  const [selectedNode, setSelectedNode] = createSignal<string>();
  const [expanded, setExpanded] = createSignal(new Set<string>());
  const [search, setSearch] = createSignal('');
  const [adding, setAdding] = createSignal(false);
  const query = () => search().trim().toLowerCase();

  const source = () => {
    const id = selectedSource();
    return id ? sourceById().get(id) : undefined;
  };
  const node = () => source()?.nodes.find((n) => n.id === selectedNode());

  const openSource = (id: string) => {
    setSelectedSource(id);
    setSelectedNode(undefined);
    setExpanded((s) => new Set(s).add(id));
  };
  const openNode = (sourceId: string, nodeId: string) => {
    setSelectedSource(sourceId);
    setSelectedNode(nodeId);
    setExpanded((s) => new Set(s).add(sourceId));
  };
  const toggle = (id: string) =>
    setExpanded((s) => {
      const next = new Set(s);
      if (!next.delete(id)) next.add(id);
      return next;
    });

  const totals = createMemo(() => {
    const all = sections().flatMap((s) => s.sources);
    return {
      sources: all.length,
      nodes: all.reduce((n, s) => n + s.nodes.length, 0),
      edges: all.reduce((n, s) => n + s.edges.length, 0),
    };
  });

  const catalogError = () =>
    catalog.error instanceof CatalogError ? catalog.error : undefined;

  return (
    <div class="flex h-[calc(100vh-4rem)] bg-gray-50">
      {/* ----------------------------------------------------------- sidebar */}
      <aside class="flex w-80 shrink-0 flex-col border-r bg-white border-gray-200">
        <div class="border-b p-3 border-gray-200">
          <input
            type="search"
            class="w-full rounded-md border border-gray-200 px-3 py-1.5 text-sm focus:border-indigo-400 focus:outline-none"
            placeholder="Search tables, models, fields…"
            value={search()}
            onInput={(e) => setSearch(e.currentTarget.value)}
          />
        </div>
        <div class="flex-1 overflow-y-auto p-2 text-sm">
          <For each={sections()}>
            {(section) => (
              <section
                class="mb-3"
                classList={{
                  hidden:
                    !!query() &&
                    !section.sources.some((s) =>
                      s.nodes.some((n) => matches(n, query())),
                    ),
                }}
              >
                <div class="flex items-center justify-between px-2 py-1">
                  <h3 class="flex items-center gap-2 text-[11px] font-semibold uppercase tracking-wider text-gray-500">
                    <span
                      class={cn(
                        'h-2 w-2 rounded-full',
                        KIND_META[section.kind].dot,
                      )}
                    />
                    {KIND_META[section.kind].label}
                  </h3>
                  <Show when={section.kind === 'custom'}>
                    <button
                      type="button"
                      class="rounded px-1.5 text-xs text-indigo-600 hover:bg-indigo-50"
                      onClick={() => setAdding(true)}
                    >
                      + Add
                    </button>
                  </Show>
                  <Show when={section.kind === 'database' && catalog.isFetched}>
                    <button
                      type="button"
                      class="rounded px-1.5 text-xs text-gray-500 hover:bg-gray-100"
                      onClick={() => catalog.refetch()}
                    >
                      ↻
                    </button>
                  </Show>
                </div>
                <Show when={section.kind === 'database'}>
                  <Show when={catalog.isLoading}>
                    <p class="px-2 py-1 text-xs text-gray-400">
                      Loading catalog…
                    </p>
                  </Show>
                  <Show when={catalogError()}>
                    {(err) => (
                      <div class="mx-2 rounded-md bg-amber-50 p-2 text-xs text-amber-800">
                        {err().message}
                        <Show when={err().reason === 'unauthenticated'}>
                          {' '}
                          <Link to="/login" class="font-medium underline">
                            Sign in
                          </Link>
                        </Show>
                      </div>
                    )}
                  </Show>
                </Show>
                <Show
                  when={section.kind === 'custom' && !section.sources.length}
                >
                  <p class="px-2 py-1 text-xs text-gray-400">
                    Paste or drop any JSON Schema, OpenAPI or CRD.
                  </p>
                </Show>
                <For each={section.sources}>
                  {(src) => {
                    const visible = () =>
                      src.nodes.filter((n) => matches(n, query()));
                    const open = () => !!query() || expanded().has(src.id);
                    return (
                      <Show when={!query() || visible().length}>
                        <div>
                          <div
                            class={cn(
                              'group flex items-center gap-1 rounded-md px-1 py-1',
                              selectedSource() === src.id && !selectedNode()
                                ? 'bg-indigo-50 text-indigo-800'
                                : 'hover:bg-gray-50',
                            )}
                          >
                            <button
                              type="button"
                              class={cn(
                                'w-4 text-gray-400 transition-transform',
                                open() && 'rotate-90',
                              )}
                              aria-label="Toggle"
                              onClick={() => toggle(src.id)}
                            >
                              ▸
                            </button>
                            <button
                              type="button"
                              class="flex flex-1 items-baseline gap-2 truncate text-left"
                              onClick={() => openSource(src.id)}
                            >
                              <span class="truncate font-medium">
                                {src.title}
                              </span>
                              <span class="text-xs text-gray-400">
                                {src.nodes.length}
                              </span>
                            </button>
                            <Show when={src.error}>
                              <span
                                class="text-xs text-red-500"
                                title={src.error}
                              >
                                !
                              </span>
                            </Show>
                            <Show when={section.kind === 'custom'}>
                              <button
                                type="button"
                                class="invisible px-1 text-xs text-gray-400 hover:text-red-600 group-hover:visible"
                                aria-label={`Remove ${src.title}`}
                                onClick={() => {
                                  persist(
                                    documents().filter((d) => d.id !== src.id),
                                  );
                                  if (selectedSource() === src.id)
                                    setSelectedSource(undefined);
                                }}
                              >
                                ✕
                              </button>
                            </Show>
                          </div>
                          <Show when={open()}>
                            <ul class="ml-5 border-l pl-2 border-gray-200">
                              <For each={visible()}>
                                {(n) => (
                                  <li>
                                    <button
                                      type="button"
                                      class={cn(
                                        'flex w-full items-center justify-between gap-2 rounded px-2 py-0.5 text-left font-mono text-xs',
                                        selectedNode() === n.id
                                          ? 'bg-indigo-600 text-white'
                                          : 'text-gray-700 hover:bg-gray-100',
                                      )}
                                      onClick={() => openNode(src.id, n.id)}
                                    >
                                      <span class="truncate">{n.label}</span>
                                      <Show
                                        when={
                                          n.table?.info.row_estimate != null
                                        }
                                      >
                                        <span
                                          class={cn(
                                            'text-[10px]',
                                            selectedNode() === n.id
                                              ? 'text-indigo-100'
                                              : 'text-gray-400',
                                          )}
                                        >
                                          {n.table?.info.row_estimate}
                                        </span>
                                      </Show>
                                    </button>
                                  </li>
                                )}
                              </For>
                            </ul>
                          </Show>
                        </div>
                      </Show>
                    );
                  }}
                </For>
              </section>
            )}
          </For>
        </div>
      </aside>

      {/* -------------------------------------------------------------- main */}
      <main class="flex-1 overflow-y-auto p-6">
        <Show
          when={source()}
          fallback={
            <Overview
              sections={sections()}
              totals={totals()}
              onOpenSource={openSource}
            />
          }
        >
          {(src) => (
            <Show
              when={node()}
              fallback={
                <SourceView
                  source={src()}
                  onSelect={(id) => openNode(src().id, id)}
                  onBack={() => setSelectedSource(undefined)}
                />
              }
            >
              {(n) => (
                <NodeDetail
                  node={n()}
                  source={src()}
                  onSelect={(id) => openNode(src().id, id)}
                  onBack={() => setSelectedNode(undefined)}
                />
              )}
            </Show>
          )}
        </Show>
      </main>

      <Show when={adding()}>
        <AddDocument
          onClose={() => setAdding(false)}
          onAdd={(doc) => {
            persist([...documents(), doc]);
            setAdding(false);
            openSource(doc.id);
          }}
        />
      </Show>
    </div>
  );
}

function Overview(props: {
  sections: { kind: SourceKind; sources: Source[] }[];
  totals: { sources: number; nodes: number; edges: number };
  onOpenSource: (id: string) => void;
}) {
  return (
    <div class="mx-auto max-w-6xl space-y-8">
      <div>
        <h2 class="text-2xl font-bold">Data map</h2>
        <p class="mt-1 text-sm text-gray-600">
          Every schema in the platform in one place: live Postgres tables, the
          OpenAPI models each service publishes, and the Crossplane resources
          the platform offers.{' '}
          <span class="font-medium text-gray-800">
            {props.totals.nodes} schemas · {props.totals.edges} relationships ·{' '}
            {props.totals.sources} sources
          </span>
        </p>
      </div>
      <For each={props.sections.filter((s) => s.sources.length)}>
        {(section) => (
          <section>
            <h3 class="mb-3 flex items-center gap-2 text-sm font-semibold text-gray-700">
              <span
                class={cn('h-2 w-2 rounded-full', KIND_META[section.kind].dot)}
              />
              {KIND_META[section.kind].label}
            </h3>
            <div class="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
              <For each={section.sources}>
                {(src) => (
                  <button
                    type="button"
                    class="rounded-xl border border-gray-200 bg-white p-4 text-left shadow-sm transition hover:-translate-y-0.5 hover:border-indigo-300 hover:shadow-md"
                    onClick={() => props.onOpenSource(src.id)}
                  >
                    <div class="flex items-start justify-between gap-2">
                      <span class="font-semibold text-gray-900">
                        {src.title}
                      </span>
                      <span
                        class={cn(
                          'rounded px-1.5 py-0.5 text-[10px] font-semibold',
                          KIND_META[src.kind].badge,
                        )}
                      >
                        {KIND_META[src.kind].singular}s
                      </span>
                    </div>
                    <Show when={src.subtitle}>
                      <p class="mt-0.5 truncate font-mono text-xs text-gray-400">
                        {src.subtitle}
                      </p>
                    </Show>
                    <Show
                      when={!src.error}
                      fallback={
                        <p class="mt-2 text-xs text-red-600">{src.error}</p>
                      }
                    >
                      <p class="mt-3 text-sm text-gray-600">
                        <span class="font-semibold text-gray-900">
                          {src.nodes.length}
                        </span>{' '}
                        schemas ·{' '}
                        <span class="font-semibold text-gray-900">
                          {src.edges.length}
                        </span>{' '}
                        links
                      </p>
                      <p class="mt-1 truncate text-xs text-gray-400">
                        {src.nodes
                          .slice(0, 6)
                          .map((n) => n.label)
                          .join(', ')}
                        {src.nodes.length > 6 ? ', …' : ''}
                      </p>
                    </Show>
                  </button>
                )}
              </For>
            </div>
          </section>
        )}
      </For>
    </div>
  );
}

function SourceView(props: {
  source: Source;
  onSelect: (id: string) => void;
  onBack: () => void;
}) {
  const [view, setView] = createSignal<'map' | 'cards'>('map');
  const degree = createMemo(() => {
    const d = new Map<string, number>();
    for (const e of props.source.edges) {
      d.set(e.from, (d.get(e.from) ?? 0) + 1);
      d.set(e.to, (d.get(e.to) ?? 0) + 1);
    }
    return d;
  });

  return (
    <div class="flex h-full flex-col gap-4">
      <div class="flex flex-wrap items-end justify-between gap-3">
        <div>
          <button
            type="button"
            class="text-sm text-gray-500 hover:text-gray-900"
            onClick={props.onBack}
          >
            ← All sources
          </button>
          <div class="mt-1 flex items-center gap-3">
            <h2 class="text-2xl font-bold">{props.source.title}</h2>
            <span
              class={cn(
                'rounded px-2 py-0.5 text-xs font-semibold',
                KIND_META[props.source.kind].badge,
              )}
            >
              {KIND_META[props.source.kind].label}
            </span>
          </div>
          <p class="mt-1 text-sm text-gray-500">
            <Show when={props.source.subtitle}>
              <span class="font-mono">{props.source.subtitle}</span> ·{' '}
            </Show>
            {props.source.nodes.length} schemas · {props.source.edges.length}{' '}
            relationships
            <Show when={props.source.operations}>
              {(ops) => <> · {ops().length} operations</>}
            </Show>
          </p>
        </div>
        <div class="flex overflow-hidden rounded-md border bg-white text-sm border-gray-200">
          <For each={['map', 'cards'] as const}>
            {(v) => (
              <button
                type="button"
                class={cn(
                  'px-3 py-1.5 capitalize',
                  view() === v ? 'bg-gray-900 text-white' : 'hover:bg-gray-50',
                )}
                onClick={() => setView(v)}
              >
                {v}
              </button>
            )}
          </For>
        </div>
      </div>
      <Show when={props.source.error}>
        <div class="rounded-md border border-red-200 bg-red-50 p-3 text-sm text-red-700">
          {props.source.error}
        </div>
      </Show>
      <Show
        when={props.source.nodes.length}
        fallback={
          <p class="text-sm text-gray-500">No schemas in this source.</p>
        }
      >
        <Show
          when={view() === 'map'}
          fallback={
            <div class="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
              <For each={props.source.nodes}>
                {(n) => (
                  <button
                    type="button"
                    class="rounded-lg border border-gray-200 bg-white p-3 text-left shadow-sm hover:border-indigo-300"
                    onClick={() => props.onSelect(n.id)}
                  >
                    <div class="flex items-center justify-between gap-2">
                      <span class="truncate font-mono text-sm font-semibold">
                        {n.label}
                      </span>
                      <span class="text-xs text-gray-400">
                        {degree().get(n.id) ?? 0} links
                      </span>
                    </div>
                    <Show when={n.description}>
                      <p class="mt-1 line-clamp-2 text-xs text-gray-500">
                        {n.description}
                      </p>
                    </Show>
                    <p class="mt-2 truncate font-mono text-[11px] text-gray-400">
                      {Object.keys(n.schema.properties ?? {}).join(' · ')}
                    </p>
                  </button>
                )}
              </For>
            </div>
          }
        >
          <div class="min-h-[28rem] flex-1">
            <MapGraph
              nodes={props.source.nodes}
              edges={props.source.edges}
              onSelect={props.onSelect}
            />
          </div>
          <p class="text-xs text-gray-400">
            Scroll to zoom, drag to pan, hover to highlight neighbours, click to
            open.
          </p>
        </Show>
      </Show>
    </div>
  );
}
