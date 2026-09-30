// Shapes shared by every data-map source. The catalog types mirror the
// zerg-api `/api/catalog` contract (snake_case JSON, dev-only routes).

/** A JSON Schema / OpenAPI schema object. Kept loose on purpose: the viewer
 *  reads draft-07, 2020-12, OpenAPI 3.0 (`nullable`) and 3.1 dialects. */
export type JsonSchema = {
  [key: string]: unknown;
  $ref?: string;
  type?: string | string[];
  title?: string;
  description?: string;
  format?: string;
  enum?: unknown[];
  const?: unknown;
  default?: unknown;
  examples?: unknown[];
  example?: unknown;
  nullable?: boolean;
  properties?: Record<string, JsonSchema>;
  required?: string[];
  items?: JsonSchema | JsonSchema[];
  additionalProperties?: boolean | JsonSchema;
  allOf?: JsonSchema[];
  anyOf?: JsonSchema[];
  oneOf?: JsonSchema[];
};

export interface CatalogColumn {
  name: string;
  position: number;
  data_type: string;
  nullable: boolean;
  default: string | null;
  is_primary_key: boolean;
  comment: string | null;
  enum_values: string[] | null;
}

export interface CatalogForeignKey {
  name: string;
  columns: string[];
  ref_schema: string;
  ref_table: string;
  ref_columns: string[];
}

export type CatalogTableKind =
  | 'table'
  | 'view'
  | 'materialized_view'
  | 'partitioned_table'
  | 'foreign_table';

export interface CatalogTable {
  schema: string;
  name: string;
  kind: CatalogTableKind;
  comment: string | null;
  row_estimate: number | null;
  columns: CatalogColumn[];
  foreign_keys: CatalogForeignKey[];
}

export interface CatalogDatabase {
  name: string;
  current: boolean;
  error: string | null;
  tables: CatalogTable[];
}

export interface CatalogResponse {
  databases: CatalogDatabase[];
}

export interface RowsPage {
  columns: string[];
  rows: Record<string, unknown>[];
  total: number;
  limit: number;
  offset: number;
}

export type SourceKind = 'database' | 'openapi' | 'platform' | 'custom';

/** One OpenAPI operation that references a schema (directly or nested). */
export interface Operation {
  method: string;
  path: string;
  summary?: string;
  refs: Set<string>;
}

/** One browsable schema: a table, an OpenAPI component, an XRD version, a
 *  JSON Schema definition. `root` is the document `$ref`s resolve against. */
export interface MapNode {
  /** Unique across all sources: `<sourceId>::<pointer>`. */
  id: string;
  sourceId: string;
  label: string;
  /** Secondary grouping inside a source (pg schema, XRD version, …). */
  group?: string;
  /** JSON pointer of `schema` inside `root` (`#/components/schemas/Task`). */
  pointer: string;
  schema: JsonSchema;
  root: unknown;
  description?: string;
  table?: { database: string; info: CatalogTable };
}

export interface MapEdge {
  from: string;
  to: string;
  label?: string;
}

export interface Source {
  id: string;
  kind: SourceKind;
  title: string;
  subtitle?: string;
  nodes: MapNode[];
  edges: MapEdge[];
  operations?: Operation[];
  error?: string;
}
