-- Seed data for local development of the tasks service.
-- Applied after schema.sql via: just db-fresh tasks
--
-- `org_ref`/`user_ref` are identity-provider references, so seed rows use clearly
-- synthetic refs. Real logins derive their own refs from the verified token
-- (`org_01...` / `user_01...` / `personal:{subject}`) and will not see these rows -
-- that is the tenant isolation working, not a bug.

INSERT INTO tasks (
    id, title, description, completed, org_ref, user_ref, project_id, priority, status,
    due_date, created_at, updated_at
)
VALUES
    (
        '01930b3c-7c5f-7009-8000-000000000010',
        'Setup CI/CD pipeline',
        'Configure GitHub Actions for automated testing and deployment',
        false,
        'personal:user_seed_admin',
        'user_seed_admin',
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
        true,
        'personal:user_seed_dev',
        'user_seed_dev',
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
        true,
        'personal:user_seed_admin',
        'user_seed_admin',
        NULL,
        'medium'::task_priority,
        'done'::task_status,
        NOW() - INTERVAL '1 day',
        NOW() - INTERVAL '3 days',
        NOW()
    ),
    -- An org-scoped pair: both members of `org_seed_acme` see both rows, and the
    -- "mine" filter narrows to one. Exercises the B2B path locally.
    (
        '01930b3c-7c5f-700c-8000-000000000013',
        'Draft Q3 architecture review',
        'Shared org task - visible to every member of the seeded organization',
        false,
        'org_seed_acme',
        'user_seed_admin',
        NULL,
        'urgent'::task_priority,
        'todo'::task_status,
        NOW() + INTERVAL '3 days',
        NOW(),
        NOW()
    ),
    (
        '01930b3c-7c5f-700d-8000-000000000014',
        'Audit dependency licences',
        'Shared org task owned by the second seeded member',
        false,
        'org_seed_acme',
        'user_seed_dev',
        NULL,
        'low'::task_priority,
        'todo'::task_status,
        NOW() + INTERVAL '14 days',
        NOW(),
        NOW()
    )
ON CONFLICT (id) DO NOTHING;
