-- WorkOS AuthKit migration: credentials move to the IdP.
-- Drops the custom-auth columns and the oauth_accounts table; adds the IdP
-- subject linking column used by JIT provisioning.

-- Custom-auth indexes first (they reference the dropped columns).
DROP INDEX IF EXISTS idx_users_google_id;
DROP INDEX IF EXISTS idx_users_github_id;

ALTER TABLE users
  DROP COLUMN password_hash,
  DROP COLUMN is_locked,
  DROP COLUMN failed_login_attempts,
  DROP COLUMN locked_until,
  DROP COLUMN google_id,
  DROP COLUMN github_id,
  ADD COLUMN subject TEXT;

CREATE UNIQUE INDEX idx_users_subject ON users(subject) WHERE subject IS NOT NULL;

-- Provider tokens now live server-side in Redis sessions, keyed by the IdP.
DROP TABLE oauth_accounts;
