//! Vector API routes
//!
//! Exposes vector storage and search operations via REST API.
//!
//! # Tenancy
//!
//! The client names the *target project* in the query string or request body, but
//! that value is only a resource selector - never an identity. Every handler
//! resolves the caller's [`crate::orgs::TenantContext`] (injected by
//! `orgs::tenant_context_mw`, which runs inside `oidc_auth::auth_required`) and
//! verifies the requested project belongs to that user before touching Qdrant.
//! A project owned by somebody else is reported as **404**, not 403, so the API
//! never leaks the existence of another tenant's projects. The `user_id` scoping
//! the vector store always comes from the verified tenant, never from the wire.

use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use domain_projects::{
    PgProjectRepository, ProjectService, error::ProjectError, repository::ProjectRepository,
};
use domain_vector::{
    QdrantRepository, SearchWithEmbedding, VectorService,
    error::{VectorError, VectorResult},
    models::{
        CollectionInfo, CreateCollection, EmbeddingResult, SearchResult, TenantContext, Vector,
        VectorConfig,
    },
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

// Request DTOs
use domain_vector::handlers::direct::{
    CreateCollectionRequest, DeleteVectorsRequest, EmbedRequest, GetVectorsRequest, SearchRequest,
    SearchWithEmbeddingRequest, UpsertBatchRequest, UpsertRequest,
};

/// State for the vector routes: the Qdrant-backed vector service plus the project
/// service the ownership guard consults.
#[derive(Clone)]
pub struct VectorState {
    service: Arc<VectorService<QdrantRepository>>,
    projects: ProjectService<PgProjectRepository>,
}

/// Create router for vector operations.
///
/// Returns `None` if the vector service is not configured (no Qdrant).
pub fn router(state: &crate::state::AppState) -> Option<Router> {
    let service = state.vector_service.as_ref()?;
    let vector_state = VectorState {
        service: Arc::clone(service),
        projects: ProjectService::new(PgProjectRepository::new(state.db.clone())),
    };

    use axum::routing::{get, post};

    Some(
        Router::new()
            // Collection management
            .route(
                "/collections",
                get(list_collections).post(create_collection),
            )
            .route(
                "/collections/{name}",
                get(get_collection).delete(delete_collection),
            )
            // Vector operations
            .route("/vectors/search", post(search))
            .route("/vectors/upsert", post(upsert))
            .route("/vectors/upsert-batch", post(upsert_batch))
            .route("/vectors/get", post(get_vectors))
            .route("/vectors/delete", post(delete_vectors))
            // Embedding operations
            .route("/embed", post(embed))
            .route("/search-with-embedding", post(search_with_embedding))
            .with_state(vector_state),
    )
}

/// Selects which project's vectors a request targets. Authorization is derived
/// from the verified tenant, not from these params.
#[derive(Debug, Clone, Deserialize)]
pub struct TenantQueryParams {
    pub project_id: Uuid,
    pub namespace: Option<String>,
}

/// Build the store-facing tenant context after proving the caller owns `project_id`.
///
/// `user_id` is taken from the verified tenant; anything the client sent for it is
/// discarded. Both "no such project" and "somebody else's project" surface as 404 so
/// the API never confirms that another tenant's project exists.
///
/// Generic over the repository so the authorization decision is unit-testable
/// without Postgres.
async fn authorize<R: ProjectRepository>(
    projects: &ProjectService<R>,
    caller: &crate::orgs::TenantContext,
    project_id: Uuid,
    namespace: Option<String>,
) -> VectorResult<TenantContext> {
    match projects
        .get_project_for_user(project_id, caller.user_id)
        .await
    {
        Ok(_) => Ok(TenantContext {
            project_id,
            namespace,
            user_id: Some(caller.user_id),
        }),
        Err(ProjectError::NotFound(_) | ProjectError::Unauthorized(_)) => Err(
            VectorError::NotFound(format!("Project {project_id} not found")),
        ),
        Err(e) => Err(VectorError::Internal(format!("project lookup failed: {e}"))),
    }
}

/// Handler-facing wrapper over [`authorize`].
async fn authorized_tenant(
    st: &VectorState,
    caller: &crate::orgs::TenantContext,
    project_id: Uuid,
    namespace: Option<String>,
) -> VectorResult<TenantContext> {
    authorize(&st.projects, caller, project_id, namespace).await
}

// Collection handlers

pub async fn list_collections(
    State(st): State<VectorState>,
    Extension(caller): Extension<crate::orgs::TenantContext>,
    axum::extract::Query(params): axum::extract::Query<TenantQueryParams>,
) -> VectorResult<Json<Vec<CollectionInfo>>> {
    let tenant = authorized_tenant(&st, &caller, params.project_id, params.namespace).await?;

    let collections = st.service.list_collections(&tenant).await?;
    Ok(Json(collections))
}

pub async fn get_collection(
    State(st): State<VectorState>,
    Extension(caller): Extension<crate::orgs::TenantContext>,
    Path(name): Path<String>,
    axum::extract::Query(params): axum::extract::Query<TenantQueryParams>,
) -> VectorResult<impl IntoResponse> {
    let tenant = authorized_tenant(&st, &caller, params.project_id, params.namespace).await?;

    let collection = st
        .service
        .get_collection(&tenant, &name)
        .await?
        .ok_or(VectorError::CollectionNotFound(name))?;

    Ok(Json(collection))
}

pub async fn create_collection(
    State(st): State<VectorState>,
    Extension(caller): Extension<crate::orgs::TenantContext>,
    Json(request): Json<CreateCollectionRequest>,
) -> VectorResult<impl IntoResponse> {
    let tenant = authorized_tenant(
        &st,
        &caller,
        request.tenant.project_id,
        request.tenant.namespace,
    )
    .await?;

    let input = CreateCollection {
        name: request.name,
        config: request.config.unwrap_or_else(|| VectorConfig::new(1536)),
    };

    let collection = st.service.create_collection(&tenant, input).await?;
    Ok((StatusCode::CREATED, Json(collection)))
}

pub async fn delete_collection(
    State(st): State<VectorState>,
    Extension(caller): Extension<crate::orgs::TenantContext>,
    Path(name): Path<String>,
    axum::extract::Query(params): axum::extract::Query<TenantQueryParams>,
) -> VectorResult<impl IntoResponse> {
    let tenant = authorized_tenant(&st, &caller, params.project_id, params.namespace).await?;

    st.service.delete_collection(&tenant, &name).await?;
    Ok(StatusCode::NO_CONTENT)
}

// Vector handlers

pub async fn search(
    State(st): State<VectorState>,
    Extension(caller): Extension<crate::orgs::TenantContext>,
    Json(request): Json<SearchRequest>,
) -> VectorResult<Json<Vec<SearchResult>>> {
    let tenant = authorized_tenant(
        &st,
        &caller,
        request.tenant.project_id,
        request.tenant.namespace,
    )
    .await?;

    let results = st
        .service
        .search(&tenant, &request.collection_name, request.query)
        .await?;
    Ok(Json(results))
}

#[derive(serde::Serialize)]
pub struct UpsertResponse {
    pub id: Uuid,
    pub status: String,
}

pub async fn upsert(
    State(st): State<VectorState>,
    Extension(caller): Extension<crate::orgs::TenantContext>,
    Json(request): Json<UpsertRequest>,
) -> VectorResult<Json<UpsertResponse>> {
    let tenant = authorized_tenant(
        &st,
        &caller,
        request.tenant.project_id,
        request.tenant.namespace,
    )
    .await?;

    let id = st
        .service
        .upsert(
            &tenant,
            &request.collection_name,
            request.vector,
            request.wait,
        )
        .await?;

    Ok(Json(UpsertResponse {
        id,
        status: if request.wait {
            "completed".to_string()
        } else {
            "pending".to_string()
        },
    }))
}

#[derive(serde::Serialize)]
pub struct UpsertBatchResponse {
    pub ids: Vec<Uuid>,
    pub count: u32,
    pub status: String,
}

pub async fn upsert_batch(
    State(st): State<VectorState>,
    Extension(caller): Extension<crate::orgs::TenantContext>,
    Json(request): Json<UpsertBatchRequest>,
) -> VectorResult<Json<UpsertBatchResponse>> {
    let tenant = authorized_tenant(
        &st,
        &caller,
        request.tenant.project_id,
        request.tenant.namespace,
    )
    .await?;

    let count = request.vectors.len() as u32;
    let ids = st
        .service
        .upsert_batch(
            &tenant,
            &request.collection_name,
            request.vectors,
            request.wait,
        )
        .await?;

    Ok(Json(UpsertBatchResponse {
        ids,
        count,
        status: if request.wait {
            "completed".to_string()
        } else {
            "pending".to_string()
        },
    }))
}

pub async fn get_vectors(
    State(st): State<VectorState>,
    Extension(caller): Extension<crate::orgs::TenantContext>,
    Json(request): Json<GetVectorsRequest>,
) -> VectorResult<Json<Vec<Vector>>> {
    let tenant = authorized_tenant(
        &st,
        &caller,
        request.tenant.project_id,
        request.tenant.namespace,
    )
    .await?;

    let vectors = st
        .service
        .get(
            &tenant,
            &request.collection_name,
            request.ids,
            request.with_vectors,
            request.with_payloads,
        )
        .await?;
    Ok(Json(vectors))
}

#[derive(serde::Serialize)]
pub struct DeleteResponse {
    pub deleted_count: u32,
    pub status: String,
}

pub async fn delete_vectors(
    State(st): State<VectorState>,
    Extension(caller): Extension<crate::orgs::TenantContext>,
    Json(request): Json<DeleteVectorsRequest>,
) -> VectorResult<Json<DeleteResponse>> {
    let tenant = authorized_tenant(
        &st,
        &caller,
        request.tenant.project_id,
        request.tenant.namespace,
    )
    .await?;

    let deleted_count = st
        .service
        .delete(&tenant, &request.collection_name, request.ids, request.wait)
        .await?;

    Ok(Json(DeleteResponse {
        deleted_count,
        status: if request.wait {
            "completed".to_string()
        } else {
            "pending".to_string()
        },
    }))
}

// Embedding handlers

/// Computes an embedding; touches no stored data, so it needs no tenant scope.
pub async fn embed(
    State(st): State<VectorState>,
    Json(request): Json<EmbedRequest>,
) -> VectorResult<Json<EmbeddingResult>> {
    let result = st.service.embed(request.model, &request.text).await?;
    Ok(Json(result))
}

pub async fn search_with_embedding(
    State(st): State<VectorState>,
    Extension(caller): Extension<crate::orgs::TenantContext>,
    Json(request): Json<SearchWithEmbeddingRequest>,
) -> VectorResult<Json<Vec<SearchResult>>> {
    let tenant = authorized_tenant(
        &st,
        &caller,
        request.tenant.project_id,
        request.tenant.namespace,
    )
    .await?;

    let results = st
        .service
        .search_with_embedding(
            &tenant,
            &request.collection_name,
            SearchWithEmbedding {
                text: request.text,
                limit: request.limit,
                score_threshold: request.score_threshold,
                with_vectors: request.with_vectors,
                with_payloads: request.with_payloads,
                model: request.model,
            },
        )
        .await?;
    Ok(Json(results))
}

#[cfg(test)]
mod tests {
    //! Guards the P0 tenancy contract: the project named by the client is only a
    //! resource selector, and the vector store is always scoped to the *verified*
    //! caller. Repository is faked so this needs no Postgres.

    use super::*;
    use async_trait::async_trait;
    use chrono::Utc;
    use domain_projects::error::ProjectResult;
    use domain_projects::models::{
        CloudProvider, CreateProject, Environment, Project, ProjectFilter, ProjectStatus,
        UpdateProject,
    };

    /// Repository holding exactly one project, owned by `owner`.
    struct OneProject {
        id: Uuid,
        owner: Uuid,
    }

    impl OneProject {
        fn project(&self) -> Project {
            Project {
                id: self.id,
                name: "p".to_string(),
                user_id: self.owner,
                description: String::new(),
                cloud_provider: CloudProvider::Local,
                region: "local".to_string(),
                environment: Environment::default(),
                status: ProjectStatus::default(),
                budget_limit: None,
                tags: Vec::new(),
                enabled: true,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            }
        }
    }

    #[async_trait]
    impl ProjectRepository for OneProject {
        async fn get_by_id(&self, id: Uuid) -> ProjectResult<Option<Project>> {
            Ok((id == self.id).then(|| self.project()))
        }

        async fn create(&self, _input: CreateProject) -> ProjectResult<Project> {
            unimplemented!("not exercised by the authorization tests")
        }
        async fn list(&self, _filter: ProjectFilter) -> ProjectResult<Vec<Project>> {
            unimplemented!("not exercised by the authorization tests")
        }
        async fn update(&self, _id: Uuid, _input: UpdateProject) -> ProjectResult<Project> {
            unimplemented!("not exercised by the authorization tests")
        }
        async fn delete(&self, _id: Uuid) -> ProjectResult<bool> {
            unimplemented!("not exercised by the authorization tests")
        }
        async fn exists_by_name(&self, _user_id: Uuid, _name: &str) -> ProjectResult<bool> {
            unimplemented!("not exercised by the authorization tests")
        }
        async fn count_by_user(&self, _user_id: Uuid) -> ProjectResult<usize> {
            unimplemented!("not exercised by the authorization tests")
        }
    }

    fn caller(user_id: Uuid) -> crate::orgs::TenantContext {
        crate::orgs::TenantContext {
            user_id,
            org_id: Uuid::now_v7(),
            external_org_id: format!("personal:{user_id}"),
            org_name: "test".to_string(),
            role: "admin".to_string(),
        }
    }

    #[tokio::test]
    async fn owner_gets_a_tenant_scoped_to_the_verified_user() {
        let owner = Uuid::now_v7();
        let project_id = Uuid::now_v7();
        let projects = ProjectService::new(OneProject {
            id: project_id,
            owner,
        });

        let tenant = authorize(
            &projects,
            &caller(owner),
            project_id,
            Some("ns".to_string()),
        )
        .await
        .expect("owner is authorized for their own project");

        assert_eq!(tenant.project_id, project_id);
        assert_eq!(tenant.namespace.as_deref(), Some("ns"));
        assert_eq!(
            tenant.user_id,
            Some(owner),
            "store scope must come from the verified tenant"
        );
    }

    #[tokio::test]
    async fn another_tenants_project_is_reported_as_not_found() {
        let owner = Uuid::now_v7();
        let attacker = Uuid::now_v7();
        let project_id = Uuid::now_v7();
        let projects = ProjectService::new(OneProject {
            id: project_id,
            owner,
        });

        let err = authorize(&projects, &caller(attacker), project_id, None)
            .await
            .expect_err("cross-tenant access must be refused");

        assert!(
            matches!(err, VectorError::NotFound(_)),
            "must 404 (not 403) so project existence never leaks, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn unknown_project_is_reported_as_not_found() {
        let owner = Uuid::now_v7();
        let projects = ProjectService::new(OneProject {
            id: Uuid::now_v7(),
            owner,
        });

        let err = authorize(&projects, &caller(owner), Uuid::now_v7(), None)
            .await
            .expect_err("unknown project must be refused");

        assert!(matches!(err, VectorError::NotFound(_)), "got: {err:?}");
    }
}
