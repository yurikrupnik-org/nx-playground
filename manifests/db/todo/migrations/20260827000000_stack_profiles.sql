-- Frontend stack profiles for the todo vertical comparison page.
--
-- Reference data answering "which UI language is the default and which is
-- cheapest to ship", measured against the production build of todo web-astro
-- (raw transfer, uncompressed, 2026-08-27). Served by todo-api at
-- GET /api/stacks and rendered by web-astro's landing page.
--
-- The cluster DB (CNPG + Atlas migrations ConfigMap) is the source of truth:
-- change values via a follow-up migration, not by hand.

CREATE TABLE IF NOT EXISTS stack_profiles (
    slug       VARCHAR(64) PRIMARY KEY,
    name       VARCHAR(128) NOT NULL,
    language   VARCHAR(128) NOT NULL,
    is_default BOOLEAN NOT NULL DEFAULT false,
    js_kb      REAL NOT NULL,      -- client JS transfer at first render
    html_kb    REAL NOT NULL,      -- initial document transfer
    requests   INTEGER NOT NULL,   -- subresource requests to first render
    notes      TEXT NOT NULL DEFAULT '',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- At most one stack may be flagged as the default.
CREATE UNIQUE INDEX IF NOT EXISTS idx_stack_profiles_single_default
    ON stack_profiles ((true)) WHERE is_default;

INSERT INTO stack_profiles (slug, name, language, is_default, js_kb, html_kb, requests, notes) VALUES
    ('solid-island', 'Solid island', 'TypeScript (SolidJS)', true, 26.3, 8.3, 5,
     'Workspace default: every web app in this repo is Solid. Typed end-to-end via ts-rs DTOs.'),
    ('htmx', 'HTML + htmx', 'HTML (htmx over the API)', false, 58.9, 5.0, 1,
     'Zero app JS; the server renders fragments. htmx itself is ~16 kB once gzipped at the ingress.'),
    ('static-html', 'Static HTML', 'HTML + CSS only', false, 0, 6.8, 0,
     'No interactivity: cheapest possible output and the baseline for the other two.')
ON CONFLICT (slug) DO NOTHING;
