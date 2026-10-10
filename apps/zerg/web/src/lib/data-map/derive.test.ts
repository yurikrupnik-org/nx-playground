import { describe, expect, it } from 'vitest';
import {
  documentSource,
  openApiSource,
  refEdges,
  sampleFromSchema,
  tableToJsonSchema,
} from './derive';
import type { CatalogTable } from './types';

describe('refEdges', () => {
  it('maps deep $refs to the owning definition and drops self references', () => {
    const doc = {
      $defs: {
        Order: {
          type: 'object',
          properties: {
            parent: { $ref: '#/$defs/Order' },
            line: { $ref: '#/$defs/Line/properties/sku' },
          },
        },
        Line: { type: 'object', properties: { sku: { type: 'string' } } },
      },
    };
    const source = documentSource('s', 'orders', JSON.stringify(doc));
    const labels = new Map(source.nodes.map((n) => [n.id, n.label]));
    expect(
      source.edges.map((e) => [labels.get(e.from), labels.get(e.to)]),
    ).toEqual([['Order', 'Line']]);
  });

  it('does not let the root node claim edges owned by its definitions', () => {
    const source = documentSource(
      's',
      'root',
      JSON.stringify({
        properties: { a: { $ref: '#/$defs/A' } },
        $defs: { A: { properties: { b: { $ref: '#/$defs/B' } } }, B: {} },
      }),
    );
    const labels = new Map(source.nodes.map((n) => [n.id, n.label]));
    expect(
      refEdges(source.nodes).map(
        (e) => `${labels.get(e.from)}->${labels.get(e.to)}`,
      ),
    ).toEqual(['root->A', 'A->B']);
  });
});

describe('tableToJsonSchema', () => {
  const table: CatalogTable = {
    schema: 'public',
    name: 'tasks',
    kind: 'table',
    comment: null,
    row_estimate: 3,
    columns: [
      col('id', 'uuid', { is_primary_key: true, default: 'uuidv7()' }),
      col('title', 'character varying(200)'),
      col('project_id', 'uuid', { nullable: true }),
      col('tags', 'text[]'),
      col('status', 'task_status', { enum_values: ['todo', 'done'] }),
      col('meta', 'jsonb', { nullable: true }),
    ],
    foreign_keys: [
      {
        name: 'tasks_project_id_fkey',
        columns: ['project_id'],
        ref_schema: 'public',
        ref_table: 'projects',
        ref_columns: ['id'],
      },
    ],
  };
  const schema = tableToJsonSchema(table);

  it('requires only NOT NULL columns without a default', () => {
    expect(schema.required).toEqual(['title', 'tags', 'status']);
  });

  it('maps pg types, nullability, enums and foreign keys', () => {
    const p = schema.properties ?? {};
    expect(p.title).toMatchObject({ type: 'string', maxLength: 200 });
    expect(p.project_id).toMatchObject({
      type: ['string', 'null'],
      format: 'uuid',
      'x-references': 'public.projects.id',
    });
    expect(p.tags).toMatchObject({ type: 'array', items: { type: 'string' } });
    expect(p.status).toMatchObject({ type: 'string', enum: ['todo', 'done'] });
    // Untyped (any JSON) columns carry nullability without inventing a type.
    expect(p.meta).toMatchObject({ nullable: true });
    expect(p.meta.type).toBeUndefined();
  });
});

describe('openApiSource', () => {
  it('reaches schemas used by an operation transitively through $refs', () => {
    const source = openApiSource(
      'api',
      {
        openapi: '3.1.0',
        info: { title: 'T', version: '1' },
        paths: {
          '/tasks': {
            get: {
              responses: {
                200: {
                  content: {
                    'application/json': {
                      schema: { $ref: '#/components/schemas/TaskPage' },
                    },
                  },
                },
              },
            },
          },
        },
        components: {
          schemas: {
            TaskPage: {
              properties: {
                items: { items: { $ref: '#/components/schemas/Task' } },
              },
            },
            Task: { properties: { id: { type: 'string' } } },
            Unused: {},
          },
        },
      },
      'fallback',
    );
    const [op] = source.operations ?? [];
    const reached = source.nodes
      .filter((n) => op.refs.has(n.id))
      .map((n) => n.label);
    expect(reached.sort()).toEqual(['Task', 'TaskPage']);
  });
});

describe('documentSource', () => {
  it('reads a Crossplane XRD from YAML as one node per version', () => {
    const source = documentSource(
      'x',
      'xrd.yaml',
      [
        'apiVersion: apiextensions.crossplane.io/v1',
        'kind: CompositeResourceDefinition',
        'spec:',
        '  group: platform.example.io',
        '  names: { kind: XDb }',
        '  claimNames: { kind: Db }',
        '  versions:',
        '    - name: v1alpha1',
        '      served: true',
        '      schema:',
        '        openAPIV3Schema:',
        '          type: object',
        '          properties: { spec: { type: object } }',
      ].join('\n'),
    );
    expect(source.kind).toBe('platform');
    expect(source.title).toBe('Db');
    expect(source.nodes.map((n) => n.label)).toEqual(['XDb v1alpha1']);
    expect(source.nodes[0].schema.properties).toHaveProperty('spec');
  });

  it('rejects YAML syntax errors instead of returning an empty source', () => {
    expect(() => documentSource('x', 'bad', 'a: [1, 2')).toThrow();
  });
});

describe('sampleFromSchema', () => {
  it('terminates on recursive schemas', () => {
    const root = {
      $defs: {
        Node: {
          type: 'object',
          properties: {
            name: { type: 'string', format: 'email' },
            children: { type: 'array', items: { $ref: '#/$defs/Node' } },
          },
        },
      },
    };
    expect(sampleFromSchema({ $ref: '#/$defs/Node' }, root)).toEqual({
      name: 'user@example.com',
      children: [{}],
    });
  });
});

function col(
  name: string,
  data_type: string,
  extra: Partial<CatalogTable['columns'][number]> = {},
): CatalogTable['columns'][number] {
  return {
    name,
    position: 0,
    data_type,
    nullable: false,
    default: null,
    is_primary_key: false,
    comment: null,
    enum_values: null,
    ...extra,
  };
}
