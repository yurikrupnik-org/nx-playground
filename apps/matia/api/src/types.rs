use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// ── Data Source (enum connectors) ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DataSource {
    Csv { path: String },
    Json { path: String },
    Parquet { path: String },
    Postgres { query: String },
    Mongo { database: String, collection: String },
    Nats { stream: String, subject: String },
    Influxdb { bucket: String, query: String },
    Qdrant { collection: String },
    Transform { pipeline: String, sources: Vec<String> },
    InMemory,
}

// ── Output Target (enum sinks) ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputTarget {
    Catalog { name: String },
    Postgres { table: String },
    Mongo { database: String, collection: String },
    Nats { topic: String },
    Parquet { path: String },
    Bigquery { table_id: String },
    Bigtable { table: String, column_family: String },
    S3 { bucket: String, prefix: String, format: ExportFormat },
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Parquet,
    Csv,
    Json,
}

// ── Catalog ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ColumnSchema {
    pub name: String,
    pub dtype: String,
    pub nullable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    Fresh,
    Stale,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DatasetMeta {
    pub name: String,
    pub source: DataSource,
    pub schema: Vec<ColumnSchema>,
    pub row_count: u64,
    pub column_count: u32,
    pub lineage: Vec<String>,
    pub tags: Vec<String>,
    pub freshness: Freshness,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RegisterDatasetRequest {
    pub name: String,
    pub source: DataSource,
    pub tags: Option<Vec<String>>,
}

// ── Pipeline ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PipelineStep {
    Filter { expr: String },
    Select { columns: Vec<String> },
    Join { dataset: String, left_on: String, right_on: String },
    GroupBy { column: String, agg: String },
    Sort { columns: Vec<String>, descending: bool },
    Limit { n: u64 },
    AddColumn { name: String, expr: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PipelineStatus {
    Idle,
    Running,
    Success,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Pipeline {
    pub id: String,
    pub name: String,
    pub sources: Vec<String>,
    pub steps: Vec<PipelineStep>,
    pub output: OutputTarget,
    pub schedule: Option<String>,
    pub status: PipelineStatus,
    pub last_run: Option<String>,
    pub rows_processed: u64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePipelineRequest {
    pub name: String,
    pub sources: Vec<String>,
    pub steps: Vec<PipelineStep>,
    pub output: OutputTarget,
    pub schedule: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PipelineRunResult {
    pub pipeline_id: String,
    pub status: PipelineStatus,
    pub rows_processed: u64,
    pub duration_ms: u64,
    pub error: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
}

// ── Quality ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "check", rename_all = "snake_case")]
pub enum QualityCheckType {
    NotNull { columns: Vec<String> },
    Unique { column: String },
    Range { column: String, min: f64, max: f64 },
    Regex { column: String, pattern: String },
    Freshness { column: String, max_age: String },
    RowCount { min: u64, max: Option<u64> },
    SchemaDrift,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct QualityCheckResult {
    pub dataset: String,
    pub check: String,
    pub status: CheckStatus,
    pub value: String,
    pub threshold: String,
    pub checked_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct QualityReport {
    pub dataset: String,
    pub checks: Vec<QualityCheckResult>,
    pub passed: u32,
    pub warned: u32,
    pub failed: u32,
    pub overall: CheckStatus,
    pub generated_at: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ConfigureQualityRequest {
    pub dataset: String,
    pub checks: Vec<QualityCheckType>,
}

// ── Connector ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorType {
    Database,
    Messaging,
    Storage,
    Vector,
    Timeseries,
    Warehouse,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorDirection {
    Source,
    Sink,
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorStatus {
    Connected,
    Disconnected,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Connector {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub connector_type: ConnectorType,
    pub direction: ConnectorDirection,
    pub status: ConnectorStatus,
    pub config: std::collections::HashMap<String, String>,
    pub datasets: u32,
    pub created_at: String,
    pub last_checked: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateConnectorRequest {
    pub name: String,
    #[serde(rename = "type")]
    pub connector_type: ConnectorType,
    pub direction: ConnectorDirection,
    pub config: std::collections::HashMap<String, String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ConnectorTestResult {
    pub status: String,
    pub error: Option<String>,
}

// ── Issue ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueSeverity {
    Critical,
    Warning,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueType {
    Quality,
    Pipeline,
    Schema,
    Freshness,
    Sync,
    Connector,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum IssueStatus {
    Open,
    Acknowledged,
    Resolved,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Issue {
    pub id: String,
    pub title: String,
    pub dataset: String,
    pub severity: IssueSeverity,
    #[serde(rename = "type")]
    pub issue_type: IssueType,
    pub status: IssueStatus,
    pub description: Option<String>,
    pub assignee: Option<String>,
    pub detected_at: String,
    pub resolved_at: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateIssueRequest {
    pub status: IssueStatus,
}

// ── Query params ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SampleQuery {
    pub limit: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct IssueListQuery {
    pub status: Option<IssueStatus>,
}
