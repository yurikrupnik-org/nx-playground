import type { CreateTodo, Todo } from '@domain/todo';
import { fireEvent, render, waitFor } from '@solidjs/testing-library';
import { QueryClient, QueryClientProvider } from '@tanstack/solid-query';
import type { ParentComponent } from 'solid-js';
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';

import { TodoApp } from './todo-app';

const sampleTodo: Todo = {
  id: 'todo-1',
  title: 'Buy milk',
  description: '',
  completed: false,
  priority: 'medium',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
};

function jsonResponse(data: unknown, status = 200): Response {
  return {
    ok: status >= 200 && status < 300,
    status,
    json: async () => data,
  } as unknown as Response;
}

const fetchMock = vi.fn(
  async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const url = typeof input === 'string' ? input : input.toString();
    const method = init?.method ?? 'GET';

    if (url.split('?')[0] === '/api/todos' && method === 'GET') {
      return jsonResponse([sampleTodo]);
    }
    if (url === '/api/todos' && method === 'POST') {
      const body = JSON.parse(String(init?.body)) as CreateTodo;
      return jsonResponse(
        {
          ...sampleTodo,
          id: 'todo-2',
          title: body.title,
          priority: body.priority,
        },
        201,
      );
    }
    return jsonResponse(null);
  },
);

beforeEach(() => {
  vi.stubGlobal('fetch', fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
  vi.clearAllMocks();
});

function renderApp() {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
      mutations: { retry: false },
    },
  });
  const Wrapper: ParentComponent = (props) => (
    <QueryClientProvider client={queryClient}>
      {props.children}
    </QueryClientProvider>
  );
  return render(() => <TodoApp />, { wrapper: Wrapper });
}

describe('<TodoApp />', () => {
  test('renders the add input and button', () => {
    const { getByPlaceholderText, getByRole } = renderApp();
    expect(getByPlaceholderText('new todo title')).toBeInTheDocument();
    expect(getByRole('button', { name: /add/i })).toBeInTheDocument();
  });

  test('renders a todo returned by the list query', async () => {
    const { findByText } = renderApp();
    expect(await findByText('Buy milk')).toBeInTheDocument();
  });

  test('clicking add posts a typed CreateTodo to /api/todos', async () => {
    const { getByPlaceholderText, getByRole } = renderApp();
    const input = getByPlaceholderText('new todo title') as HTMLInputElement;

    fireEvent.input(input, { target: { value: 'write tests' } });
    fireEvent.click(getByRole('button', { name: /add/i }));

    await waitFor(() => {
      const posted = fetchMock.mock.calls.find(
        ([, init]) => (init as RequestInit | undefined)?.method === 'POST',
      );
      expect(posted).toBeTruthy();
    });

    const posted = fetchMock.mock.calls.find(
      ([, init]) => (init as RequestInit | undefined)?.method === 'POST',
    );
    if (!posted) throw new Error('expected a POST call to /api/todos');

    expect(posted[0]).toBe('/api/todos');
    const body = JSON.parse(
      String((posted[1] as RequestInit).body),
    ) as CreateTodo;
    expect(body).toEqual({
      title: 'write tests',
      description: '',
      priority: 'medium',
    });
  });
});
