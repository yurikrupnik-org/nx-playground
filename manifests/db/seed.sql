-- Seed data for local development
-- Applied after schema.sql via: just db-fresh or just db-seed

-- =============================================================================
-- Seed Users
-- =============================================================================
-- Password hash is bcrypt of "password123" for testing purposes
INSERT INTO users (id, email, name, password_hash, roles, email_verified, is_active, created_at, updated_at)
VALUES
    (
        '01930b3c-7c5f-7000-8000-000000000001',
        'admin@example.com',
        'Admin User',
        '$2a$12$LQv3c1yqBWVHxkd0LHAkCOYz6TtxMQJqhN8/X4.VTtYxFyL8rKYmK',
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
        '$2a$12$LQv3c1yqBWVHxkd0LHAkCOYz6TtxMQJqhN8/X4.VTtYxFyL8rKYmK',
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
        '$2a$12$LQv3c1yqBWVHxkd0LHAkCOYz6TtxMQJqhN8/X4.VTtYxFyL8rKYmK',
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
      '$2a$12$LQv3c1yqBWVHxkd0LHAkCOYz6TtxMQJqhN8/X4.VTtYxFyL8rKYmK',
      ARRAY['user', 'developer'],
      true,
      true,
      NOW(),
      NOW()
    )
ON CONFLICT (id) DO NOTHING;

-- =============================================================================
-- Seed OAuth Accounts (linked to developer user)
-- =============================================================================
INSERT INTO oauth_accounts (
    id, user_id, provider, provider_user_id, provider_username, email,
    display_name, created_at, updated_at
)
VALUES
    (
        '01930b3c-7c5f-7100-8000-000000000101',
        '01930b3c-7c5f-7001-8000-000000000099',
        'github',
        'gh_12345',
        'developer',
        'developer@example.com',
        'Developer User',
        NOW(),
        NOW()
    )
ON CONFLICT (id) DO NOTHING;

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

-- =============================================================================
-- Seed Tasks
-- =============================================================================
INSERT INTO tasks (
    id, title, description, project_id, priority, status,
    due_date, created_at, updated_at
)
VALUES
    (
        '01930b3c-7c5f-7009-8000-000000000010',
        'Setup CI/CD pipeline',
        'Configure GitHub Actions for automated testing and deployment',
        '01930b3c-7c5f-7002-8000-000000000003',
        'high'::task_priority,
        'in_progress'::task_status,
        NOW() + INTERVAL '7 days',
        NOW(),
        NOW()
    ),
    (
        '01930b3c-7c5f-700a-8000-000000000011',
        'Implement OAuth authentication',
        'Add Google and GitHub OAuth support with PKCE',
        '01930b3c-7c5f-7003-8000-000000000004',
        'high'::task_priority,
        'done'::task_status,
        NOW() - INTERVAL '2 days',
        NOW() - INTERVAL '5 days',
        NOW()
    ),
    (
        '01930b3c-7c5f-700b-8000-000000000012',
        'Database migration cleanup',
        'Consolidate and optimize database migrations',
        '01930b3c-7c5f-7002-8000-000000000003',
        'medium'::task_priority,
        'done'::task_status,
        NOW(),
        NOW() - INTERVAL '1 day',
        NOW()
    ),
    (
        '01930b3c-7c5f-700c-8000-000000000013',
        'Setup monitoring and alerts',
        'Configure Prometheus and Grafana for production monitoring',
        '01930b3c-7c5f-7003-8000-000000000004',
        'high'::task_priority,
        'todo'::task_status,
        NOW() + INTERVAL '14 days',
        NOW(),
        NOW()
    ),
    (
        '01930b3c-7c5f-700d-8000-000000000014',
        'Optimize API performance',
        'Profile and optimize slow API endpoints, add caching',
        '01930b3c-7c5f-7003-8000-000000000004',
        'medium'::task_priority,
        'todo'::task_status,
        NOW() + INTERVAL '21 days',
        NOW(),
        NOW()
    ),
    (
        '01930b3c-7c5f-700e-8000-000000000015',
        'ML model training infrastructure',
        'Setup distributed training pipeline with GPU cluster',
        '01930b3c-7c5f-7004-8000-000000000005',
        'urgent'::task_priority,
        'in_progress'::task_status,
        NOW() + INTERVAL '10 days',
        NOW() - INTERVAL '3 days',
        NOW()
    )
ON CONFLICT (id) DO NOTHING;
