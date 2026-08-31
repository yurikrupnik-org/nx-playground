-- Desired Database Schema for the `tasks` service (single source of truth - no migrations)
--
-- This database is owned exclusively by `apps/zerg/tasks`. `zerg_api` holds no
-- credentials for it: the only way to reach these rows is the gRPC service. See
-- `docs/adr-tasks-service-boundary.md`.
--
-- Local dev:  `just db-fresh tasks` drops/recreates from this file
-- K8s:        Atlas Operator reconciles it declaratively (AtlasSchema + tasks-schema ConfigMap)
--
-- PostgreSQL 18 (uuidv7() is a built-in function, no extension required)

-- ============================================================================
-- Enums
-- ============================================================================

CREATE TYPE task_priority AS ENUM ('low', 'medium', 'high', 'urgent');
CREATE TYPE task_status AS ENUM ('todo', 'in_progress', 'done');

-- ============================================================================
-- Tasks
-- ============================================================================

-- `org_ref` and `user_ref` are **identity-provider references, not foreign keys**.
-- They hold exactly what a verified access token carries - a WorkOS organization id
-- (`org_01...`), a WorkOS user id (`user_01...`), or the personal-workspace fallback
-- `personal:{subject}`. That is what makes the service able to derive tenancy from the
-- token alone, with no lookup into another service's database.
--
-- Deliberate trade: there is no referential integrity across the service boundary, so
-- deleting a user or organization elsewhere leaves orphan rows here. Accepted - account
-- deletion is not implemented; revisit with a `UserDeleted` NATS event when it is.
--
-- `project_id` is likewise an opaque id, not an FK into the zerg `projects` table.
-- Unlike the user/org refs above, this one IS maintained across the boundary: the
-- owning service publishes `ProjectDeleted` (`projects.>`, contract_projects) and
-- this service's `tasks-project-refs` consumer nulls the column. An ID reference
-- plus an event is what replaces a cross-context FK here; the read side still
-- tolerates a stale id, because the correction is eventually consistent.
CREATE TABLE tasks (
  id UUID PRIMARY KEY DEFAULT uuidv7(),
  title VARCHAR(255) NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  completed BOOLEAN NOT NULL DEFAULT false,
  org_ref TEXT NOT NULL,
  user_ref TEXT NOT NULL,
  project_id UUID,
  priority task_priority NOT NULL DEFAULT 'medium',
  status task_status NOT NULL DEFAULT 'todo',
  due_date TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Every query is org-scoped; the composite index serves the "mine" narrowing.
CREATE INDEX idx_tasks_org_ref ON tasks(org_ref);
CREATE INDEX idx_tasks_org_user ON tasks(org_ref, user_ref);
CREATE INDEX idx_tasks_project_id ON tasks(project_id);
CREATE INDEX idx_tasks_project_status ON tasks(project_id, status);
CREATE INDEX idx_tasks_due_date ON tasks(due_date) WHERE due_date IS NOT NULL;

-- ============================================================================
-- Triggers
-- ============================================================================

-- The repository sets `updated_at` on its own update path; this trigger is the
-- backstop for any other writer (psql, a future migration) so the column cannot
-- silently go stale. Mirrors the convention in manifests/db/zerg/schema.sql.
CREATE SCHEMA IF NOT EXISTS util;

CREATE OR REPLACE FUNCTION util.touch_updated_at()
RETURNS trigger AS $$
BEGIN
  NEW.updated_at = NOW();
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER tasks_touch_updated_at
  BEFORE UPDATE ON tasks
  FOR EACH ROW
  EXECUTE FUNCTION util.touch_updated_at();
