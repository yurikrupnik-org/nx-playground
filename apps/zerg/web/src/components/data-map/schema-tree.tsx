import { createSignal, For, type JSX, Show } from 'solid-js';
import {
  refName,
  resolvePointer,
  schemaTypes,
} from '../../lib/data-map/derive';
import type { JsonSchema } from '../../lib/data-map/types';
import { cn } from '../../lib/utils';

const TYPE_CLASSES: Record<string, string> = {
  string: 'bg-emerald-50 text-emerald-700 ring-emerald-200',
  integer: 'bg-sky-50 text-sky-700 ring-sky-200',
  number: 'bg-sky-50 text-sky-700 ring-sky-200',
  boolean: 'bg-amber-50 text-amber-700 ring-amber-200',
  object: 'bg-violet-50 text-violet-700 ring-violet-200',
  array: 'bg-pink-50 text-pink-700 ring-pink-200',
  enum: 'bg-teal-50 text-teal-700 ring-teal-200',
  null: 'bg-gray-50 text-gray-500 ring-gray-200',
};

const COMBINATORS = [
  ['allOf', 'all of'],
  ['oneOf', 'one of'],
  ['anyOf', 'any of'],
] as const;

type Child = { name: string; schema: JsonSchema; required?: boolean };
type Group = { label?: string; children: Child[] };

export function TypeBadge(props: { type: string }) {
  return (
    <span
      class={cn(
        'inline-flex items-center rounded px-1.5 py-0.5 font-mono text-[11px] ring-1 ring-inset',
        TYPE_CLASSES[props.type] ?? 'bg-gray-50 text-gray-600 ring-gray-200',
      )}
    >
      {props.type}
    </span>
  );
}

const isSchema = (v: unknown): v is JsonSchema =>
  typeof v === 'object' && v !== null && !Array.isArray(v);

/** Nested members of a schema, grouped (properties, items, combinators…). */
function childGroups(s: JsonSchema): Group[] {
  const groups: Group[] = [];
  if (s.properties) {
    const required = new Set(s.required ?? []);
    groups.push({
      children: Object.entries(s.properties).map(([name, schema]) => ({
        name,
        schema,
        required: required.has(name),
      })),
    });
  }
  const items = Array.isArray(s.items) ? s.items : s.items ? [s.items] : [];
  if (items.length) {
    groups.push({
      label: 'items',
      children: items.map((schema, i) => ({
        name: items.length > 1 ? `[${i}]` : '[ ]',
        schema,
      })),
    });
  }
  if (isSchema(s.additionalProperties)) {
    groups.push({
      label: 'additional properties',
      children: [{ name: '{key}', schema: s.additionalProperties }],
    });
  }
  for (const [key, label] of COMBINATORS) {
    const variants = s[key];
    if (!Array.isArray(variants) || !variants.length) continue;
    groups.push({
      label,
      children: variants.map((schema, i) => ({
        name:
          schema.$ref !== undefined
            ? refName(schema.$ref)
            : (schema.title ?? `variant ${i + 1}`),
        schema,
      })),
    });
  }
  return groups;
}

function constraints(s: JsonSchema): string[] {
  const out: string[] = [];
  const add = (key: string, label: string) => {
    if (s[key] !== undefined) out.push(`${label} ${JSON.stringify(s[key])}`);
  };
  add('minLength', 'minLen');
  add('maxLength', 'maxLen');
  add('minimum', '≥');
  add('exclusiveMinimum', '>');
  add('maximum', '≤');
  add('exclusiveMaximum', '<');
  add('minItems', 'minItems');
  add('maxItems', 'maxItems');
  add('pattern', 'pattern');
  add('default', 'default');
  add('x-pg-default', 'default');
  if (s.uniqueItems) out.push('unique');
  if (s.readOnly) out.push('read-only');
  if (s.writeOnly) out.push('write-only');
  if (s.additionalProperties === false) out.push('closed');
  return out;
}

interface NodeProps {
  name?: string;
  schema: JsonSchema;
  root: unknown;
  required?: boolean;
  depth: number;
  /** `$ref`s already expanded above this node — stops recursive schemas. */
  refChain: string[];
  onNavigate?: (ref: string) => void;
}

function SchemaNode(props: NodeProps) {
  const ref = () => props.schema.$ref;
  const resolved = () => {
    const r = ref();
    if (!r) return undefined;
    const target = resolvePointer(props.root, r);
    return isSchema(target) ? target : undefined;
  };
  const cyclic = () => {
    const r = ref();
    return !!r && props.refChain.includes(r);
  };
  // What to expand into: the ref target (once per chain) or the schema itself.
  const body = () =>
    ref() ? (cyclic() ? undefined : resolved()) : props.schema;
  const groups = () => {
    const b = body();
    return b ? childGroups(b) : [];
  };
  const expandable = () => groups().length > 0;
  const [open, setOpen] = createSignal(props.depth < 2 && !props.schema.$ref);

  const shown = () => resolved() ?? props.schema;
  const types = () => (ref() ? [] : schemaTypes(props.schema));
  const description = () => props.schema.description ?? resolved()?.description;
  const enumValues = () => shown().enum;

  return (
    <div class={cn(props.depth > 0 && 'border-l border-gray-100 ml-2 pl-3')}>
      <div class="group flex flex-wrap items-center gap-1.5 py-1">
        <button
          type="button"
          class={cn(
            'w-4 h-4 flex items-center justify-center text-gray-400 hover:text-gray-700 transition-transform',
            !expandable() && 'invisible',
            open() && 'rotate-90',
          )}
          aria-label={open() ? 'Collapse' : 'Expand'}
          onClick={() => setOpen(!open())}
        >
          ▸
        </button>
        <Show when={props.name}>
          <span class="font-mono text-sm font-medium text-gray-900">
            {props.name}
          </span>
        </Show>
        <Show when={props.required}>
          <span class="text-red-500 text-xs" title="required">
            *
          </span>
        </Show>
        <For each={types()}>{(t) => <TypeBadge type={t} />}</For>
        <Show when={ref()}>
          {(r) => (
            <button
              type="button"
              class="inline-flex items-center gap-1 rounded bg-indigo-50 px-1.5 py-0.5 font-mono text-[11px] text-indigo-700 ring-1 ring-inset ring-indigo-200 hover:bg-indigo-100"
              title={`Go to ${r()}`}
              onClick={() => props.onNavigate?.(r())}
            >
              → {refName(r())}
              <Show when={cyclic()}>
                <span class="text-indigo-400">(recursive)</span>
              </Show>
            </button>
          )}
        </Show>
        <Show when={shown().format}>
          <span class="font-mono text-[11px] text-gray-500">
            ({String(shown().format)})
          </span>
        </Show>
        <Show when={props.schema['x-primary-key']}>
          <span class="rounded bg-yellow-100 px-1.5 py-0.5 text-[11px] font-semibold text-yellow-800">
            PK
          </span>
        </Show>
        <Show when={props.schema['x-references']}>
          {(target) => (
            <span class="rounded bg-orange-50 px-1.5 py-0.5 font-mono text-[11px] text-orange-700 ring-1 ring-inset ring-orange-200">
              FK → {String(target())}
            </span>
          )}
        </Show>
        <Show when={props.schema['x-pg-type']}>
          {(pg) => (
            <span class="font-mono text-[11px] text-gray-400">
              {String(pg())}
            </span>
          )}
        </Show>
        <Show when={props.schema.deprecated}>
          <span class="rounded bg-gray-100 px-1.5 text-[11px] text-gray-500 line-through">
            deprecated
          </span>
        </Show>
        <For each={constraints(shown())}>
          {(c) => (
            <span class="rounded bg-gray-50 px-1.5 py-0.5 font-mono text-[11px] text-gray-500">
              {c}
            </span>
          )}
        </For>
      </div>
      <Show when={description()}>
        <p class="ml-6 -mt-0.5 mb-1 text-xs text-gray-500 max-w-3xl">
          {description()}
        </p>
      </Show>
      <Show when={enumValues()?.length}>
        <div class="ml-6 mb-1 flex flex-wrap gap-1">
          <For each={enumValues()}>
            {(v) => (
              <span class="rounded-full bg-teal-50 px-2 py-0.5 font-mono text-[11px] text-teal-700">
                {JSON.stringify(v)}
              </span>
            )}
          </For>
        </div>
      </Show>
      <Show when={open() && expandable()}>
        <For each={groups()}>
          {(group) => (
            <div class="ml-2">
              <Show when={group.label}>
                <div class="ml-4 mt-1 text-[11px] uppercase tracking-wide text-gray-400">
                  {group.label}
                </div>
              </Show>
              <For each={group.children}>
                {(child) => (
                  <SchemaNode
                    name={child.name}
                    schema={child.schema}
                    root={props.root}
                    required={child.required}
                    depth={props.depth + 1}
                    refChain={
                      ref()
                        ? [...props.refChain, ref() as string]
                        : props.refChain
                    }
                    onNavigate={props.onNavigate}
                  />
                )}
              </For>
            </div>
          )}
        </For>
      </Show>
    </div>
  );
}

/** Collapsible JSON Schema tree; `$ref`s expand inline and navigate on click. */
export function SchemaTree(props: {
  schema: JsonSchema;
  root: unknown;
  onNavigate?: (ref: string) => void;
}): JSX.Element {
  return (
    <div class="text-sm">
      <SchemaNode
        schema={props.schema}
        root={props.root}
        depth={0}
        refChain={[]}
        onNavigate={props.onNavigate}
      />
    </div>
  );
}
