/** Where a dataset originated from. */
export type DataSource =
  | { type: 'csv'; path: string }
  | { type: 'json'; path: string }
  | { type: 'parquet'; path: string }
  | { type: 'postgres'; query: string }
  | { type: 'mongo'; database: string; collection: string }
  | { type: 'nats'; stream: string; subject: string }
  | { type: 'influxdb'; bucket: string; query: string }
  | { type: 'qdrant'; collection: string }
  | { type: 'transform'; pipeline: string; sources: string[] }
  | { type: 'in_memory' };

/** Column schema entry. */
export interface ColumnSchema {
  name: string;
  dtype: string;
  nullable: boolean;
}

/** Dataset metadata in the catalog. */
export interface DatasetMeta {
  name: string;
  source: DataSource;
  schema: ColumnSchema[];
  row_count: number;
  column_count: number;
  lineage: string[];
  tags: string[];
  freshness: 'fresh' | 'stale' | 'unknown';
  created_at: string;
  updated_at: string;
}

/** Request to register a new dataset. */
export interface RegisterDatasetRequest {
  name: string;
  source: DataSource;
  tags?: string[];
}
