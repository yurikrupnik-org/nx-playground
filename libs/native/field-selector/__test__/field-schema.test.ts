import { describe, expect, it } from 'vitest';
import { type FieldRule, FieldSchema } from '../index.js';

const rules: FieldRule[] = [
  { field: 'id' },
  { field: 'title' },
  { field: 'assigneeEmail', requiredRole: 'user' },
  { field: 'auditTrail', requiredRole: 'admin' },
  { field: 'passwordHash', restricted: true },
];

const schema = new FieldSchema(rules);

const task = {
  id: 'a1',
  title: 'ship napi lib',
  assigneeEmail: 'yuri@example.com',
  auditTrail: ['created'],
  passwordHash: 'never-leaks',
};

describe('FieldSchema.allowedFields', () => {
  it('defaults to every field the role may read, in declaration order', () => {
    expect(schema.allowedFields()).toEqual(['id', 'title']);
    expect(schema.allowedFields(null, 'user')).toEqual([
      'id',
      'title',
      'assigneeEmail',
    ]);
    expect(schema.allowedFields(null, 'admin')).toEqual([
      'id',
      'title',
      'assigneeEmail',
      'auditTrail',
    ]);
  });

  it('parses the fields query grammar: trims and ignores empties', () => {
    expect(schema.allowedFields(' title , , id ')).toEqual(['id', 'title']);
  });

  it('drops fields above the caller role instead of throwing', () => {
    expect(schema.allowedFields('id,auditTrail', 'user')).toEqual(['id']);
  });

  it('throws on fields absent from the schema, listing them sorted', () => {
    expect(() => schema.allowedFields('id,nope,alsoNope')).toThrow(
      'invalid fields requested: alsoNope, nope',
    );
  });

  it('never yields a restricted field, even when requested by an admin', () => {
    expect(schema.allowedFields('passwordHash', 'admin')).toEqual([]);
  });
});

describe('FieldSchema.filter', () => {
  it('projects an object down to the allowed fields', () => {
    expect(schema.filter(task, 'id,title,passwordHash', 'admin')).toEqual({
      id: 'a1',
      title: 'ship napi lib',
    });
  });

  it('applies role gating to the projection', () => {
    expect(schema.filter(task, null, 'user')).toEqual({
      id: 'a1',
      title: 'ship napi lib',
      assigneeEmail: 'yuri@example.com',
    });
  });

  it('passes non-objects through untouched', () => {
    expect(schema.filter(42, 'id')).toBe(42);
    expect(schema.filter(null, 'id')).toBeNull();
  });

  it('returns keys alphabetically, not in input order', () => {
    expect(Object.keys(schema.filter(task, 'title,id') as object)).toEqual([
      'id',
      'title',
    ]);
  });
});

describe('FieldSchema.filterList', () => {
  it('resolves the schema once and projects every row', () => {
    const rows = [task, { ...task, id: 'a2' }];
    expect(schema.filterList(rows, 'id', 'anonymous')).toEqual([
      { id: 'a1' },
      { id: 'a2' },
    ]);
  });
});

describe('FieldSchema constructor', () => {
  it('rejects duplicate fields', () => {
    expect(() => new FieldSchema([{ field: 'id' }, { field: 'id' }])).toThrow(
      'duplicate field in schema: id',
    );
  });
});
