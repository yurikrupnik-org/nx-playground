-- Seed data for local development
-- Applied after schema.sql via: just db-fresh or just db-seed

-- =============================================================================
-- Seed Users
-- =============================================================================
-- Credentials live at the IdP (WorkOS); local rows are linked by `subject` on
-- first login (JIT provisioning backfills it by matching email).
INSERT INTO users (id, email, name, roles, email_verified, is_active, created_at, updated_at)
VALUES
    (
        '01930b3c-7c5f-7000-8000-000000000001',
        'admin@example.com',
        'Admin User',
        ARRAY['user', 'admin'],
        true,
        true,
        NOW(),
        NOW()
    ),
    (
        '01930b3c-7c5f-7001-8000-000000000002',
        'user@example.com',
        'Regular User',
        ARRAY['user'],
        true,
        true,
        NOW(),
        NOW()
    ),
    (
        '01930b3c-7c5f-7001-8000-000000000099',
        'developer@example.com',
        'Developer User',
        ARRAY['user', 'developer'],
        true,
        true,
        NOW(),
        NOW()
    ),
    (
      '01930b3c-7c5f-7001-8000-000000000322',
      'yuri@example.com',
      'Manager',
      ARRAY['user', 'developer'],
      true,
      true,
      NOW(),
      NOW()
    )
ON CONFLICT (id) DO NOTHING;

-- =============================================================================
-- Seed Organizations (personal workspaces; external id matches JIT convention
-- 'personal:{subject-or-user-id}' used by the backfill and provisioning)
-- =============================================================================
INSERT INTO organizations (id, external_org_id, name, created_at)
VALUES
    ('01930b3c-7c5f-7020-8000-00000000a001', 'personal:01930b3c-7c5f-7000-8000-000000000001', 'Admin User''s workspace', NOW()),
    ('01930b3c-7c5f-7020-8000-00000000a002', 'personal:01930b3c-7c5f-7001-8000-000000000002', 'Regular User''s workspace', NOW()),
    ('01930b3c-7c5f-7020-8000-00000000a099', 'personal:01930b3c-7c5f-7001-8000-000000000099', 'Developer User''s workspace', NOW()),
    ('01930b3c-7c5f-7020-8000-00000000a322', 'personal:01930b3c-7c5f-7001-8000-000000000322', 'Manager''s workspace', NOW())
ON CONFLICT (id) DO NOTHING;

INSERT INTO memberships (user_id, org_id, role)
VALUES
    ('01930b3c-7c5f-7000-8000-000000000001', '01930b3c-7c5f-7020-8000-00000000a001', 'admin'),
    ('01930b3c-7c5f-7001-8000-000000000002', '01930b3c-7c5f-7020-8000-00000000a002', 'admin'),
    ('01930b3c-7c5f-7001-8000-000000000099', '01930b3c-7c5f-7020-8000-00000000a099', 'admin'),
    ('01930b3c-7c5f-7001-8000-000000000322', '01930b3c-7c5f-7020-8000-00000000a322', 'admin')
ON CONFLICT DO NOTHING;

-- =============================================================================
-- Seed Projects
-- =============================================================================
INSERT INTO projects (
    id, name, user_id, description, cloud_provider, region,
    environment, status, budget_limit, tags, enabled, created_at, updated_at
)
VALUES
    (
        '01930b3c-7c5f-7002-8000-000000000003',
        'playground-monorepo',
        '01930b3c-7c5f-7001-8000-000000000002',
        'Main development playground for experimenting with Rust and Kubernetes',
        'aws',
        'us-east-1',
        'development',
        'active',
        100.0,
        '{}'::JSONB,
        true,
        NOW(),
        NOW()
    ),
    (
        '01930b3c-7c5f-7003-8000-000000000004',
        'zerg-api-production',
        '01930b3c-7c5f-7001-8000-000000000002',
        'Production deployment of Zerg API services',
        'aws',
        'us-west-2',
        'production',
        'active',
        500.0,
        '{"team": "platform", "criticality": "high"}'::JSONB,
        true,
        NOW(),
        NOW()
    ),
    (
        '01930b3c-7c5f-7004-8000-000000000005',
        'ml-training-cluster',
        '01930b3c-7c5f-7000-8000-000000000001',
        'Machine learning model training infrastructure',
        'gcp',
        'us-central1',
        'development',
        'provisioning',
        1000.0,
        '{"type": "ml", "gpu": "true"}'::JSONB,
        true,
        NOW(),
        NOW()
    )
ON CONFLICT (id) DO NOTHING;

-- =============================================================================
-- Seed Cloud Resources (using enum types)
-- =============================================================================
INSERT INTO cloud_resources (
    id, project_id, name, resource_type, status, region,
    configuration, cost_per_hour, monthly_cost_estimate, tags,
    enabled, created_at, updated_at, deleted_at
)
VALUES
    (
        '01930b3c-7c5f-7005-8000-000000000006',
        '01930b3c-7c5f-7002-8000-000000000003',
        'dev-postgres-primary',
        'database'::resource_type,
        'active'::resource_status,
        'us-east-1',
        '{"instance_type": "db.t3.medium", "engine": "postgres", "version": "15.3"}'::JSONB,
        0.068,
        48.96,
        '{"backup": "daily"}'::JSONB,
        true,
        NOW(),
        NOW(),
        NULL
    ),
    (
        '01930b3c-7c5f-7006-8000-000000000007',
        '01930b3c-7c5f-7002-8000-000000000003',
        'dev-redis-cache',
        'database'::resource_type,
        'active'::resource_status,
        'us-east-1',
        '{"instance_type": "cache.t3.micro", "engine": "redis", "version": "7.0"}'::JSONB,
        0.017,
        12.24,
        '{}'::JSONB,
        true,
        NOW(),
        NOW(),
        NULL
    ),
    (
        '01930b3c-7c5f-7007-8000-000000000008',
        '01930b3c-7c5f-7003-8000-000000000004',
        'prod-api-loadbalancer',
        'network'::resource_type,
        'active'::resource_status,
        'us-west-2',
        '{"type": "application", "scheme": "internet-facing", "ssl": true}'::JSONB,
        0.025,
        18.0,
        '{"public": "true"}'::JSONB,
        true,
        NOW(),
        NOW(),
        NULL
    ),
    (
        '01930b3c-7c5f-7008-8000-000000000009',
        '01930b3c-7c5f-7004-8000-000000000005',
        'ml-gpu-cluster',
        'compute'::resource_type,
        'creating'::resource_status,
        'us-central1',
        '{"instance_type": "n1-standard-16", "gpu": "nvidia-tesla-v100", "gpu_count": 4}'::JSONB,
        12.5,
        9000.0,
        '{"gpu": "v100", "count": "4"}'::JSONB,
        true,
        NOW(),
        NOW(),
        NULL
    )
ON CONFLICT (id) DO NOTHING;

-- Seed tasks moved to manifests/db/tasks/seed.sql along with the table.
