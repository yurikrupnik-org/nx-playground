use utoipa::OpenApi;

use crate::types::*;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Matia API",
        version = "0.1.0",
        description = "Self-hosted data platform API — catalog, pipelines, quality, connectors, issues"
    ),
    paths(
        crate::api::catalog::list_datasets,
        crate::api::catalog::get_dataset,
        crate::api::catalog::register_dataset,
        crate::api::pipelines::list_pipelines,
        crate::api::pipelines::get_pipeline,
        crate::api::pipelines::create_pipeline,
        crate::api::pipelines::run_pipeline,
        crate::api::quality::quality_report,
        crate::api::quality::configure_quality,
        crate::api::connectors::list_connectors,
        crate::api::connectors::get_connector,
        crate::api::connectors::create_connector,
        crate::api::connectors::test_connector,
        crate::api::issues::list_issues,
        crate::api::issues::get_issue,
        crate::api::issues::update_issue,
        crate::api::lineage::get_lineage,
    ),
    components(schemas(
        DataSource, OutputTarget, ExportFormat,
        ColumnSchema, Freshness, DatasetMeta, RegisterDatasetRequest,
        PipelineStep, PipelineStatus, Pipeline, CreatePipelineRequest, PipelineRunResult,
        QualityCheckType, CheckStatus, QualityCheckResult, QualityReport, ConfigureQualityRequest,
        ConnectorType, ConnectorDirection, ConnectorStatus, Connector, CreateConnectorRequest, ConnectorTestResult,
        IssueSeverity, IssueType, IssueStatus, Issue,
    ))
)]
pub struct ApiDoc;
