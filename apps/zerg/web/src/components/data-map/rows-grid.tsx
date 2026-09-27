import { keepPreviousData, useQuery } from '@tanstack/solid-query';
import { createSignal, For, Show } from 'solid-js';
import { catalogApi } from '../../lib/api-client';
import type { CatalogTable } from '../../lib/data-map/types';
import { JsonView } from './json-view';

const PAGE_SIZE = 50;

function Cell(props: { value: unknown }) {
  const v = () => props.value;
  return (
    <Show
      when={v() !== null && v() !== undefined}
      fallback={<span class="italic text-gray-300">null</span>}
    >
      <Show
        when={typeof v() === 'object'}
        fallback={
          <span
            class={
              typeof v() === 'number'
                ? 'text-sky-700'
                : typeof v() === 'boolean'
                  ? 'text-amber-700'
                  : 'text-gray-800'
            }
          >
            {String(v())}
          </span>
        }
      >
        <span class="text-violet-700">{JSON.stringify(v())}</span>
      </Show>
    </Show>
  );
}

/** Paged live rows of one table; click a row to inspect it as JSON. */
export function RowsGrid(props: { database: string; table: CatalogTable }) {
  const [offset, setOffset] = createSignal(0);
  const [openRow, setOpenRow] = createSignal<number>();
  const query = useQuery(() => ({
    queryKey: [
      'catalog',
      'rows',
      props.database,
      props.table.schema,
      props.table.name,
      offset(),
    ] as const,
    queryFn: () =>
      catalogApi.rows(props.database, props.table.schema, props.table.name, {
        limit: PAGE_SIZE,
        offset: offset(),
      }),
    placeholderData: keepPreviousData,
    retry: false,
  }));
  const pk = () =>
    new Set(
      props.table.columns.filter((c) => c.is_primary_key).map((c) => c.name),
    );

  return (
    <div class="space-y-3">
      <Show when={query.error}>
        <div class="rounded-md border border-red-200 bg-red-50 p-3 text-sm text-red-700">
          {query.error?.message}
        </div>
      </Show>
      <Show when={query.data}>
        {(page) => (
          <>
            <div class="flex items-center justify-between text-sm text-gray-600">
              <span>
                <Show when={page().total > 0} fallback="No rows">
                  Rows {page().offset + 1}–{page().offset + page().rows.length}{' '}
                  of <span class="font-semibold">{page().total}</span>
                </Show>
                <Show when={query.isFetching}>
                  <span class="ml-2 text-gray-400">loading…</span>
                </Show>
              </span>
              <div class="flex gap-2">
                <button
                  type="button"
                  class="rounded border px-2.5 py-1 hover:bg-gray-50 disabled:opacity-40 border-gray-200"
                  disabled={offset() === 0}
                  onClick={() => {
                    setOpenRow(undefined);
                    setOffset(Math.max(0, offset() - PAGE_SIZE));
                  }}
                >
                  ← Prev
                </button>
                <button
                  type="button"
                  class="rounded border px-2.5 py-1 hover:bg-gray-50 disabled:opacity-40 border-gray-200"
                  disabled={offset() + PAGE_SIZE >= page().total}
                  onClick={() => {
                    setOpenRow(undefined);
                    setOffset(offset() + PAGE_SIZE);
                  }}
                >
                  Next →
                </button>
              </div>
            </div>
            <div class="overflow-auto rounded-lg border max-h-[60vh] border-gray-200">
              <table class="min-w-full text-xs">
                <thead class="sticky top-0 bg-gray-50">
                  <tr>
                    <For each={page().columns}>
                      {(c) => (
                        <th class="whitespace-nowrap border-b px-3 py-2 text-left font-mono font-semibold text-gray-700 border-gray-200">
                          {c}
                          <Show when={pk().has(c)}>
                            <span class="ml-1 text-yellow-600">●</span>
                          </Show>
                        </th>
                      )}
                    </For>
                  </tr>
                </thead>
                <tbody class="font-mono">
                  <For each={page().rows}>
                    {(row, i) => (
                      <>
                        <tr
                          class="cursor-pointer border-b last:border-0 hover:bg-indigo-50/50 border-gray-200"
                          classList={{ 'bg-indigo-50': openRow() === i() }}
                          onClick={() =>
                            setOpenRow(openRow() === i() ? undefined : i())
                          }
                        >
                          <For each={page().columns}>
                            {(c) => (
                              <td class="max-w-xs truncate whitespace-nowrap px-3 py-1.5">
                                <Cell value={row[c]} />
                              </td>
                            )}
                          </For>
                        </tr>
                        <Show when={openRow() === i()}>
                          <tr>
                            <td colSpan={page().columns.length} class="p-2">
                              <JsonView value={row} />
                            </td>
                          </tr>
                        </Show>
                      </>
                    )}
                  </For>
                </tbody>
              </table>
            </div>
          </>
        )}
      </Show>
    </div>
  );
}
