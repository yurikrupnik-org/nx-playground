-- terran — Desired Database Schema (single source of truth)
-- B2B multi-tenant: identity is global, authorization is tenant-scoped (see
-- docs/terran-apps-plan.md → "Multi-tenancy & security model").
-- Local dev: `just db-fresh terran` applies this directly via psql.
-- Cluster:   Atlas Operator applies migrations via ConfigMaps.
-- PostgreSQL 18 (uuidv7() is a built-in function, no extension required).

-- =============================================================================
-- Extensions
-- =============================================================================
CREATE EXTENSION IF NOT EXISTS pgcrypto;

-- =============================================================================
-- Schemas
-- =============================================================================
CREATE SCHEMA IF NOT EXISTS util;

-- =============================================================================
-- Utility Functions
-- =============================================================================
CREATE OR REPLACE FUNCTION util.touch_updated_at()
RETURNS trigger AS $$
BEGIN
  NEW.updated_at = NOW();
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Active tenant for the current transaction. The API sets this per request as
-- `SET LOCAL app.org_id = '<uuid>'` from the verified IdP token; NULL when unset.
-- Row-Level Security policies below key off this value.
CREATE OR REPLACE FUNCTION util.current_org_id()
RETURNS uuid AS $$
  SELECT NULLIF(current_setting('app.org_id', true), '')::uuid;
$$ LANGUAGE sql STABLE;

-- =============================================================================
-- Enum Types
-- =============================================================================
CREATE TYPE org_role AS ENUM ('org_admin', 'member', 'viewer');
CREATE TYPE cloud_provider AS ENUM ('aws', 'gcp', 'azure');
CREATE TYPE asset_status AS ENUM ('active', 'stopped', 'terminated', 'unknown');

-- =============================================================================
-- Tables
-- =============================================================================

-- Organizations (tenants). `external_org_id` maps to the IdP organization/group id.
-- Managed by the identity/provisioning layer; not RLS-protected (business endpoints
-- never query it directly — they resolve the active org from the token).
CREATE TABLE organizations (
  id UUID PRIMARY KEY DEFAULT uuidv7(),
  external_org_id VARCHAR(255) NOT NULL,
  name VARCHAR(255) NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX idx_organizations_external_id ON organizations(external_org_id);

-- Users (global identity). `subject` is the IdP `sub`. No credentials are stored —
-- the IdP (Keycloak) owns passwords, social links, MFA, and lockout.
CREATE TABLE users (
  id UUID PRIMARY KEY DEFAULT uuidv7(),
  subject VARCHAR(255) NOT NULL,
  email VARCHAR(255) NOT NULL,
  name VARCHAR(255) NOT NULL DEFAULT '',
  avatar_url TEXT,
  last_login_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX idx_users_subject ON users(subject);
CREATE UNIQUE INDEX idx_users_email ON users(email);

-- Membership of a user in an organization, with a coarse role. The authoritative
-- active org+role for a request comes from the token; this backs fine-grained data
-- and admin UIs. RLS-protected: callers only see memberships within their active org.
CREATE TABLE memberships (
  id UUID PRIMARY KEY DEFAULT uuidv7(),
  user_id UUID NOT NULL,
  org_id UUID NOT NULL,
  role org_role NOT NULL DEFAULT 'member',
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT fk_memberships_user FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
  CONSTRAINT fk_memberships_org FOREIGN KEY (org_id) REFERENCES organizations(id) ON DELETE CASCADE
);
CREATE UNIQUE INDEX uq_membership_user_org ON memberships(user_id, org_id);
CREATE INDEX idx_memberships_org ON memberships(org_id);

-- Sample tenant-scoped business resource: discovered cloud asset inventory
-- (FinOps/right-sizing source). Phase 3 ships this end-to-end as the auth/isolation
-- exemplar; real domain tables follow the same org_id + RLS shape.
CREATE TABLE cloud_assets (
  id UUID PRIMARY KEY DEFAULT uuidv7(),
  org_id UUID NOT NULL,
  discovered_by UUID,
  provider cloud_provider NOT NULL,
  external_id VARCHAR(512) NOT NULL,
  name VARCHAR(255) NOT NULL DEFAULT '',
  asset_type VARCHAR(128) NOT NULL,
  region VARCHAR(128) NOT NULL DEFAULT '',
  status asset_status NOT NULL DEFAULT 'unknown',
  monthly_cost NUMERIC(12, 2) NOT NULL DEFAULT 0,
  metadata JSONB NOT NULL DEFAULT '{}',
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  CONSTRAINT fk_cloud_assets_org FOREIGN KEY (org_id) REFERENCES organizations(id) ON DELETE CASCADE,
  CONSTRAINT fk_cloud_assets_user FOREIGN KEY (discovered_by) REFERENCES users(id) ON DELETE SET NULL,
  CONSTRAINT chk_monthly_cost_positive CHECK (monthly_cost >= 0)
);
CREATE INDEX idx_cloud_assets_org ON cloud_assets(org_id);
CREATE UNIQUE INDEX uq_cloud_asset_provider_external ON cloud_assets(org_id, provider, external_id);

-- =============================================================================
-- Triggers
-- =============================================================================
CREATE TRIGGER organizations_touch_updated_at
  BEFORE UPDATE ON organizations
  FOR EACH ROW EXECUTE FUNCTION util.touch_updated_at();

CREATE TRIGGER users_touch_updated_at
  BEFORE UPDATE ON users
  FOR EACH ROW EXECUTE FUNCTION util.touch_updated_at();

CREATE TRIGGER memberships_touch_updated_at
  BEFORE UPDATE ON memberships
  FOR EACH ROW EXECUTE FUNCTION util.touch_updated_at();

CREATE TRIGGER cloud_assets_touch_updated_at
  BEFORE UPDATE ON cloud_assets
  FOR EACH ROW EXECUTE FUNCTION util.touch_updated_at();

-- =============================================================================
-- Row-Level Security (defense-in-depth tenant isolation)
-- =============================================================================
-- The API sets `SET LOCAL app.org_id` from the verified token per transaction.
-- During login the provisioning path resolves the org first, then sets app.org_id
-- before upserting membership. FORCE applies RLS even to the table owner.
-- NOTE: superusers and BYPASSRLS roles are exempt — in every environment the API
-- MUST connect as a dedicated non-superuser role (not the local `myuser` superuser)
-- for these policies to take effect.
ALTER TABLE memberships ENABLE ROW LEVEL SECURITY;
ALTER TABLE memberships FORCE ROW LEVEL SECURITY;
CREATE POLICY membership_isolation ON memberships
  USING (org_id = util.current_org_id())
  WITH CHECK (org_id = util.current_org_id());

ALTER TABLE cloud_assets ENABLE ROW LEVEL SECURITY;
ALTER TABLE cloud_assets FORCE ROW LEVEL SECURITY;
CREATE POLICY cloud_asset_isolation ON cloud_assets
  USING (org_id = util.current_org_id())
  WITH CHECK (org_id = util.current_org_id());
