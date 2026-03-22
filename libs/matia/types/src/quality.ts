/** Types of data quality checks. */
export type QualityCheckType =
  | { check: 'not_null'; columns: string[] }
  | { check: 'unique'; column: string }
  | { check: 'range'; column: string; min: number; max: number }
  | { check: 'regex'; column: string; pattern: string }
  | { check: 'freshness'; column: string; max_age: string }
  | { check: 'row_count'; min: number; max?: number }
  | { check: 'schema_drift' };

export type CheckStatus = 'pass' | 'warn' | 'fail';

/** Result of a single quality check. */
export interface QualityCheckResult {
  dataset: string;
  check: string;
  status: CheckStatus;
  value: string;
  threshold: string;
  checked_at: string;
}

/** Full quality report for a dataset. */
export interface QualityReport {
  dataset: string;
  checks: QualityCheckResult[];
  passed: number;
  warned: number;
  failed: number;
  overall: CheckStatus;
  generated_at: string;
}

/** Request to configure quality checks for a dataset. */
export interface ConfigureQualityRequest {
  dataset: string;
  checks: QualityCheckType[];
}
