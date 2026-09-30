// These exercise the real N-API addon (Rust), not a stub: the point of the
// integration is that JS and the Rust services share one projection
// implementation, so a test against a fake would defend nothing.
import { describe, expect, it } from 'vitest';

import {
  allowedTodoFields,
  InvalidFieldsError,
  projectTodo,
  projectTodos,
  resolveRole,
  TODO_FIELDS,
} from './projection';

const todo = {
  id: 'a1',
  title: 'ship the addon',
  description: 'internal note',
  completed: false,
  priority: 'high',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-02T00:00:00Z',
};

describe('role gating', () => {
  it('hides admin-only fields from a normal caller', () => {
    const [projected] = projectTodos([todo], null, 'user');

    expect(Object.keys(projected as object).sort()).toEqual([
      'completed',
      'description',
      'id',
      'priority',
      'title',
    ]);
  });

  it('hides description from an anonymous caller', () => {
    const [projected] = projectTodos([todo], null, 'anonymous');

    expect(Object.keys(projected as object).sort()).toEqual([
      'completed',
      'id',
      'priority',
      'title',
    ]);
  });

  it('gives admin everything in the schema', () => {
    const [projected] = projectTodos([todo], null, 'admin');

    expect(Object.keys(projected as object).sort()).toEqual(
      [...TODO_FIELDS].sort(),
    );
  });

  it('defaults to the server-resolved role, never the caller', () => {
    // resolveRole() is deliberately independent of any request input.
    expect(resolveRole()).toBe('user');
    const [byDefault] = projectTodos([todo], null);
    const [asUser] = projectTodos([todo], null, 'user');

    expect(byDefault).toEqual(asUser);
  });
});

describe('sparse fieldsets', () => {
  it('trims to exactly what the island needs', () => {
    const [projected] = projectTodos([todo], 'id,title,completed,priority');

    expect(projected).toEqual({
      id: 'a1',
      title: 'ship the addon',
      completed: false,
      priority: 'high',
    });
  });

  it('applies role gating on top of the requested set', () => {
    // `created_at` is requested but above the role: dropped, not an error.
    const [projected] = projectTodos([todo], 'id,created_at', 'user');

    expect(projected).toEqual({ id: 'a1' });
  });

  it('tolerates the grammar the Rust selector defines (spaces, empties)', () => {
    const [projected] = projectTodos([todo], ' id , , title ');

    expect(Object.keys(projected as object).sort()).toEqual(['id', 'title']);
  });

  it('rejects an unknown field instead of ignoring it', () => {
    expect(() => projectTodos([todo], 'id,nope')).toThrow(InvalidFieldsError);
    expect(() => projectTodos([todo], 'id,nope')).toThrow(/nope/);
  });
});

describe('shape handling', () => {
  it('projects a single object for GET /api/todos/:id', () => {
    const projected = projectTodo(todo, 'id,title');

    expect(projected).toEqual({ id: 'a1', title: 'ship the addon' });
  });

  it('passes non-objects through untouched', () => {
    expect(projectTodo(null, null)).toBeNull();
    expect(projectTodo(42, null)).toBe(42);
  });

  it('projects every element of a list', () => {
    const projected = projectTodos([todo, { ...todo, id: 'a2' }], 'id');

    expect(projected).toEqual([{ id: 'a1' }, { id: 'a2' }]);
  });

  it('drops nothing it was never given', () => {
    // A row missing optional fields must not gain keys.
    const [projected] = projectTodos([{ id: 'a1' }], null, 'admin');

    expect(projected).toEqual({ id: 'a1' });
  });
});

describe('allowedTodoFields', () => {
  it('reports the readable set for a role, in declaration order', () => {
    expect(allowedTodoFields(null, 'anonymous')).toEqual([
      'id',
      'title',
      'completed',
      'priority',
    ]);
  });

  it('intersects the request with the role', () => {
    expect(allowedTodoFields('updated_at,title', 'user')).toEqual(['title']);
  });
});
