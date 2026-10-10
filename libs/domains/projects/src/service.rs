use std::sync::Arc;

use contract_projects::ProjectDeleted;
use tracing::{instrument, warn};
use uuid::Uuid;
use validator::Validate;

use crate::error::{ProjectError, ProjectResult};
use crate::events::ProjectEventPublisher;
use crate::models::{CreateProject, Project, ProjectFilter, ProjectStatus, UpdateProject};
use crate::repository::ProjectRepository;

/// Service layer for Project business logic
pub struct ProjectService<R: ProjectRepository> {
    repository: Arc<R>,
    publisher: Arc<dyn ProjectEventPublisher>,
}

/// Hand-written so cloning only bumps the `Arc`; `derive(Clone)` would demand
/// `R: Clone`, which repositories (DB handles) have no reason to be.
impl<R: ProjectRepository> Clone for ProjectService<R> {
    fn clone(&self) -> Self {
        Self {
            repository: Arc::clone(&self.repository),
            publisher: Arc::clone(&self.publisher),
        }
    }
}

impl<R: ProjectRepository> ProjectService<R> {
    /// The publisher is a required argument rather than an optional builder
    /// step: a project deleted without its `ProjectDeleted` event leaves
    /// permanently dangling `project_id` references in the tasks service, and
    /// that must not be reachable by forgetting a call.
    pub fn new(repository: R, publisher: Arc<dyn ProjectEventPublisher>) -> Self {
        Self {
            repository: Arc::new(repository),
            publisher,
        }
    }

    /// Create a new project with validation and limit checking
    #[instrument(skip(self, input), fields(user_id = %input.user_id, project_name = %input.name))]
    pub async fn create_project(&self, input: CreateProject) -> ProjectResult<Project> {
        // Validate input
        input
            .validate()
            .map_err(|e| ProjectError::Validation(e.to_string()))?;

        // Check if the user can create more projects
        if !self.can_user_create_project(input.user_id).await? {
            return Err(ProjectError::Validation(
                "Free tier limit reached: maximum 3 projects per user".to_string(),
            ));
        }

        self.repository.create(input).await
    }

    /// Check if a user can create more projects (free tier: 3 projects max)
    #[instrument(skip(self), fields(user_id = %user_id))]
    pub async fn can_user_create_project(&self, user_id: Uuid) -> ProjectResult<bool> {
        const FREE_TIER_LIMIT: usize = 3;

        let count = self.repository.count_by_user(user_id).await?;
        Ok(count < FREE_TIER_LIMIT)
    }

    /// Get a project by ID
    #[instrument(skip(self), fields(project_id = %id))]
    pub async fn get_project(&self, id: Uuid) -> ProjectResult<Project> {
        self.repository
            .get_by_id(id)
            .await?
            .ok_or(ProjectError::NotFound(id))
    }

    /// Get a project by ID, verifying user ownership
    pub async fn get_project_for_user(&self, id: Uuid, user_id: Uuid) -> ProjectResult<Project> {
        let project = self.get_project(id).await?;

        if project.user_id != user_id {
            return Err(ProjectError::Unauthorized(id));
        }

        Ok(project)
    }

    /// List projects with filters
    pub async fn list_projects(&self, filter: ProjectFilter) -> ProjectResult<Vec<Project>> {
        self.repository.list(filter).await
    }

    /// Update a project
    #[instrument(skip(self, input), fields(project_id = %id))]
    pub async fn update_project(&self, id: Uuid, input: UpdateProject) -> ProjectResult<Project> {
        // Validate input
        input
            .validate()
            .map_err(|e| ProjectError::Validation(e.to_string()))?;

        self.repository.update(id, input).await
    }

    /// Update a project, verifying user ownership
    pub async fn update_project_for_user(
        &self,
        id: Uuid,
        user_id: Uuid,
        input: UpdateProject,
    ) -> ProjectResult<Project> {
        let project = self.get_project(id).await?;

        if project.user_id != user_id {
            return Err(ProjectError::Unauthorized(id));
        }

        self.update_project(id, input).await
    }

    /// Delete a project, then announce it so other services can drop their
    /// references to the id.
    ///
    /// The publish is **best-effort by design**, matching the welcome-email
    /// dual write in `establish_session`: the row is already gone, so failing
    /// the request would report an error for work that succeeded and invite a
    /// retry of a delete that cannot be repeated. A lost event therefore
    /// leaves a stale `project_id` in the tasks service, which is why the read
    /// side must render an unresolvable project reference as "no project"
    /// rather than trusting it — the two halves of backlog 0.3. Upgrade this
    /// to a transactional outbox (backlog 5.2) if a consumer ever needs an
    /// exactly-once guarantee here.
    #[instrument(skip(self), fields(project_id = %id))]
    pub async fn delete_project(&self, id: Uuid) -> ProjectResult<()> {
        let deleted = self.repository.delete(id).await?;

        if !deleted {
            return Err(ProjectError::NotFound(id));
        }

        if let Err(e) = self
            .publisher
            .publish_deleted(ProjectDeleted::new(id))
            .await
        {
            warn!(error = %e, project_id = %id, "failed to publish ProjectDeleted");
        }

        Ok(())
    }

    /// Delete a project, verifying user ownership
    pub async fn delete_project_for_user(&self, id: Uuid, user_id: Uuid) -> ProjectResult<()> {
        let project = self.get_project(id).await?;

        if project.user_id != user_id {
            return Err(ProjectError::Unauthorized(id));
        }

        self.delete_project(id).await
    }

    /// Activate a project (change status to Active)
    pub async fn activate_project(&self, id: Uuid) -> ProjectResult<Project> {
        let project = self.get_project(id).await?;

        if project.status == ProjectStatus::Active {
            return Ok(project);
        }

        if project.status == ProjectStatus::Deleting {
            return Err(ProjectError::Validation(
                "Cannot activate a project being deleted".to_string(),
            ));
        }

        self.repository
            .update(
                id,
                UpdateProject {
                    status: Some(ProjectStatus::Active),
                    ..Default::default()
                },
            )
            .await
    }

    /// Suspend a project
    pub async fn suspend_project(&self, id: Uuid) -> ProjectResult<Project> {
        let project = self.get_project(id).await?;

        if project.status == ProjectStatus::Suspended {
            return Ok(project);
        }

        if project.status != ProjectStatus::Active {
            return Err(ProjectError::Validation(
                "Only active projects can be suspended".to_string(),
            ));
        }

        self.repository
            .update(
                id,
                UpdateProject {
                    status: Some(ProjectStatus::Suspended),
                    ..Default::default()
                },
            )
            .await
    }

    /// Archive a project
    pub async fn archive_project(&self, id: Uuid) -> ProjectResult<Project> {
        self.repository
            .update(
                id,
                UpdateProject {
                    status: Some(ProjectStatus::Archived),
                    enabled: Some(false),
                    ..Default::default()
                },
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::events::{MockProjectEventPublisher, NoopProjectPublisher};
    use crate::repository::MockProjectRepository;

    #[tokio::test]
    async fn test_can_create_project_when_under_limit() {
        let mut mock_repo = MockProjectRepository::new();
        let user_id = Uuid::now_v7();

        // Mock: user has 2 projects (under the 3-project limit)
        mock_repo
            .expect_count_by_user()
            .with(mockall::predicate::eq(user_id))
            .returning(|_| Ok(2));

        let service = ProjectService::new(mock_repo, Arc::new(NoopProjectPublisher));
        let can_create = service.can_user_create_project(user_id).await.unwrap();

        assert!(
            can_create,
            "User with 2 projects should be able to create more"
        );
    }

    #[tokio::test]
    async fn test_cannot_create_project_when_at_limit() {
        let mut mock_repo = MockProjectRepository::new();
        let user_id = Uuid::now_v7();

        // Mock: user has 3 projects (at the limit)
        mock_repo
            .expect_count_by_user()
            .with(mockall::predicate::eq(user_id))
            .returning(|_| Ok(3));

        let service = ProjectService::new(mock_repo, Arc::new(NoopProjectPublisher));
        let can_create = service.can_user_create_project(user_id).await.unwrap();

        assert!(
            !can_create,
            "User with 3 projects should not be able to create more"
        );
    }

    #[tokio::test]
    async fn test_cannot_create_project_when_over_limit() {
        let mut mock_repo = MockProjectRepository::new();
        let user_id = Uuid::now_v7();

        // Mock: user has 5 projects (over the limit)
        mock_repo
            .expect_count_by_user()
            .with(mockall::predicate::eq(user_id))
            .returning(|_| Ok(5));

        let service = ProjectService::new(mock_repo, Arc::new(NoopProjectPublisher));
        let can_create = service.can_user_create_project(user_id).await.unwrap();

        assert!(
            !can_create,
            "User with 5 projects should not be able to create more"
        );
    }

    #[tokio::test]
    async fn test_can_create_first_project() {
        let mut mock_repo = MockProjectRepository::new();
        let user_id = Uuid::now_v7();

        // Mock: user has 0 projects
        mock_repo
            .expect_count_by_user()
            .with(mockall::predicate::eq(user_id))
            .returning(|_| Ok(0));

        let service = ProjectService::new(mock_repo, Arc::new(NoopProjectPublisher));
        let can_create = service.can_user_create_project(user_id).await.unwrap();

        assert!(
            can_create,
            "User with 0 projects should be able to create their first"
        );
    }

    /// The delete → event link is the whole mechanism behind backlog 0.3: if a
    /// delete stops announcing itself, other services keep references to an id
    /// that no longer exists, and nothing fails loudly.
    #[tokio::test]
    async fn deleting_a_project_publishes_project_deleted() {
        let id = Uuid::now_v7();
        let mut mock_repo = MockProjectRepository::new();
        mock_repo
            .expect_delete()
            .with(mockall::predicate::eq(id))
            .returning(|_| Ok(true));

        let mut publisher = MockProjectEventPublisher::new();
        publisher
            .expect_publish_deleted()
            .withf(move |event| event.project_id == id)
            .times(1)
            .returning(|_| Ok(()));

        let service = ProjectService::new(mock_repo, Arc::new(publisher));
        service.delete_project(id).await.unwrap();
    }

    /// A delete that changed nothing must not announce a deletion: consumers
    /// would clear references to a project that still exists.
    #[tokio::test]
    async fn a_missing_project_publishes_nothing() {
        let id = Uuid::now_v7();
        let mut mock_repo = MockProjectRepository::new();
        mock_repo.expect_delete().returning(|_| Ok(false));

        let mut publisher = MockProjectEventPublisher::new();
        publisher.expect_publish_deleted().never();

        let service = ProjectService::new(mock_repo, Arc::new(publisher));
        let err = service.delete_project(id).await.unwrap_err();
        assert!(matches!(err, ProjectError::NotFound(missing) if missing == id));
    }

    /// A broker outage must not fail a delete that already committed — the row
    /// is gone, and reporting an error would invite a retry of unrepeatable
    /// work. Documented as best-effort on `delete_project`.
    #[tokio::test]
    async fn a_publish_failure_does_not_fail_the_delete() {
        let id = Uuid::now_v7();
        let mut mock_repo = MockProjectRepository::new();
        mock_repo.expect_delete().returning(|_| Ok(true));

        let mut publisher = MockProjectEventPublisher::new();
        publisher
            .expect_publish_deleted()
            .times(1)
            .returning(|_| Err(ProjectError::Internal("nats down".to_string())));

        let service = ProjectService::new(mock_repo, Arc::new(publisher));
        service
            .delete_project(id)
            .await
            .expect("delete must succeed even when the event cannot be published");
    }
}
