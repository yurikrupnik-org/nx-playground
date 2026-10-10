// Pure transforms from raw documents (pg catalog, OpenAPI, CRD/XRD, JSON
// Schema) into the browsable Source/MapNode/MapEdge model. No I/O here.

import { parseAllDocuments } from 'yaml';
import type {
  CatalogDatabase,
  CatalogTable,
  JsonSchema,
  MapEdge,
  MapNode,
  Operation,
  Source,
} from './types';

type Obj = Record<string, unknown>;

const isObj = (v: unknown): v is Obj =>
  typeof v === 'object' && v !== null && !Array.isArray(v);

export const nodeId = (sourceId: string, pointer: string) =>
  `${sourceId}::${pointer}`;

const escapePointer = (s: string) => s.replace(/~/g, '~0').replace(/\//g, '~1');
const unescapePointer = (s: string) =>
  s.replace(/~1/g, '/').replace(/~0/g, '~');

/** Resolve a local `#/a/b` JSON pointer against `root`; external refs → undefined. */
export function resolvePointer(root: unknown, ref: string): unknown {
  if (!ref.startsWith('#')) return undefined;
  const path = ref.slice(1);
  if (path === '' || path === '/') return root;
  let cur: unknown = root;
  for (const raw of path.replace(/^\//, '').split('/')) {
    const key = unescapePointer(decodeURIComponent(raw));
    if (Array.isArray(cur)) cur = cur[Number(key)];
    else if (isObj(cur)) cur = cur[key];
    else return undefined;
  }
  return cur;
}

/** Last pointer segment, the human name of a `$ref` target. */
export const refName = (ref: string) =>
  unescapePointer(ref.split('/').pop() ?? ref);

/** Every `$ref` string anywhere inside `value`. */
export function collectRefs(value: unknown, out = new Set<string>()) {
  if (Array.isArray(value)) {
    for (const v of value) collectRefs(v, out);
  } else if (isObj(value)) {
    for (const [k, v] of Object.entries(value)) {
      if (k === '$ref' && typeof v === 'string') out.add(v);
      else collectRefs(v, out);
    }
  }
  return out;
}

/** Map a `$ref` to the node owning it: exact pointer, else the longest node
 *  pointer that prefixes it (`#/$defs/A/properties/b` → `#/$defs/A`). */
function refTarget(ref: string, byPointer: Map<string, MapNode>) {
  const exact = byPointer.get(ref);
  if (exact) return exact;
  let best: MapNode | undefined;
  for (const [pointer, node] of byPointer) {
    if (
      pointer !== '#' &&
      ref.startsWith(`${pointer}/`) &&
      (!best || pointer.length > best.pointer.length)
    ) {
      best = node;
    }
  }
  return best ?? byPointer.get('#');
}

/** Directed `$ref` edges between nodes of one source (self-refs dropped). */
export function refEdges(nodes: MapNode[]): MapEdge[] {
  const byPointer = new Map(nodes.map((n) => [n.pointer, n]));
  const edges = new Map<string, MapEdge>();
  for (const node of nodes) {
    // A root node (`#`) would otherwise re-collect every nested definition.
    const { $defs: _d, definitions: _defs, ...ownSchema } = node.schema;
    const scope = node.pointer === '#' ? ownSchema : node.schema;
    for (const ref of collectRefs(scope)) {
      const target = refTarget(ref, byPointer);
      if (!target || target.id === node.id) continue;
      edges.set(`${node.id}->${target.id}`, { from: node.id, to: target.id });
    }
  }
  return [...edges.values()];
}

// ---------------------------------------------------------------------------
// Postgres
// ---------------------------------------------------------------------------

/** JSON Schema for one pg type, as printed by `format_type()`. */
export function pgTypeToSchema(
  dataType: string,
  enumValues: string[] | null,
): JsonSchema {
  if (dataType.endsWith('[]')) {
    return {
      type: 'array',
      items: pgTypeToSchema(dataType.slice(0, -2), enumValues),
    };
  }
  if (enumValues) return { type: 'string', enum: enumValues };
  const t = dataType.toLowerCase();
  const len = /\((\d+)\)$/.exec(t);
  const base = t.replace(/\(.*\)$/, '').trim();
  switch (base) {
    case 'smallint':
    case 'integer':
      return { type: 'integer', format: 'int32' };
    case 'bigint':
      return { type: 'integer', format: 'int64' };
    case 'numeric':
    case 'decimal':
    case 'real':
    case 'double precision':
    case 'money':
      return { type: 'number' };
    case 'boolean':
      return { type: 'boolean' };
    case 'uuid':
      return { type: 'string', format: 'uuid' };
    case 'date':
      return { type: 'string', format: 'date' };
    case 'timestamp with time zone':
    case 'timestamp without time zone':
      return { type: 'string', format: 'date-time' };
    case 'time with time zone':
    case 'time without time zone':
      return { type: 'string', format: 'time' };
    case 'interval':
      return { type: 'string', format: 'duration' };
    case 'json':
    case 'jsonb':
      return { description: dataType };
    case 'bytea':
      return { type: 'string', contentEncoding: 'base64' };
    case 'character varying':
    case 'character':
    case 'text':
    case 'citext':
      return len
        ? { type: 'string', maxLength: Number(len[1]) }
        : { type: 'string' };
    case 'inet':
    case 'cidr':
      return { type: 'string', format: 'ip' };
    default:
      return { type: 'string', 'x-pg-type': dataType };
  }
}

/** A row of `table` as JSON Schema: nullable → `[t, "null"]`, required =
 *  NOT NULL without default, keys/FKs as `x-` annotations. */
export function tableToJsonSchema(table: CatalogTable): JsonSchema {
  const fkByColumn = new Map<string, string>();
  for (const fk of table.foreign_keys) {
    for (const [i, col] of fk.columns.entries()) {
      fkByColumn.set(
        col,
        `${fk.ref_schema}.${fk.ref_table}.${fk.ref_columns[i] ?? '?'}`,
      );
    }
  }
  const properties: Record<string, JsonSchema> = {};
  const required: string[] = [];
  for (const col of table.columns) {
    const s: JsonSchema = { ...pgTypeToSchema(col.data_type, col.enum_values) };
    if (col.nullable) {
      if (typeof s.type === 'string') s.type = [s.type, 'null'];
      else if (!s.type) s.nullable = true;
    }
    if (col.comment) s.description = col.comment;
    if (col.default !== null) s['x-pg-default'] = col.default;
    if (col.is_primary_key) s['x-primary-key'] = true;
    const fk = fkByColumn.get(col.name);
    if (fk) s['x-references'] = fk;
    s['x-pg-type'] = col.data_type;
    properties[col.name] = s;
    if (!col.nullable && col.default === null) required.push(col.name);
  }
  return {
    title: `${table.schema}.${table.name}`,
    description: table.comment ?? undefined,
    type: 'object',
    properties,
    required,
  };
}

export const tablePointer = (schema: string, table: string) =>
  `#/${escapePointer(schema)}/${escapePointer(table)}`;

export function databaseSource(db: CatalogDatabase): Source {
  const id = `db:${db.name}`;
  const nodes: MapNode[] = db.tables.map((t) => {
    const pointer = tablePointer(t.schema, t.name);
    const schema = tableToJsonSchema(t);
    return {
      id: nodeId(id, pointer),
      sourceId: id,
      label: t.schema === 'public' ? t.name : `${t.schema}.${t.name}`,
      group: t.schema,
      pointer,
      schema,
      root: schema,
      description: t.comment ?? undefined,
      table: { database: db.name, info: t },
    };
  });
  const known = new Set(nodes.map((n) => n.id));
  const edges: MapEdge[] = [];
  for (const t of db.tables) {
    for (const fk of t.foreign_keys) {
      const from = nodeId(id, tablePointer(t.schema, t.name));
      const to = nodeId(id, tablePointer(fk.ref_schema, fk.ref_table));
      if (from !== to && known.has(to)) {
        edges.push({ from, to, label: fk.columns.join(', ') });
      }
    }
  }
  return {
    id,
    kind: 'database',
    title: db.name,
    subtitle: db.current ? 'zerg-api connection' : undefined,
    nodes,
    edges,
    error: db.error ?? undefined,
  };
}

// ---------------------------------------------------------------------------
// OpenAPI
// ---------------------------------------------------------------------------

const METHODS = ['get', 'put', 'post', 'delete', 'patch', 'options', 'head'];

export const isOpenApi = (doc: unknown): doc is Obj =>
  isObj(doc) && (typeof doc.openapi === 'string' || doc.swagger === '2.0');

export function openApiSource(id: string, doc: Obj, fallback: string): Source {
  const info = isObj(doc.info) ? doc.info : {};
  const components = isObj(doc.components) ? doc.components : {};
  // OpenAPI 3.x keeps models in components.schemas; Swagger 2 in definitions.
  const [base, schemas] = isObj(components.schemas)
    ? ['#/components/schemas', components.schemas]
    : ['#/definitions', isObj(doc.definitions) ? doc.definitions : {}];
  const nodes: MapNode[] = Object.entries(schemas)
    .filter(([, s]) => isObj(s))
    .map(([name, s]) => {
      const pointer = `${base}/${escapePointer(name)}`;
      const schema = s as JsonSchema;
      return {
        id: nodeId(id, pointer),
        sourceId: id,
        label: name,
        pointer,
        schema,
        root: doc,
        description:
          typeof schema.description === 'string'
            ? schema.description
            : undefined,
      };
    });
  const edges = refEdges(nodes);
  return {
    id,
    kind: 'openapi',
    title: typeof info.title === 'string' ? info.title : fallback,
    subtitle: typeof info.version === 'string' ? `v${info.version}` : undefined,
    nodes,
    edges,
    operations: operations(doc, nodes, edges),
  };
}

/** Operations with the set of node ids they reach, transitively via `$ref`. */
function operations(doc: Obj, nodes: MapNode[], edges: MapEdge[]) {
  const byPointer = new Map(nodes.map((n) => [n.pointer, n]));
  const out = new Map<string, string[]>();
  for (const e of edges) out.set(e.from, [...(out.get(e.from) ?? []), e.to]);
  const paths = isObj(doc.paths) ? doc.paths : {};
  const ops: Operation[] = [];
  for (const [path, item] of Object.entries(paths)) {
    if (!isObj(item)) continue;
    for (const method of METHODS) {
      const op = item[method];
      if (!isObj(op)) continue;
      const reached = new Set<string>();
      const stack = [...collectRefs(op)]
        .map((r) => refTarget(r, byPointer)?.id)
        .filter((x): x is string => !!x);
      while (stack.length) {
        const cur = stack.pop() as string;
        if (reached.has(cur)) continue;
        reached.add(cur);
        stack.push(...(out.get(cur) ?? []));
      }
      ops.push({
        method: method.toUpperCase(),
        path,
        summary:
          typeof op.summary === 'string'
            ? op.summary
            : typeof op.operationId === 'string'
              ? op.operationId
              : undefined,
        refs: reached,
      });
    }
  }
  return ops;
}

// ---------------------------------------------------------------------------
// Kubernetes CRD / Crossplane XRD
// ---------------------------------------------------------------------------

const CRD_KINDS: Record<string, true> = {
  CustomResourceDefinition: true,
  CompositeResourceDefinition: true,
};

export const isCrd = (doc: unknown): doc is Obj =>
  isObj(doc) && typeof doc.kind === 'string' && CRD_KINDS[doc.kind] === true;

/** One node per served version, schema = `openAPIV3Schema`. */
export function crdSource(id: string, docs: Obj[], fallback: string): Source {
  const nodes: MapNode[] = [];
  let title = fallback;
  let subtitle: string | undefined;
  docs.forEach((doc, docIndex) => {
    const spec = isObj(doc.spec) ? doc.spec : {};
    const names = isObj(spec.names) ? spec.names : {};
    const claim = isObj(spec.claimNames) ? spec.claimNames : undefined;
    const kind = typeof names.kind === 'string' ? names.kind : String(doc.kind);
    if (docIndex === 0) {
      title = claim && typeof claim.kind === 'string' ? claim.kind : kind;
      subtitle = typeof spec.group === 'string' ? spec.group : undefined;
    }
    const versions = Array.isArray(spec.versions) ? spec.versions : [];
    versions.forEach((v, i) => {
      if (!isObj(v)) return;
      const s = isObj(v.schema) ? v.schema.openAPIV3Schema : undefined;
      if (!isObj(s)) return;
      const pointer = `#/${docIndex}/spec/versions/${i}/schema/openAPIV3Schema`;
      const version = typeof v.name === 'string' ? v.name : `v${i}`;
      nodes.push({
        id: nodeId(id, pointer),
        sourceId: id,
        label: `${kind} ${version}`,
        group: [
          doc.kind === 'CompositeResourceDefinition' ? 'XRD' : 'CRD',
          v.served === false ? 'not served' : null,
          v.referenceable ? 'referenceable' : null,
        ]
          .filter(Boolean)
          .join(' · '),
        pointer,
        schema: s as JsonSchema,
        root: s,
        description: claim
          ? `Composite ${kind}; namespaced claim ${String(claim.kind)}`
          : undefined,
      });
    });
  });
  return { id, kind: 'platform', title, subtitle, nodes, edges: [] };
}

// ---------------------------------------------------------------------------
// Arbitrary documents (JSON or YAML, pasted or dropped)
// ---------------------------------------------------------------------------

/** Parse JSON first (strict), else every YAML document in the text. */
export function parseDocuments(text: string): unknown[] {
  try {
    return [JSON.parse(text)];
  } catch {
    const docs = parseAllDocuments(text);
    const list = Array.isArray(docs) ? docs : [docs];
    for (const d of list) {
      if (d.errors.length) throw new Error(d.errors[0].message);
    }
    return list.map((d) => d.toJS()).filter((d) => d != null);
  }
}

/** Detect OpenAPI / CRD / plain JSON Schema and build the matching source. */
export function documentSource(id: string, name: string, text: string): Source {
  const docs = parseDocuments(text);
  const first = docs[0];
  if (!first) throw new Error('document is empty');
  if (isOpenApi(first)) return openApiSource(id, first, name);
  const crds = docs.filter(isCrd);
  if (crds.length) return crdSource(id, crds, name);
  if (!isObj(first)) throw new Error('expected a JSON Schema object');
  return { ...jsonSchemaSource(id, name, first as JsonSchema), kind: 'custom' };
}

/** Root node plus one node per `$defs` / `definitions` entry. */
export function jsonSchemaSource(
  id: string,
  name: string,
  doc: JsonSchema,
): Source {
  const nodes: MapNode[] = [
    {
      id: nodeId(id, '#'),
      sourceId: id,
      label: typeof doc.title === 'string' ? doc.title : name,
      group: 'root',
      pointer: '#',
      schema: doc,
      root: doc,
      description:
        typeof doc.description === 'string' ? doc.description : undefined,
    },
  ];
  for (const key of ['$defs', 'definitions']) {
    const defs = doc[key];
    if (!isObj(defs)) continue;
    for (const [defName, s] of Object.entries(defs)) {
      if (!isObj(s)) continue;
      const pointer = `#/${key}/${escapePointer(defName)}`;
      nodes.push({
        id: nodeId(id, pointer),
        sourceId: id,
        label: defName,
        group: key,
        pointer,
        schema: s as JsonSchema,
        root: doc,
        description:
          typeof s.description === 'string' ? s.description : undefined,
      });
    }
  }
  return {
    id,
    kind: 'custom',
    title: name,
    nodes,
    edges: refEdges(nodes),
  };
}

// ---------------------------------------------------------------------------
// Schema helpers used by the viewer
// ---------------------------------------------------------------------------

/** Declared types, folding OpenAPI 3.0 `nullable` into `"null"`. */
export function schemaTypes(s: JsonSchema): string[] {
  const types = Array.isArray(s.type)
    ? [...s.type]
    : typeof s.type === 'string'
      ? [s.type]
      : [];
  if (!types.length) {
    if (s.properties) types.push('object');
    else if (s.items) types.push('array');
    else if (s.enum) types.push('enum');
  }
  if (s.nullable === true && !types.includes('null')) types.push('null');
  return types;
}

/** A plausible instance of `schema`: example → default → const → enum → by type. */
export function sampleFromSchema(
  schema: JsonSchema,
  root: unknown,
  seen: Set<string> = new Set(),
  depth = 0,
): unknown {
  if (depth > 8) return null;
  if (schema.$ref) {
    if (seen.has(schema.$ref)) return {};
    const target = resolvePointer(root, schema.$ref);
    if (!isObj(target)) return null;
    return sampleFromSchema(
      target as JsonSchema,
      root,
      new Set([...seen, schema.$ref]),
      depth + 1,
    );
  }
  if (schema.example !== undefined) return schema.example;
  if (Array.isArray(schema.examples) && schema.examples.length)
    return schema.examples[0];
  // An object default (often `{}`) would hide the documented properties;
  // it is merged over the per-property sample below instead.
  if (
    schema.default !== undefined &&
    !(isObj(schema.default) && schema.properties)
  )
    return schema.default;
  if (schema.const !== undefined) return schema.const;
  if (Array.isArray(schema.enum) && schema.enum.length) return schema.enum[0];
  if (schema.allOf?.length) {
    const merged: Obj = {};
    for (const part of schema.allOf) {
      const v = sampleFromSchema(part, root, seen, depth + 1);
      if (!isObj(v)) return v;
      Object.assign(merged, v);
    }
    return merged;
  }
  const alt = schema.oneOf?.[0] ?? schema.anyOf?.[0];
  if (alt) return sampleFromSchema(alt, root, seen, depth + 1);
  const type = schemaTypes(schema).find((t) => t !== 'null');
  switch (type) {
    case 'object': {
      const out: Obj = {};
      for (const [k, v] of Object.entries(schema.properties ?? {})) {
        out[k] = sampleFromSchema(v, root, seen, depth + 1);
      }
      return isObj(schema.default) ? { ...out, ...schema.default } : out;
    }
    case 'array': {
      const items = Array.isArray(schema.items)
        ? schema.items[0]
        : schema.items;
      return items ? [sampleFromSchema(items, root, seen, depth + 1)] : [];
    }
    case 'string':
      return sampleString(schema.format);
    case 'integer':
    case 'number':
      return typeof schema.minimum === 'number' ? schema.minimum : 0;
    case 'boolean':
      return true;
    case 'null':
      return null;
    default:
      return null;
  }
}

function sampleString(format: string | undefined): string {
  switch (format) {
    case 'uuid':
      return '00000000-0000-0000-0000-000000000000';
    case 'date-time':
      return '2026-01-01T00:00:00Z';
    case 'date':
      return '2026-01-01';
    case 'time':
      return '00:00:00';
    case 'email':
      return 'user@example.com';
    case 'uri':
    case 'url':
      return 'https://example.com';
    default:
      return 'string';
  }
}
