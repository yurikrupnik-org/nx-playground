import type { SourceKind } from '../../lib/data-map/types';

export const KIND_META: Record<
  SourceKind,
  { label: string; singular: string; badge: string; dot: string }
> = {
  database: {
    label: 'Databases',
    singular: 'Table',
    badge: 'bg-orange-100 text-orange-800',
    dot: 'bg-orange-500',
  },
  openapi: {
    label: 'APIs',
    singular: 'API model',
    badge: 'bg-indigo-100 text-indigo-800',
    dot: 'bg-indigo-500',
  },
  platform: {
    label: 'Platform',
    singular: 'Custom resource',
    badge: 'bg-emerald-100 text-emerald-800',
    dot: 'bg-emerald-500',
  },
  custom: {
    label: 'Your schemas',
    singular: 'JSON Schema',
    badge: 'bg-pink-100 text-pink-800',
    dot: 'bg-pink-500',
  },
};
