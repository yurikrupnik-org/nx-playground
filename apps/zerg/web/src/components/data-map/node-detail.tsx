import {
  createMemo,
  createSignal,
  For,
  type JSX,
  Match,
  Show,
  Switch,
} from 'solid-js';
import { sampleFromSchema, tablePointer } from '../../lib/data-map/derive';
import type { MapNode, Source } from '../../lib/data-map/types';
import { cn } from '../../lib/utils';
import { JsonView } from './json-view';
import { KIND_META } from './kind-meta';
import { MapGraph } from './map-graph';
import { RowsGrid } from './rows-grid';
import { SchemaTree, TypeBadge } from './schema-tree';

type Tab =
  | 'columns'
  | 'data'
  | 'schema'
  | 'example'
  | 'relations'
  | 'usage'
  | 'json';

const TAB_LABELS: Record<Tab, string> = {
  columns: 'Columns',
  data: 'Data',
  schema: 'Schema',
  example: 'Example',
  relations: 'Relations',
  usage: 'Used by',
  json: 'JSON',
};

const METHOD_CLASSES: Record<string, string> = {
  GET: 'bg-sky-100 text-sky-800',
  POST: 'bg-emerald-100 text-emerald-800',
  PUT: 'bg-amber-100 text-amber-800',
  PATCH: 'bg-violet-100 text-violet-800',
  DELETE: 'bg-red-100 text-red-800',
};

function NodeChip(props: {
  node?: MapNode;
  label?: string;
  onSelect: (id: string) => void;
}) {
  return (
    <Show when={props.node}>
      {(n) => (
        <button
          type="button"
          class="inline-flex items-center gap-1.5 rounded-md border border-gray-200 bg-white px-2 py-1 font-mono text-xs text-gray-800 shadow-sm hover:border-indigo-300 hover:bg-indigo-50"
          onClick={() => props.onSelect(n().id)}
        >
          {n().label}
          <Show when={props.label}>
            <span class="text-gray-400">via {props.label}</span>
          </Show>
        </button>
      )}
    </Show>
  );
}

/** Everything about one schema node: structure, live data, links, raw JSON. */
export function NodeDetail(props: {
  node: MapNode;
  source: Source;
  onSelect: (id: string) => void;
  onBack: () => void;
}): JSX.Element {
  const nodes = createMemo(
    () => new Map(props.source.nodes.map((n) => [n.id, n])),
  );
  const outgoing = createMemo(() =>
    props.source.edges.filter((e) => e.from === props.node.id),
  );
  const incoming = createMemo(() =>
    props.source.edges.filter((e) => e.to === props.node.id),
  );
  const usage = createMemo(
    () =>
      props.source.operations?.filter((op) => op.refs.has(props.node.id)) ?? [],
  );
  const tabs = createMemo<Tab[]>(() => {
    if (props.node.table)
      return ['columns', 'data', 'schema', 'relations', 'json'];
    const t: Tab[] = ['schema', 'example', 'relations'];
    if (props.source.operations) t.push('usage');
    t.push('json');
    return t;
  });
  const [tab, setTab] = createSignal<Tab>();
  const current = () => {
    const t = tab();
    return t && tabs().includes(t) ? t : tabs()[0];
  };

  const neighbourhood = createMemo(() => {
    const ids = new Set([props.node.id]);
    for (const e of [...outgoing(), ...incoming()]) {
      ids.add(e.from);
      ids.add(e.to);
    }
    return {
      nodes: props.source.nodes.filter((n) => ids.has(n.id)),
      edges: props.source.edges.filter((e) => ids.has(e.from) && ids.has(e.to)),
    };
  });

  /** `$ref` → node of this source, by exact pointer or owning prefix. */
  const navigate = (ref: string) => {
    const target =
      props.source.nodes.find((n) => n.pointer === ref) ??
      props.source.nodes
        .filter((n) => n.pointer !== '#' && ref.startsWith(`${n.pointer}/`))
        .sort((a, b) => b.pointer.length - a.pointer.length)[0];
    if (target) props.onSelect(target.id);
  };

  const propertyCount = () =>
    Object.keys(props.node.schema.properties ?? {}).length;
  const meta = () => KIND_META[props.source.kind];

  return (
    <div class="space-y-5">
      <div>
        <nav class="flex items-center gap-1.5 text-sm text-gray-500">
          <button
            type="button"
            class="hover:text-gray-900"
            onClick={props.onBack}
          >
            {props.source.title}
          </button>
          <span>/</span>
          <span class="text-gray-900">{props.node.label}</span>
        </nav>
        <div class="mt-2 flex flex-wrap items-center gap-3">
          <span
            class={cn(
              'rounded-md px-2 py-0.5 text-xs font-semibold',
              meta().badge,
            )}
          >
            {meta().singular}
          </span>
          <h2 class="font-mono text-2xl font-bold text-gray-900">
            {props.node.label}
          </h2>
          <Show when={props.node.group}>
            <span class="rounded-full bg-gray-100 px-2 py-0.5 text-xs text-gray-600">
              {props.node.group}
            </span>
          </Show>
          <Show
            when={
              props.node.table?.info.kind !== 'table' &&
              props.node.table?.info.kind
            }
          >
            {(k) => (
              <span class="rounded-full bg-gray-100 px-2 py-0.5 text-xs text-gray-600">
                {k().replace('_', ' ')}
              </span>
            )}
          </Show>
        </div>
        <Show when={props.node.description}>
          <p class="mt-2 max-w-3xl text-sm text-gray-600">
            {props.node.description}
          </p>
        </Show>
        <dl class="mt-3 flex flex-wrap gap-x-6 gap-y-1 text-sm">
          <div class="flex gap-1.5">
            <dt class="text-gray-500">
              {props.node.table ? 'columns' : 'properties'}
            </dt>
            <dd class="font-semibold">{propertyCount()}</dd>
          </div>
          <div class="flex gap-1.5">
            <dt class="text-gray-500">required</dt>
            <dd class="font-semibold">
              {props.node.schema.required?.length ?? 0}
            </dd>
          </div>
          <Show when={props.node.table}>
            {(t) => (
              <div class="flex gap-1.5">
                <dt class="text-gray-500">rows (est.)</dt>
                <dd class="font-semibold">{t().info.row_estimate ?? '—'}</dd>
              </div>
            )}
          </Show>
          <div class="flex gap-1.5">
            <dt class="text-gray-500">references</dt>
            <dd class="font-semibold">{outgoing().length}</dd>
          </div>
          <div class="flex gap-1.5">
            <dt class="text-gray-500">referenced by</dt>
            <dd class="font-semibold">{incoming().length}</dd>
          </div>
          <Show when={props.source.operations}>
            <div class="flex gap-1.5">
              <dt class="text-gray-500">operations</dt>
              <dd class="font-semibold">{usage().length}</dd>
            </div>
          </Show>
        </dl>
      </div>

      <div class="flex gap-1 border-b border-gray-200">
        <For each={tabs()}>
          {(t) => (
            <button
              type="button"
              class={cn(
                '-mb-px border-b-2 px-3 py-2 text-sm',
                current() === t
                  ? 'border-indigo-600 font-medium text-indigo-700'
                  : 'border-transparent text-gray-500 hover:text-gray-800',
              )}
              onClick={() => setTab(t)}
            >
              {TAB_LABELS[t]}
            </button>
          )}
        </For>
      </div>

      <Switch>
        <Match when={current() === 'columns' && props.node.table}>
          {(t) => (
            <div class="overflow-auto rounded-lg border border-gray-200">
              <table class="min-w-full text-sm">
                <thead class="bg-gray-50 text-left text-xs uppercase tracking-wide text-gray-500">
                  <tr>
                    <th class="px-3 py-2">Column</th>
                    <th class="px-3 py-2">Type</th>
                    <th class="px-3 py-2">Null</th>
                    <th class="px-3 py-2">Default</th>
                    <th class="px-3 py-2">References / notes</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={t().info.columns}>
                    {(col) => {
                      const fk = t().info.foreign_keys.find((f) =>
                        f.columns.includes(col.name),
                      );
                      const fkNode = fk
                        ? nodes().get(
                            `${props.source.id}::${tablePointer(fk.ref_schema, fk.ref_table)}`,
                          )
                        : undefined;
                      return (
                        <tr class="border-t align-top border-gray-200">
                          <td class="whitespace-nowrap px-3 py-2 font-mono font-medium">
                            {col.name}
                            <Show when={col.is_primary_key}>
                              <span class="ml-1.5 rounded bg-yellow-100 px-1 text-[10px] font-semibold text-yellow-800">
                                PK
                              </span>
                            </Show>
                          </td>
                          <td class="whitespace-nowrap px-3 py-2 font-mono text-xs text-gray-700">
                            {col.data_type}
                            <Show when={col.enum_values}>
                              {(values) => (
                                <div class="mt-1 flex max-w-xs flex-wrap gap-1">
                                  <For each={values()}>
                                    {(v) => (
                                      <span class="rounded-full bg-teal-50 px-1.5 text-[10px] text-teal-700">
                                        {v}
                                      </span>
                                    )}
                                  </For>
                                </div>
                              )}
                            </Show>
                          </td>
                          <td class="px-3 py-2 text-xs">
                            {col.nullable ? (
                              <span class="text-gray-400">yes</span>
                            ) : (
                              <span class="font-semibold text-gray-700">
                                no
                              </span>
                            )}
                          </td>
                          <td class="max-w-[14rem] truncate px-3 py-2 font-mono text-xs text-gray-500">
                            {col.default ?? ''}
                          </td>
                          <td class="px-3 py-2 text-xs text-gray-600">
                            <Show when={fk}>
                              <Show
                                when={fkNode}
                                fallback={
                                  <span class="font-mono">
                                    → {fk?.ref_schema}.{fk?.ref_table}
                                  </span>
                                }
                              >
                                <NodeChip
                                  node={fkNode}
                                  label={fk?.ref_columns.join(', ')}
                                  onSelect={props.onSelect}
                                />
                              </Show>
                            </Show>
                            <Show when={col.comment}>
                              <p class="mt-0.5">{col.comment}</p>
                            </Show>
                          </td>
                        </tr>
                      );
                    }}
                  </For>
                </tbody>
              </table>
            </div>
          )}
        </Match>
        <Match when={current() === 'data' && props.node.table}>
          {(t) => <RowsGrid database={t().database} table={t().info} />}
        </Match>
        <Match when={current() === 'schema'}>
          <div class="rounded-lg border bg-white p-4 border-gray-200">
            <div class="mb-2 flex flex-wrap items-center gap-2 text-xs text-gray-500">
              <span>Legend:</span>
              <For
                each={[
                  'string',
                  'integer',
                  'boolean',
                  'object',
                  'array',
                  'null',
                ]}
              >
                {(t) => <TypeBadge type={t} />}
              </For>
              <span class="text-red-500">* required</span>
              <span class="text-indigo-700">→ ref (click to open)</span>
            </div>
            <SchemaTree
              schema={props.node.schema}
              root={props.node.root}
              onNavigate={navigate}
            />
          </div>
        </Match>
        <Match when={current() === 'example'}>
          <div class="space-y-2">
            <p class="text-sm text-gray-500">
              Generated from the schema: examples and defaults first, then the
              first enum value, then a placeholder per type.
            </p>
            <JsonView
              value={sampleFromSchema(props.node.schema, props.node.root)}
            />
          </div>
        </Match>
        <Match when={current() === 'relations'}>
          <div class="grid gap-4 lg:grid-cols-[1fr_1.4fr]">
            <div class="space-y-4">
              <section>
                <h3 class="mb-2 text-sm font-semibold text-gray-700">
                  References ({outgoing().length})
                </h3>
                <div class="flex flex-wrap gap-2">
                  <For
                    each={outgoing()}
                    fallback={<span class="text-sm text-gray-400">none</span>}
                  >
                    {(e) => (
                      <NodeChip
                        node={nodes().get(e.to)}
                        label={e.label}
                        onSelect={props.onSelect}
                      />
                    )}
                  </For>
                </div>
              </section>
              <section>
                <h3 class="mb-2 text-sm font-semibold text-gray-700">
                  Referenced by ({incoming().length})
                </h3>
                <div class="flex flex-wrap gap-2">
                  <For
                    each={incoming()}
                    fallback={<span class="text-sm text-gray-400">none</span>}
                  >
                    {(e) => (
                      <NodeChip
                        node={nodes().get(e.from)}
                        label={e.label}
                        onSelect={props.onSelect}
                      />
                    )}
                  </For>
                </div>
              </section>
            </div>
            <div class="h-80">
              <MapGraph
                nodes={neighbourhood().nodes}
                edges={neighbourhood().edges}
                selected={props.node.id}
                onSelect={props.onSelect}
              />
            </div>
          </div>
        </Match>
        <Match when={current() === 'usage'}>
          <div class="divide-y rounded-lg border border-gray-200 divide-gray-200">
            <For
              each={usage()}
              fallback={
                <p class="p-4 text-sm text-gray-400">
                  No operation reaches this schema.
                </p>
              }
            >
              {(op) => (
                <div class="flex items-center gap-3 px-3 py-2 text-sm">
                  <span
                    class={cn(
                      'w-16 rounded px-1.5 py-0.5 text-center font-mono text-[11px] font-semibold',
                      METHOD_CLASSES[op.method] ?? 'bg-gray-100 text-gray-700',
                    )}
                  >
                    {op.method}
                  </span>
                  <span class="font-mono text-gray-800">{op.path}</span>
                  <Show when={op.summary}>
                    <span class="truncate text-gray-500">{op.summary}</span>
                  </Show>
                </div>
              )}
            </For>
          </div>
        </Match>
        <Match when={current() === 'json'}>
          <JsonView value={props.node.schema} />
        </Match>
      </Switch>
    </div>
  );
}
