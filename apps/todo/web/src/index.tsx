import './index.css';
import { createRouter, defineRoutes } from '@solidjs/router';
import { render } from '@solidjs/web';
import { QueryClient, QueryClientProvider } from '@tanstack/solid-query';
import { lazy } from 'solid-js';

import { AppNav } from './app-nav';
import { TodoApp } from './todo-app';

const queryClient = new QueryClient();

/**
 * Three routes, three ways to hold the same state.
 *
 * `/` is the production app (TanStack Query + signals, plus feature flags and
 * identity switching). `/xstate` and `/effect` are minimal re-implementations of
 * the same core loop — load, add, toggle, delete, patch from database events —
 * so the only variable between them is the state-management library. See
 * `docs/todo-state-management.md`.
 *
 * Both alternatives are `lazy()` so their runtimes are separate chunks: visiting
 * `/` must not download xstate or effect. That is load-bearing for this vertical,
 * which publishes shipped-JS numbers (`stack_profiles`).
 */
const routes = defineRoutes([
  { path: '/', component: TodoApp },
  {
    path: '/xstate',
    component: lazy(() => import('./routes/xstate-route')),
  },
  {
    path: '/effect',
    component: lazy(() => import('./routes/effect-route')),
  },
]);

const Router = createRouter({ routes });

const root = document.getElementById('root');

if (import.meta.env.DEV && !(root instanceof HTMLElement)) {
  throw new Error(
    'Root element not found. Did you forget to add it to your index.html? Or maybe the id attribute got misspelled?',
  );
}

render(
  () => (
    <QueryClientProvider client={queryClient}>
      <Router>
        {(props) => (
          <>
            <AppNav />
            {props.children}
          </>
        )}
      </Router>
    </QueryClientProvider>
  ),
  root as HTMLElement,
);
