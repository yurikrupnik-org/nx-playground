// The htmx wire format IS these fragments: escaping and the swap contract
// (#todo-list id, hx-target) are load-bearing.
import type { Todo } from '@domain/todo';
import { describe, expect, it } from 'vitest';

import { escapeHtml, renderTodoItem, renderTodoList } from './fragments';

const todo = (overrides: Partial<Todo> = {}): Todo => ({
  id: '00000000-0000-0000-0000-000000000001',
  title: 'buy milk',
  description: '',
  completed: false,
  priority: 'medium',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
  ...overrides,
});

describe('escapeHtml', () => {
  it('escapes markup-significant characters', () => {
    expect(escapeHtml(`<img src=x onerror="pwn('&')">`)).toBe(
      '&lt;img src=x onerror=&quot;pwn(&#39;&amp;&#39;)&quot;&gt;',
    );
  });
});

describe('renderTodoItem', () => {
  it('escapes user titles in text and attribute positions', () => {
    const html = renderTodoItem(todo({ title: '<script>"x"</script>' }));
    expect(html).not.toContain('<script>');
    expect(html).toContain('&lt;script&gt;&quot;x&quot;&lt;/script&gt;');
  });

  it('reflects completion state', () => {
    expect(renderTodoItem(todo({ completed: true }))).toContain(
      'todo-item__title--done',
    );
    expect(renderTodoItem(todo())).not.toContain('todo-item__title--done');
  });

  it('targets the list swap on every mutation control', () => {
    const html = renderTodoItem(todo());
    expect(html).toContain(`hx-post="/partials/todos/${todo().id}/toggle"`);
    expect(html).toContain(`hx-delete="/partials/todos/${todo().id}"`);
    expect(html.match(/hx-target="#todo-list"/g)).toHaveLength(2);
  });
});

describe('renderTodoList', () => {
  it('renders the swap anchor with all items', () => {
    const html = renderTodoList([todo(), todo({ id: 'b', title: 'other' })]);
    expect(html).toContain('id="todo-list"');
    expect(html.match(/<li class="todo-item">/g)).toHaveLength(2);
  });
});
