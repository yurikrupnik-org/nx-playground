-- Initial schema (mirrors manifests/db/schema.sql)
-- PostgreSQL 18 (uuidv7() is a built-in function, no extension required)

CREATE EXTENSION IF NOT EXISTS pgcrypto;
CREATE EXTENSION IF NOT EXISTS hstore;

CREATE SCHEMA IF NOT EXISTS util;

CREATE OR REPLACE FUNCTION util.touch_updated_at()
RETURNS trigger AS $$
BEGIN
  NEW.updated_at = NOW();
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TYPE task_priority AS ENUM ('low', 'medium', 'high', 'urgent');
CREATE TYPE task_status AS ENUM ('todo', 'in_progress', 'done');
CREATE TYPE cloud_provider AS ENUM ('aws', 'gcp', 'azure');
CREATE TYPE environment AS ENUM ('development', 'staging', 'production');
CREATE TYPE project_status AS ENUM ('provisioning', 'active', 'suspended', 'deleting', 'archived');
CREATE TYPE resource_type AS ENUM ('compute', 'storage', 'database', 'network', 'serverless', 'analytics', 'other');
CREATE TYPE resource_status AS ENUM ('creating', 'active', 'updating', 'deleting', 'deleted', 'failed');

CREATE TABLE users (
  id UUID PRIMARY KEY DEFAULT uuidv7(),
  email VARCHAR(255) NOT NULL,
  name VARCHAR(255) NOT NULL,
  password_hash VARCHAR(255) NOT NULL,
  avatar_url TEXT,
  roles TEXT[] NOT NULL DEFAULT ARRAY['user'::text],
  email_verified BOOLEAN NOT NULL DEFAULT false,
  is_active BOOLEAN NOT NULL DEFAULT true,
  is_locked BOOLEAN NOT NULL DEFAULT false,
  failed_login_attempts INTEGER NOT NULL DEFAULT 0,
  locked_until TIMESTAMPTZ,
  last_login_at TIMESTAMPTZ,
  google_id VARCHAR(255),
  github_id VARCHAR(255),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX idx_users_email ON users(email);
CREATE UNIQUE INDEX idx_users_google_id ON users(google_id) WHERE google_id IS NOT NULL;
CREATE UNIQUE INDEX idx_users_github_id ON users(github_id) WHERE github_id IS NOT NULL;

CREATE TABLE oauth_accounts (
  id UUID PRIMARY KEY DEFAULT uuidv7(),
  user_id UUID NOT NULL,
  provider VARCHAR(50) NOT NULL,
  provider_user_id VARCHAR(255) NOT NULL,
  provider_username VARCHAR(255),
  email VARCHAR(255),
  display_name VARCHAR(255),
  avatar_url TEXT,
  access_token TEXT,
  refresh_token TEXT,
  token_expires_at TIMESTAMPTZ,
  scopes TEXT[],
  raw_user_data JSONB,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT fk_oauth_accounts_user FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
);

CREATE UNIQUE INDEX idx_oauth_provider_user ON oauth_accounts(provider, provider_user_id);
CREATE INDEX idx_oauth_accounts_user_id ON oauth_accounts(user_id);

CREATE TABLE projects (
  id UUID PRIMARY KEY DEFAULT uuidv7(),
  name VARCHAR(255) NOT NULL,
  user_id UUID NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  cloud_provider cloud_provider NOT NULL,
  region VARCHAR(255) NOT NULL,
  environment environment NOT NULL DEFAULT 'development',
  status project_status NOT NULL DEFAULT 'provisioning',
  budget_limit DOUBLE PRECISION,
  tags JSONB NOT NULL DEFAULT '{}',
  enabled BOOLEAN NOT NULL DEFAULT true,
  repository_url VARCHAR(500),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT fk_projects_user FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
  CONSTRAINT chk_budget_positive CHECK (budget_limit >= 0)
);

CREATE INDEX idx_projects_user_id ON projects(user_id);
CREATE INDEX idx_projects_user_status ON projects(user_id, status);
CREATE UNIQUE INDEX uq_project_name_per_user ON projects(user_id, name);

CREATE TABLE tasks (
  id UUID PRIMARY KEY DEFAULT uuidv7(),
  title VARCHAR(255) NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  completed BOOLEAN NOT NULL DEFAULT false,
  user_id UUID NOT NULL,
  project_id UUID,
  priority task_priority NOT NULL DEFAULT 'medium',
  status task_status NOT NULL DEFAULT 'todo',
  due_date TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT fk_tasks_user FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
  CONSTRAINT fk_tasks_project FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE SET NULL
);

CREATE INDEX idx_tasks_user_id ON tasks(user_id);
CREATE INDEX idx_tasks_project_id ON tasks(project_id);
CREATE INDEX idx_tasks_project_status ON tasks(project_id, status);
CREATE INDEX idx_tasks_due_date ON tasks(due_date) WHERE due_date IS NOT NULL;

CREATE TABLE cloud_resources (
  id UUID PRIMARY KEY DEFAULT uuidv7(),
  project_id UUID NOT NULL,
  name VARCHAR(255) NOT NULL,
  resource_type resource_type NOT NULL,
  status resource_status NOT NULL DEFAULT 'creating',
  region VARCHAR(255) NOT NULL,
  configuration JSONB NOT NULL DEFAULT '{}',
  cost_per_hour DOUBLE PRECISION,
  monthly_cost_estimate DOUBLE PRECISION,
  tags JSONB NOT NULL DEFAULT '{}',
  enabled BOOLEAN NOT NULL DEFAULT true,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  deleted_at TIMESTAMPTZ,
  CONSTRAINT fk_cloud_resources_project FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE,
  CONSTRAINT chk_cost_positive CHECK (cost_per_hour >= 0 AND monthly_cost_estimate >= 0)
);

CREATE INDEX idx_cloud_resources_project_id ON cloud_resources(project_id);
CREATE UNIQUE INDEX unique_resource_name_per_project ON cloud_resources(project_id, name);

CREATE TRIGGER users_touch_updated_at
  BEFORE UPDATE ON users
  FOR EACH ROW
  EXECUTE FUNCTION util.touch_updated_at();

CREATE TRIGGER oauth_accounts_touch_updated_at
  BEFORE UPDATE ON oauth_accounts
  FOR EACH ROW
  EXECUTE FUNCTION util.touch_updated_at();

CREATE TRIGGER projects_touch_updated_at
  BEFORE UPDATE ON projects
  FOR EACH ROW
  EXECUTE FUNCTION util.touch_updated_at();

CREATE TRIGGER tasks_touch_updated_at
  BEFORE UPDATE ON tasks
  FOR EACH ROW
  EXECUTE FUNCTION util.touch_updated_at();

CREATE TRIGGER cloud_resources_touch_updated_at
  BEFORE UPDATE ON cloud_resources
  FOR EACH ROW
  EXECUTE FUNCTION util.touch_updated_at();
