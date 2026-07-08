-- terran — local-dev seed (NOT applied in cluster). Superuser psql bypasses RLS.
-- Real orgs/users are JIT-provisioned on first login; this is demo data to view locally.

INSERT INTO organizations (id, external_org_id, name) VALUES
  ('00000000-0000-0000-0000-0000000000a1', 'org_local_demo', 'Demo Corp')
ON CONFLICT (external_org_id) DO NOTHING;

INSERT INTO users (id, subject, email, name) VALUES
  ('00000000-0000-0000-0000-0000000000b1', 'local-demo-subject', 'demo@terran.dev', 'Demo User')
ON CONFLICT (subject) DO NOTHING;

INSERT INTO memberships (user_id, org_id, role) VALUES
  (
    '00000000-0000-0000-0000-0000000000b1',
    '00000000-0000-0000-0000-0000000000a1',
    'org_admin'
  )
ON CONFLICT (user_id, org_id) DO NOTHING;

INSERT INTO cloud_assets
  (org_id, provider, external_id, name, asset_type, region, status, monthly_cost, metadata)
VALUES
  ('00000000-0000-0000-0000-0000000000a1', 'aws', 'i-0abc123', 'web-1',
   'ec2/m5.large', 'us-east-1', 'active', 70.50, '{"vcpus":2,"mem_gb":8}'),
  ('00000000-0000-0000-0000-0000000000a1', 'gcp', 'sql-prod-1', 'orders-db',
   'cloudsql/db-custom-4-16', 'us-central1', 'active', 240.00, '{"tier":"db-custom-4-16"}'),
  ('00000000-0000-0000-0000-0000000000a1', 'aws', 'vol-0def456', 'data-vol',
   'ebs/gp3', 'us-east-1', 'active', 12.00, '{"size_gb":150}')
ON CONFLICT (org_id, provider, external_id) DO NOTHING;
