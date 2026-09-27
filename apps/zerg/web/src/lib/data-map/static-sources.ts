// Repo documents bundled at build time — they are generated/committed files,
// so the page needs no API for them:
// - docs/openapi/*.json: every service's OpenAPI document (`task openapi-gen`,
//   drift-gated by `task openapi-check`).
// - platform/*/xrd.yaml: the Crossplane XRDs `task platform-install` applies.

import { crdSource, isCrd, openApiSource, parseDocuments } from './derive';
import type { Source } from './types';

const openApiDocs = import.meta.glob<Record<string, unknown>>(
  '../../../../../../docs/openapi/*.json',
  { eager: true, import: 'default' },
);

const platformDocs = import.meta.glob<string>(
  '../../../../../../platform/*/xrd.yaml',
  { eager: true, query: '?raw', import: 'default' },
);

/** Glob key → repo-relative path (`../../../docs/openapi/x.json` → `docs/openapi/x.json`). */
const repoPath = (path: string) => path.replace(/^(\.\.\/)+/, '');

export function openApiSources(): Source[] {
  return Object.entries(openApiDocs)
    .map(([path, doc]) => {
      const rel = repoPath(path);
      const source = openApiSource(`openapi:${rel}`, doc, rel);
      return {
        ...source,
        subtitle: [source.subtitle, rel].filter(Boolean).join(' · '),
      };
    })
    .sort((a, b) => a.title.localeCompare(b.title));
}

export function platformSources(): Source[] {
  return Object.entries(platformDocs)
    .map(([path, text]) => {
      const rel = repoPath(path);
      const docs = parseDocuments(text).filter(isCrd);
      const source = crdSource(`platform:${rel}`, docs, rel);
      return {
        ...source,
        subtitle: [source.subtitle, rel].filter(Boolean).join(' · '),
      };
    })
    .sort((a, b) => a.title.localeCompare(b.title));
}
