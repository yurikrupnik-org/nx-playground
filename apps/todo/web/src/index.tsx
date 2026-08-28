import './index.css';
import { render } from '@solidjs/web';
import { QueryClient, QueryClientProvider } from '@tanstack/solid-query';

import { TodoApp } from './todo-app';

const queryClient = new QueryClient();

const root = document.getElementById('root');

if (import.meta.env.DEV && !(root instanceof HTMLElement)) {
  throw new Error(
    'Root element not found. Did you forget to add it to your index.html? Or maybe the id attribute got misspelled?',
  );
}

render(
  () => (
    <QueryClientProvider client={queryClient}>
      <TodoApp />
    </QueryClientProvider>
  ),
  root as HTMLElement,
);
