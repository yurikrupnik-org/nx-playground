/** Where pipeline output is written. */
export type OutputTarget =
  | { type: 'catalog'; name: string }
  | { type: 'postgres'; table: string }
  | { type: 'mongo'; database: string; collection: string }
  | { type: 'nats'; topic: string }
  | { type: 'parquet'; path: string }
  | { type: 'bigquery'; table_id: string }
  | { type: 'bigtable'; table: string; column_family: string }
  | { type: 's3'; bucket: string; prefix: string; format: ExportFormat };

export type ExportFormat = 'parquet' | 'csv' | 'json';

/** A single transform step in a pipeline. */
export type PipelineStep =
  | { op: 'filter'; expr: string }
  | { op: 'select'; columns: string[] }
  | { op: 'join'; dataset: string; left_on: string; right_on: string }
  | { op: 'group_by'; column: string; agg: string }
  | { op: 'sort'; columns: string[]; descending: boolean }
  | { op: 'limit'; n: number }
  | { op: 'add_column'; name: string; expr: string };

export type PipelineStatus = 'idle' | 'running' | 'success' | 'failed';

/** Pipeline definition. */
export interface Pipeline {
  id: string;
  name: string;
  sources: string[];
  steps: PipelineStep[];
  output: OutputTarget;
  schedule: string | null;
  status: PipelineStatus;
  last_run: string | null;
  rows_processed: number;
  created_at: string;
  updated_at: string;
}

/** Request to create/run a pipeline. */
export interface CreatePipelineRequest {
  name: string;
  sources: string[];
  steps: PipelineStep[];
  output: OutputTarget;
  schedule?: string;
}

/** Pipeline run result. */
export interface PipelineRunResult {
  pipeline_id: string;
  status: PipelineStatus;
  rows_processed: number;
  duration_ms: number;
  error: string | null;
  started_at: string;
  finished_at: string | null;
}
