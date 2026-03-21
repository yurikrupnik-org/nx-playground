import './index.css';
import { QueryClient, QueryClientProvider } from '@tanstack/solid-query';
import {
  createRootRoute,
  createRoute,
  createRouter,
  Navigate,
  RouterProvider,
} from '@tanstack/solid-router';
import { render } from 'solid-js/web';
import 'solid-devtools';

import { AppShell } from './components/layout/app-shell';
import { CatalogPage } from './pages/catalog';
import { DesignsPage } from './pages/designs';
import { EtlPage } from './pages/etl';
import { IntegrationsPage } from './pages/integrations';
import { IssuesPage } from './pages/issues';
import { ObservabilityPage } from './pages/observability';
import { ReverseEtlPage } from './pages/reverse-etl';
import { SettingsPage } from './pages/settings';

const queryClient = new QueryClient();

const rootRoute = createRootRoute({ component: AppShell });

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/',
  component: () => <Navigate to="/catalog" />,
});

const catalogRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/catalog',
  component: CatalogPage,
});

const etlRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/etl',
  component: EtlPage,
});

const reverseEtlRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/reverse-etl',
  component: ReverseEtlPage,
});

const observabilityRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/observability',
  component: ObservabilityPage,
});

const issuesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/issues',
  component: IssuesPage,
});

const integrationsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/integrations',
  component: IntegrationsPage,
});

const designsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/designs',
  component: DesignsPage,
});

const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/settings',
  component: SettingsPage,
});

const routeTree = rootRoute.addChildren([
  indexRoute,
  catalogRoute,
  etlRoute,
  reverseEtlRoute,
  observabilityRoute,
  issuesRoute,
  integrationsRoute,
  designsRoute,
  settingsRoute,
]);

const router = createRouter({ routeTree });

const root = document.getElementById('root');

if (import.meta.env.DEV && !(root instanceof HTMLElement)) {
  throw new Error('Root element not found.');
}

if (root) {
  render(
    () => (
      <QueryClientProvider client={queryClient}>
        <RouterProvider router={router} />
      </QueryClientProvider>
    ),
    root,
  );
}
