import './index.css';
import { QueryClient, QueryClientProvider } from '@tanstack/solid-query';
import {
  createRootRoute,
  createRoute,
  createRouter,
  Navigate,
  Outlet,
  RouterProvider,
} from '@tanstack/solid-router';
import { render } from 'solid-js/web';

import { ProtectedRoute } from './components/protected-route';
import { UserMenu } from './components/user-menu';
import { AuthProvider } from './lib/auth-context';
import { AssetsPage } from './pages/assets';
import { LoginPage } from './pages/login';

const queryClient = new QueryClient();

function Layout() {
  return (
    <div>
      <nav class="border-b bg-white shadow-sm">
        <div class="mx-auto flex h-16 max-w-5xl items-center justify-between px-4">
          <h1 class="text-xl font-bold">terran</h1>
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

const routeTree = rootRoute.addChildren([indexRoute, loginRoute, assetsRoute]);
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
