-- insights — Desired Database Schema (single source of truth)
-- CI / shell observability + developer insights, written by taskgraph_insights
-- (apps/taskgraph/insights, SQL in libs/domains/insights/src/store.rs).
--
-- Contract (docs/ci-insights.md): Grafana reads ONLY the `*_v` views and the
-- `scorecard` / `kind_scorecard` functions. Tables are private to the service
-- and may change shape without notice; the views keep their names and columns.
--
-- Every writer is an idempotent upsert keyed by a natural id (GitHub ids, commit
-- sha, taskgraph event_id / run_id / instance, scan_id), so re-running a sync
-- never duplicates a row.
--
-- Local dev: `task insights-db` (applies migrations/ to compose or kind).
-- Cluster:   Atlas Operator applies migrations via the insights-migrations ConfigMap.
-- PostgreSQL 18 (range_agg / multiranges need 14+).

-- =============================================================================
-- Sync ledger
-- =============================================================================

-- One row per sync stage (`runs`, `artifacts`, `commits`, `warehouse`,
-- `derived`, `traces`) plus `github`, which carries the rate-limit block.
CREATE TABLE sync_state (
    source text PRIMARY KEY,
    -- Stage-specific resume point (runs: last listing start; commits: newest
    -- committed_at seen). Each stage re-reads a small overlap behind it.
    cursor_at timestamptz,
    -- GitHub said stop (rate limit): skip GitHub stages until then.
    blocked_until timestamptz,
    last_success_at timestamptz,
    -- Items the last successful run of the stage touched.
    last_items bigint NOT NULL DEFAULT 0,
    last_error text,
    last_error_at timestamptz
);

-- Per-item failures that did not stop a stage (an undecodable NATS message, a
-- corrupt artifact). An append-only log, not data.
CREATE TABLE sync_errors (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    source text NOT NULL,
    subject text,
    error text NOT NULL,
    at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX sync_errors_source_at_idx ON sync_errors (source, at DESC);

-- GitHub Actions artifacts already consumed. A row means "never download
-- again", so transient failures are NOT recorded here (they retry next cycle);
-- permanent ones (expired, corrupt archive) are, with `error` set.
CREATE TABLE ingested_artifacts (
    artifact_id bigint PRIMARY KEY,
    run_id bigint NOT NULL,
    name text NOT NULL,
    kind text NOT NULL CHECK (kind IN ('events', 'shell_scan')),
    items int NOT NULL DEFAULT 0,
    malformed int NOT NULL DEFAULT 0,
    error text,
    ingested_at timestamptz NOT NULL DEFAULT now()
);

-- The synced repository (one per database).
CREATE TABLE repository (
    full_name text PRIMARY KEY,
    default_branch text NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now()
);

-- =============================================================================
-- GitHub Actions
-- =============================================================================

-- A workflow run attempt. The listing yields each run's latest attempt; the
-- sync also stores attempt 1 of re-run runs (first-pass rate needs it).
CREATE TABLE ci_run_attempts (
    run_id bigint NOT NULL,
    attempt int NOT NULL,
    workflow_id bigint NOT NULL,
    workflow text NOT NULL,
    workflow_path text NOT NULL,
    -- workflow_path is in INSIGHTS_CI_WORKFLOWS (re-evaluated every cycle).
    ci_workflow boolean NOT NULL DEFAULT false,
    run_number bigint NOT NULL,
    event text NOT NULL,
    branch text,
    head_sha text NOT NULL,
    status text NOT NULL,
    conclusion text,
    created_at timestamptz NOT NULL,
    started_at timestamptz,
    updated_at timestamptz NOT NULL,
    -- Latest job completion once jobs are synced; `updated_at` before that.
    completed_at timestamptz,
    actor text,
    triggering_actor text,
    html_url text NOT NULL,
    jobs_synced boolean NOT NULL DEFAULT false,
    artifacts_synced boolean NOT NULL DEFAULT false,
    -- Exported-trace ledger: set together, only after the OTLP export succeeded.
    trace_id text,
    trace_exported_at timestamptz,
    synced_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (run_id, attempt)
);

CREATE INDEX ci_run_attempts_head_sha_idx ON ci_run_attempts (head_sha);
CREATE INDEX ci_run_attempts_push_idx ON ci_run_attempts (event, branch, created_at);

CREATE TABLE ci_jobs (
    job_id bigint PRIMARY KEY,
    run_id bigint NOT NULL,
    attempt int NOT NULL,
    name text NOT NULL,
    status text NOT NULL,
    conclusion text,
    created_at timestamptz,
    started_at timestamptz,
    completed_at timestamptz,
    runner text,
    labels text[] NOT NULL DEFAULT '{}',
    html_url text,
    FOREIGN KEY (run_id, attempt) REFERENCES ci_run_attempts (run_id, attempt) ON DELETE CASCADE
);

CREATE INDEX ci_jobs_run_idx ON ci_jobs (run_id, attempt);

CREATE TABLE ci_steps (
    job_id bigint NOT NULL REFERENCES ci_jobs (job_id) ON DELETE CASCADE,
    number int NOT NULL,
    name text NOT NULL,
    status text NOT NULL,
    conclusion text,
    started_at timestamptz,
    completed_at timestamptz,
    PRIMARY KEY (job_id, number)
);

-- =============================================================================
-- Commits (classified by core_authorship)
-- =============================================================================

CREATE TABLE commits (
    sha text PRIMARY KEY,
    authored_at timestamptz NOT NULL,
    committed_at timestamptz NOT NULL,
    author_login text,
    author_name text NOT NULL,
    author_email text NOT NULL,
    -- core_authorship::Classification
    author text NOT NULL,
    author_kind text NOT NULL CHECK (author_kind IN ('human', 'agent', 'bot')),
    assistants text[] NOT NULL DEFAULT '{}',
    attribution text NOT NULL CHECK (attribution IN ('enforced', 'legacy')),
    message text NOT NULL,
    subject text NOT NULL,
    -- Conventional-commit type (`feat`, `fix`, …), lowercased; NULL otherwise.
    conv_type text,
    on_default_branch boolean NOT NULL DEFAULT false,
    -- NULL until the single-commit endpoint was read (details_synced).
    additions int,
    deletions int,
    files int,
    details_synced boolean NOT NULL DEFAULT false,
    -- Derived flags, recomputed by the `derived` stage.
    reverted boolean NOT NULL DEFAULT false,
    fixup_followed boolean NOT NULL DEFAULT false,
    html_url text,
    synced_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX commits_authored_at_idx ON commits (authored_at);
CREATE INDEX commits_committed_at_idx ON commits (committed_at);

CREATE TABLE commit_files (
    sha text NOT NULL REFERENCES commits (sha) ON DELETE CASCADE,
    path text NOT NULL,
    status text NOT NULL,
    additions int NOT NULL,
    deletions int NOT NULL,
    previous_path text,
    PRIMARY KEY (sha, path)
);

CREATE INDEX commit_files_path_idx ON commit_files (path);

-- Commit authors and credited assistants (agent ids).
CREATE TABLE contributors (
    contributor text PRIMARY KEY,
    kind text NOT NULL CHECK (kind IN ('human', 'agent', 'bot')),
    display_name text NOT NULL,
    first_seen timestamptz NOT NULL,
    last_seen timestamptz NOT NULL
);

-- =============================================================================
-- taskgraph warehouse (TASKGRAPH stream, durable consumer insights-warehouse)
-- =============================================================================

-- Applied-event ledger: an event_id is applied at most once, whatever the
-- stream redelivers or an artifact re-publishes.
CREATE TABLE tg_events (
    event_id uuid PRIMARY KEY,
    type text NOT NULL,
    run_id uuid,
    at timestamptz NOT NULL,
    stream_seq bigint,
    applied_at timestamptz NOT NULL DEFAULT now()
);

-- Columns are nullable: run_started and run_finished may arrive in any order.
CREATE TABLE task_runs (
    run_id uuid PRIMARY KEY,
    graph_id text,
    target text,
    args text[],
    host text,
    username text,
    cwd text,
    estimate_ms bigint,
    provider text,
    ci_run_id text,
    ci_attempt int,
    ci_job text,
    ci_pipeline text,
    ci_repository text,
    ci_ref text,
    ci_sha text,
    ci_event text,
    ci_actor text,
    ci_run_url text,
    invoker_kind text,
    agent text,
    origin_sha text,
    trace_id text,
    started_at timestamptz,
    finished_at timestamptz,
    outcome text,
    exit_code int,
    duration_ms bigint,
    error text
);

CREATE INDEX task_runs_ci_idx ON task_runs (ci_run_id, ci_attempt);
CREATE INDEX task_runs_ci_sha_idx ON task_runs (ci_sha);

CREATE TABLE task_executions (
    run_id uuid NOT NULL,
    instance int NOT NULL,
    task text NOT NULL,
    parent int,
    via text,
    started_at timestamptz,
    finished_at timestamptz,
    outcome text,
    duration_ms bigint,
    error text,
    PRIMARY KEY (run_id, instance)
);

CREATE INDEX task_executions_parent_idx ON task_executions (run_id, parent);

CREATE TABLE task_commands (
    event_id uuid PRIMARY KEY,
    run_id uuid NOT NULL,
    instance int NOT NULL,
    task text NOT NULL,
    command text NOT NULL,
    at timestamptz NOT NULL
);

CREATE INDEX task_commands_run_idx ON task_commands (run_id, instance);

-- =============================================================================
-- Static shell scans (contract_taskgraph::shell::ShellScan)
-- =============================================================================

CREATE TABLE shell_scans (
    scan_id uuid PRIMARY KEY,
    scanned_at timestamptz NOT NULL,
    sha text,
    tool text NOT NULL,
    provider text,
    ci_run_id text,
    ci_attempt int,
    ci_job text,
    artifact_id bigint,
    ingested_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX shell_scans_scanned_at_idx ON shell_scans (scanned_at);

CREATE TABLE shell_sources (
    scan_id uuid NOT NULL REFERENCES shell_scans (scan_id) ON DELETE CASCADE,
    source_id text NOT NULL,
    kind text NOT NULL,
    path text NOT NULL,
    task text,
    idx int,
    lines int NOT NULL,
    branches int NOT NULL,
    digest text NOT NULL,
    PRIMARY KEY (scan_id, source_id)
);

-- `ordinal` is the finding's position in the (sorted, immutable) scan.
CREATE TABLE shell_findings (
    scan_id uuid NOT NULL REFERENCES shell_scans (scan_id) ON DELETE CASCADE,
    ordinal int NOT NULL,
    source_id text NOT NULL,
    line int NOT NULL,
    col int NOT NULL,
    end_line int NOT NULL,
    end_col int NOT NULL,
    level text NOT NULL,
    code int NOT NULL,
    message text NOT NULL,
    PRIMARY KEY (scan_id, ordinal)
);

CREATE INDEX shell_findings_source_idx ON shell_findings (scan_id, source_id);

-- =============================================================================
-- Contract views
-- =============================================================================

-- queue_s: run start → first job start (waiting for runners).
-- ci_workflow (extra, trailing): the run counts toward the scorecard.
CREATE VIEW ci_runs_v AS
SELECT
    r.run_id,
    r.attempt,
    r.attempt = max(r.attempt) OVER (PARTITION BY r.run_id) AS is_latest,
    r.workflow,
    r.workflow_path,
    r.event,
    r.branch,
    r.head_sha,
    r.status,
    r.conclusion,
    r.created_at,
    r.started_at,
    r.completed_at,
    extract(epoch FROM r.completed_at - r.started_at)::float8 AS duration_s,
    extract(epoch FROM j.first_started_at - r.started_at)::float8 AS queue_s,
    r.actor,
    r.html_url,
    c.author,
    c.author_kind,
    r.trace_id,
    r.ci_workflow
FROM ci_run_attempts r
LEFT JOIN commits c ON c.sha = r.head_sha
LEFT JOIN LATERAL (
    SELECT min(cj.started_at) AS first_started_at
    FROM ci_jobs cj
    WHERE cj.run_id = r.run_id AND cj.attempt = r.attempt
) j ON true;

CREATE VIEW ci_jobs_v AS
SELECT
    j.run_id,
    j.attempt,
    j.job_id,
    r.workflow,
    j.name AS job,
    j.status,
    j.conclusion,
    j.started_at,
    j.completed_at,
    extract(epoch FROM j.completed_at - j.started_at)::float8 AS duration_s,
    extract(epoch FROM j.started_at - j.created_at)::float8 AS queue_s,
    j.runner,
    j.html_url
FROM ci_jobs j
JOIN ci_run_attempts r ON r.run_id = j.run_id AND r.attempt = j.attempt;

CREATE VIEW ci_steps_v AS
SELECT
    j.run_id,
    j.attempt,
    s.job_id,
    r.workflow,
    j.name AS job,
    s.name AS step,
    s.number,
    s.conclusion,
    s.started_at,
    s.completed_at,
    extract(epoch FROM s.completed_at - s.started_at)::float8 AS duration_s
FROM ci_steps s
JOIN ci_jobs j ON j.job_id = s.job_id
JOIN ci_run_attempts r ON r.run_id = j.run_id AND r.attempt = j.attempt;

CREATE VIEW task_runs_v AS
SELECT
    t.run_id,
    CASE WHEN t.provider IS NULL THEN 'local' ELSE 'ci' END AS source,
    t.provider,
    t.ci_run_id,
    t.ci_attempt,
    t.ci_job,
    t.host,
    t.username,
    t.invoker_kind,
    t.agent,
    t.target,
    t.outcome,
    t.exit_code,
    t.started_at,
    t.finished_at,
    COALESCE(t.duration_ms, (extract(epoch FROM t.finished_at - t.started_at) * 1000)::bigint) AS duration_ms,
    COALESCE(t.ci_sha, t.origin_sha) AS sha
FROM task_runs t;

-- self_ms: inclusive duration minus the union of the children's intervals
-- (deps run concurrently, so their durations do not add up).
CREATE VIEW task_executions_v AS
SELECT
    e.run_id,
    CASE WHEN t.provider IS NULL THEN 'local' ELSE 'ci' END AS source,
    t.provider,
    t.ci_run_id,
    t.ci_attempt,
    t.ci_job,
    t.invoker_kind,
    t.agent,
    t.target,
    e.task,
    e.instance,
    e.parent,
    e.via,
    e.outcome,
    COALESCE(e.started_at, e.finished_at - make_interval(secs => e.duration_ms / 1000.0)) AS started_at,
    e.finished_at,
    e.duration_ms,
    CASE WHEN e.duration_ms IS NOT NULL THEN
        greatest(0, e.duration_ms - COALESCE((extract(epoch FROM ch.busy) * 1000)::bigint, 0))
    END AS self_ms,
    e.error,
    COALESCE(t.ci_sha, t.origin_sha) AS sha
FROM task_executions e
LEFT JOIN task_runs t ON t.run_id = e.run_id
LEFT JOIN LATERAL (
    SELECT sum(upper(span) - lower(span)) AS busy
    FROM unnest((
        SELECT range_agg(tstzrange(k.s, k.f))
        FROM (
            SELECT
                COALESCE(c.started_at, c.finished_at - make_interval(secs => c.duration_ms / 1000.0)) AS s,
                c.finished_at AS f
            FROM task_executions c
            WHERE c.run_id = e.run_id AND c.parent = e.instance
        ) k
        WHERE k.s IS NOT NULL AND k.f IS NOT NULL AND k.f >= k.s
    )) AS span
) ch ON true;

-- first_ci_conclusion: attempt 1 of the earliest counted run for the commit.
-- lead_time_h: authored → completion of the first successful counted `push` run
-- on the default branch created at/after the commit (default-branch commits only).
CREATE VIEW commits_v AS
SELECT
    c.sha,
    c.authored_at,
    c.committed_at,
    c.author,
    c.author_kind,
    CASE WHEN c.author_kind = 'agent' THEN c.author END AS author_agent,
    c.assistants,
    c.attribution,
    c.on_default_branch,
    c.subject,
    c.conv_type,
    c.additions,
    c.deletions,
    c.files,
    c.reverted,
    c.fixup_followed,
    f.conclusion AS first_ci_conclusion,
    extract(epoch FROM l.completed_at - c.authored_at)::float8 / 3600 AS lead_time_h
FROM commits c
LEFT JOIN LATERAL (
    SELECT r.conclusion
    FROM ci_run_attempts r
    WHERE r.head_sha = c.sha AND r.ci_workflow AND r.attempt = 1
    ORDER BY r.created_at, r.run_id
    LIMIT 1
) f ON true
LEFT JOIN LATERAL (
    SELECT r.completed_at
    FROM ci_run_attempts r
    WHERE c.on_default_branch
        AND r.ci_workflow
        AND r.event = 'push'
        AND r.branch = (SELECT rp.default_branch FROM repository rp LIMIT 1)
        AND r.conclusion = 'success'
        AND r.completed_at IS NOT NULL
        AND r.created_at >= c.committed_at
    ORDER BY r.created_at, r.run_id, r.attempt
    LIMIT 1
) l ON true;

CREATE VIEW contributors_v AS
SELECT contributor, kind, display_name, first_seen, last_seen
FROM contributors;

CREATE VIEW shell_scans_v AS
SELECT
    s.scan_id,
    s.scanned_at,
    s.sha,
    CASE WHEN s.provider IS NULL THEN 'local' ELSE 'ci' END AS origin,
    src.sources,
    src.lines,
    src.branches,
    fsum.findings,
    fsum.errors,
    fsum.warnings,
    fsum.infos,
    fsum.styles
FROM shell_scans s
CROSS JOIN LATERAL (
    SELECT
        count(*)::int AS sources,
        COALESCE(sum(x.lines), 0)::int AS lines,
        COALESCE(sum(x.branches), 0)::int AS branches
    FROM shell_sources x
    WHERE x.scan_id = s.scan_id
) src
CROSS JOIN LATERAL (
    SELECT
        count(*)::int AS findings,
        (count(*) FILTER (WHERE y.level = 'error'))::int AS errors,
        (count(*) FILTER (WHERE y.level = 'warning'))::int AS warnings,
        (count(*) FILTER (WHERE y.level = 'info'))::int AS infos,
        (count(*) FILTER (WHERE y.level = 'style'))::int AS styles
    FROM shell_findings y
    WHERE y.scan_id = s.scan_id
) fsum;

-- owner: the contributor with the most changed lines on the source's path in
-- default-branch commits up to the scan (ties: most recent), NULL when unknown.
CREATE VIEW shell_sources_v AS
SELECT
    s.scan_id,
    s.scanned_at,
    s.sha,
    x.source_id,
    x.kind,
    x.path,
    x.task,
    x.lines,
    x.branches,
    (SELECT count(*) FROM shell_findings y WHERE y.scan_id = x.scan_id AND y.source_id = x.source_id)::int AS findings,
    o.owner,
    o.owner_kind
FROM shell_sources x
JOIN shell_scans s ON s.scan_id = x.scan_id
LEFT JOIN LATERAL (
    SELECT c.author AS owner, c.author_kind AS owner_kind
    FROM commit_files cf
    JOIN commits c ON c.sha = cf.sha
    WHERE cf.path = x.path AND c.on_default_branch AND c.committed_at <= s.scanned_at
    GROUP BY c.author, c.author_kind
    ORDER BY sum(cf.additions + cf.deletions) DESC, max(c.committed_at) DESC, c.author
    LIMIT 1
) o ON true;

CREATE VIEW shell_findings_v AS
SELECT
    v.scan_id,
    v.scanned_at,
    v.sha,
    v.source_id,
    v.kind,
    v.path,
    v.task,
    f.line,
    f.level,
    f.code,
    f.message,
    v.owner
FROM shell_findings f
JOIN shell_sources_v v ON v.scan_id = f.scan_id AND v.source_id = f.source_id;

-- items: rows the stage has produced so far.
CREATE VIEW sync_status_v AS
SELECT
    st.source,
    st.last_success_at,
    st.last_error,
    st.last_error_at,
    (CASE st.source
        WHEN 'runs' THEN (SELECT count(*) FROM ci_run_attempts)
        WHEN 'artifacts' THEN (SELECT count(*) FROM ingested_artifacts)
        WHEN 'commits' THEN (SELECT count(*) FROM commits)
        WHEN 'warehouse' THEN (SELECT count(*) FROM tg_events)
        WHEN 'derived' THEN (SELECT count(*) FROM commits WHERE reverted OR fixup_followed)
        WHEN 'traces' THEN (SELECT count(*) FROM ci_run_attempts WHERE trace_exported_at IS NOT NULL)
        ELSE st.last_items
    END)::bigint AS items
FROM sync_state st;

-- =============================================================================
-- Scorecard (metric definitions: docs/ci-insights.md, Contract 4)
-- =============================================================================

-- Per-commit outcome inputs (internal; the functions below aggregate it).
-- first_pass: every counted run's completed attempt 1 succeeded (NULL: no run).
-- task_failed / task_decided: CI task executions at this sha, attempt 1.
CREATE VIEW commit_outcomes AS
SELECT
    v.sha,
    v.authored_at,
    v.author,
    v.author_kind,
    v.assistants,
    v.attribution,
    v.additions,
    v.deletions,
    v.reverted,
    v.fixup_followed,
    v.lead_time_h,
    fp.first_pass,
    COALESCE(tf.failed, 0) AS task_failed,
    COALESCE(tf.decided, 0) AS task_decided
FROM commits_v v
LEFT JOIN (
    SELECT r.head_sha, bool_and(r.conclusion = 'success') AS first_pass
    FROM ci_run_attempts r
    WHERE r.ci_workflow AND r.attempt = 1 AND r.status = 'completed'
    GROUP BY r.head_sha
) fp ON fp.head_sha = v.sha
LEFT JOIN (
    SELECT
        COALESCE(t.ci_sha, t.origin_sha) AS sha,
        count(*) FILTER (WHERE e.outcome = 'failed') AS failed,
        count(*) FILTER (WHERE e.outcome IN ('succeeded', 'failed')) AS decided
    FROM task_runs t
    JOIN task_executions e ON e.run_id = t.run_id
    WHERE t.provider IS NOT NULL AND t.ci_attempt = 1
    GROUP BY 1
) tf ON tf.sha = v.sha;

-- outcome_score = 100 × (0.30·first_pass + 0.25·(1−change_failure)
--   + 0.15·(1−fixup) + 0.15·lead_score + 0.15·(1−task_failure)),
-- lead_score = 1/(1+lead_time_h/24). A NULL metric takes the team value; a
-- metric NULL for the team too drops out and the other weights renormalise.
CREATE FUNCTION insights_outcome_score(
    fp float8, cf float8, fx float8, lt float8, tf float8,
    team_fp float8, team_cf float8, team_fx float8, team_lt float8, team_tf float8
) RETURNS float8
LANGUAGE sql IMMUTABLE
AS $$
SELECT round((100 * sum(x.weight * x.goodness) / nullif(sum(x.weight), 0))::numeric, 2)::float8
FROM (VALUES
    (0.30::float8, COALESCE(fp, team_fp)),
    (0.25::float8, 1 - COALESCE(cf, team_cf)),
    (0.15::float8, 1 - COALESCE(fx, team_fx)),
    (0.15::float8, 1 / (1 + COALESCE(lt, team_lt) / 24)),
    (0.15::float8, 1 - COALESCE(tf, team_tf))
) AS x (weight, goodness)
WHERE x.goodness IS NOT NULL
$$;

-- One row per contributor whose commit set (authored ∪ assisted) intersects
-- the window. Outcome metrics rank; activity columns are context only.
CREATE FUNCTION scorecard(p_from timestamptz, p_to timestamptz, p_min_commits int DEFAULT 5)
RETURNS TABLE (
    rank int,
    contributor text,
    kind text,
    commits int,
    authored int,
    assisted int,
    additions int,
    deletions int,
    active_days int,
    ci_runs int,
    first_pass_rate float8,
    change_failure_rate float8,
    fixup_rate float8,
    lead_time_h float8,
    task_failure_rate float8,
    outcome_score float8,
    sample_ok bool,
    why text
)
LANGUAGE sql STABLE
AS $$
WITH w AS (
    SELECT o.*
    FROM commit_outcomes o
    WHERE o.authored_at >= p_from AND o.authored_at < p_to
),
team AS (
    SELECT
        avg(w.first_pass::int)::float8 AS fp,
        avg(w.reverted::int)::float8 AS cf,
        avg(w.fixup_followed::int)::float8 AS fx,
        percentile_cont(0.5) WITHIN GROUP (ORDER BY w.lead_time_h) AS lt,
        sum(w.task_failed)::float8 / nullif(sum(w.task_decided), 0) AS tf
    FROM w
),
m AS (
    SELECT w.author AS member, true AS is_author, w.*
    FROM w
    UNION ALL
    SELECT a.agent, false, w.*
    FROM w
    CROSS JOIN LATERAL unnest(w.assistants) AS a (agent)
    WHERE a.agent <> w.author
),
per AS (
    SELECT
        m.member,
        count(DISTINCT m.sha)::int AS n_commits,
        (count(*) FILTER (WHERE m.is_author))::int AS n_authored,
        (count(*) FILTER (WHERE NOT m.is_author))::int AS n_assisted,
        COALESCE(sum(m.additions), 0)::int AS n_additions,
        COALESCE(sum(m.deletions), 0)::int AS n_deletions,
        count(DISTINCT (m.authored_at AT TIME ZONE 'UTC')::date)::int AS n_active_days,
        avg(m.first_pass::int)::float8 AS fp,
        avg(m.reverted::int)::float8 AS cf,
        avg(m.fixup_followed::int)::float8 AS fx,
        percentile_cont(0.5) WITHIN GROUP (ORDER BY m.lead_time_h) AS lt,
        sum(m.task_failed)::float8 / nullif(sum(m.task_decided), 0) AS tf,
        COALESCE(bool_and(m.attribution = 'legacy') FILTER (WHERE m.is_author), false) AS legacy_only
    FROM m
    GROUP BY m.member
),
runs AS (
    SELECT m.member, count(DISTINCT r.run_id)::int AS n_runs
    FROM m
    JOIN ci_run_attempts r ON r.head_sha = m.sha AND r.ci_workflow
    GROUP BY m.member
),
scored AS (
    SELECT
        p.*,
        COALESCE(k.kind, 'unknown') AS member_kind,
        COALESCE(rn.n_runs, 0) AS n_runs,
        p.n_commits >= p_min_commits AS ok,
        insights_outcome_score(p.fp, p.cf, p.fx, p.lt, p.tf, t.fp, t.cf, t.fx, t.lt, t.tf) AS score,
        d.pos,
        d.neg
    FROM per p
    CROSS JOIN team t
    LEFT JOIN contributors k ON k.contributor = p.member
    LEFT JOIN runs rn ON rn.member = p.member
    CROSS JOIN LATERAL (
        SELECT
            array_agg(y.txt ORDER BY y.margin DESC, y.metric) FILTER (WHERE y.margin > 0) AS pos,
            array_agg(y.txt ORDER BY y.margin, y.metric) FILTER (WHERE y.margin < 0) AS neg
        FROM (
            SELECT
                x.metric,
                round(x.margin::numeric, 6) AS margin,
                x.metric || ' '
                    || CASE WHEN x.unit = 'h' THEN round(x.v::numeric, 1) || 'h' ELSE round((x.v * 100)::numeric) || '%' END
                    || ' vs team '
                    || CASE WHEN x.unit = 'h' THEN round(x.tv::numeric, 1) || 'h' ELSE round((x.tv * 100)::numeric) || '%' END
                    AS txt
            FROM (VALUES
                ('first_pass_rate', 'pct', p.fp, t.fp, p.fp - t.fp),
                ('change_failure_rate', 'pct', p.cf, t.cf, t.cf - p.cf),
                ('fixup_rate', 'pct', p.fx, t.fx, t.fx - p.fx),
                ('lead_time_h', 'h', p.lt, t.lt, 1 / (1 + p.lt / 24) - 1 / (1 + t.lt / 24)),
                ('task_failure_rate', 'pct', p.tf, t.tf, t.tf - p.tf)
            ) AS x (metric, unit, v, tv, margin)
            WHERE x.margin IS NOT NULL
        ) y
    ) d
)
SELECT
    (CASE WHEN s.ok THEN dense_rank() OVER (PARTITION BY s.ok ORDER BY s.score DESC NULLS LAST) END)::int,
    s.member,
    s.member_kind,
    s.n_commits,
    s.n_authored,
    s.n_assisted,
    s.n_additions,
    s.n_deletions,
    s.n_active_days,
    s.n_runs,
    s.fp,
    s.cf,
    s.fx,
    s.lt,
    s.tf,
    s.score,
    s.ok,
    CASE
        WHEN s.ok THEN 'strengths: ' || COALESCE(array_to_string(s.pos[1:2], ', '), 'none')
            || '; weakest: ' || COALESCE(s.neg[1], 'none')
        ELSE format('insufficient data: %s commits (< %s)', s.n_commits, p_min_commits)
    END
    || CASE WHEN s.member_kind = 'human' AND s.legacy_only THEN ' (legacy attribution: agent share unknown)' ELSE '' END
FROM scored s
ORDER BY 1 NULLS LAST, s.score DESC NULLS LAST, s.member
$$;

-- The window's commits partitioned by who wrote them: human (no credited
-- agent, attribution enforced), human+agent, agent, bot, and unknown (human
-- author, no trailer, legacy attribution — "no trailer" proves nothing there).
CREATE FUNCTION kind_scorecard(p_from timestamptz, p_to timestamptz)
RETURNS TABLE (
    kind text,
    contributors int,
    commits int,
    additions int,
    deletions int,
    first_pass_rate float8,
    change_failure_rate float8,
    fixup_rate float8,
    lead_time_h float8,
    task_failure_rate float8,
    outcome_score float8
)
LANGUAGE sql STABLE
AS $$
WITH w AS (
    SELECT o.*
    FROM commit_outcomes o
    WHERE o.authored_at >= p_from AND o.authored_at < p_to
),
team AS (
    SELECT
        avg(w.first_pass::int)::float8 AS fp,
        avg(w.reverted::int)::float8 AS cf,
        avg(w.fixup_followed::int)::float8 AS fx,
        percentile_cont(0.5) WITHIN GROUP (ORDER BY w.lead_time_h) AS lt,
        sum(w.task_failed)::float8 / nullif(sum(w.task_decided), 0) AS tf
    FROM w
),
b AS (
    SELECT
        CASE
            WHEN w.author_kind = 'bot' THEN 'bot'
            WHEN w.author_kind = 'agent' THEN 'agent'
            WHEN cardinality(w.assistants) > 0 THEN 'human+agent'
            WHEN w.attribution = 'legacy' THEN 'unknown'
            ELSE 'human'
        END AS bucket,
        w.*
    FROM w
),
per AS (
    SELECT
        b.bucket,
        count(DISTINCT b.author)::int AS n_contributors,
        count(*)::int AS n_commits,
        COALESCE(sum(b.additions), 0)::int AS n_additions,
        COALESCE(sum(b.deletions), 0)::int AS n_deletions,
        avg(b.first_pass::int)::float8 AS fp,
        avg(b.reverted::int)::float8 AS cf,
        avg(b.fixup_followed::int)::float8 AS fx,
        percentile_cont(0.5) WITHIN GROUP (ORDER BY b.lead_time_h) AS lt,
        sum(b.task_failed)::float8 / nullif(sum(b.task_decided), 0) AS tf
    FROM b
    GROUP BY b.bucket
)
SELECT
    p.bucket,
    p.n_contributors,
    p.n_commits,
    p.n_additions,
    p.n_deletions,
    p.fp,
    p.cf,
    p.fx,
    p.lt,
    p.tf,
    insights_outcome_score(p.fp, p.cf, p.fx, p.lt, p.tf, t.fp, t.cf, t.fx, t.lt, t.tf)
FROM per p
CROSS JOIN team t
ORDER BY array_position(ARRAY['human', 'human+agent', 'agent', 'bot', 'unknown'], p.bucket)
$$;
