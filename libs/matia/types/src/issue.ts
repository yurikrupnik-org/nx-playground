export type IssueSeverity = 'critical' | 'warning' | 'info';

export type IssueType =
  | 'quality'
  | 'pipeline'
  | 'schema'
  | 'freshness'
  | 'sync'
  | 'connector';

export type IssueStatus = 'open' | 'acknowledged' | 'resolved';

/** An auto-detected or manually created data issue. */
export interface Issue {
  id: string;
  title: string;
  dataset: string;
  severity: IssueSeverity;
  type: IssueType;
  status: IssueStatus;
  description: string | null;
  assignee: string | null;
  detected_at: string;
  resolved_at: string | null;
}
