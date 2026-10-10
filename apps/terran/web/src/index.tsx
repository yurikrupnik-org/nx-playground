import './index.css';
import { QueryClient, QueryClientProvider } from '@tanstack/solid-query';
import {
  createRootRoute,
  createRoute,
  createRouter,
  Link,
  Navigate,
  Outlet,
  RouterProvider,
} from '@tanstack/solid-router';
import { render } from 'solid-js/web';

import { UserMenu } from './components/user-menu';
import { AuthProvider, ProtectedRoute } from './lib/auth';
import { AssetsPage } from './pages/assets';
import { CloudResourcesPage } from './pages/cloud-resources';
import { LoginPage } from './pages/login';

const queryClient = new QueryClient();

function Layout() {
  return (
    <div>
      <nav class="border-b bg-white shadow-sm">
        <div class="mx-auto flex h-16 max-w-5xl items-center justify-between px-4">
          <div class="flex items-center gap-6">
            <h1 class="text-xl font-bold">terran</h1>
            <Link
              to="/assets"
              class="text-sm text-gray-600 hover:text-gray-900"
              activeProps={{ class: 'text-sm font-medium text-gray-900' }}
            >
              Assets
            </Link>
            <Link
              to="/cloud-resources"
              class="text-sm text-gray-600 hover:text-gray-900"
              activeProps={{ class: 'text-sm font-medium text-gray-900' }}
            >
              Cloud resources
            </Link>
          </div>
          <UserMenu />
        </div>
      </nav>
      <main>
        <Outlet />
      </main>
    </div>
  );
}

const rootRoute = createRootRoute({ component: Layout });

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/',
  component: () => <Navigate to="/assets" />,
});

const loginRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/login',
  component: LoginPage,
});

const assetsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/assets',
  component: () => (
    <ProtectedRoute>
      <AssetsPage />
    </ProtectedRoute>
  ),
});

const cloudResourcesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/cloud-resources',
  component: () => (
    <ProtectedRoute>
      <CloudResourcesPage />
    </ProtectedRoute>
  ),
});

const routeTree = rootRoute.addChildren([
  indexRoute,
  loginRoute,
  assetsRoute,
  cloudResourcesRoute,
]);
const router = createRouter({ routeTree });

const root = document.getElementById('root');
if (root) {
  render(
    () => (
      <QueryClientProvider client={queryClient}>
        <AuthProvider>
          <RouterProvider router={router} />
        </AuthProvider>
      </QueryClientProvider>
    ),
    root,
  );
}
