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
import 'solid-devtools';

import { ProtectedRoute } from './components/protected-route';
import { UserMenu } from './components/user-menu';
import { AuthProvider } from './lib/auth-context';
import { LoginPage } from './pages/login';
import { RegisterPage } from './pages/register';
import { RegistriesPage } from './pages/registries';
import { SettingsPage } from './pages/settings';
import { TaskDetailPage } from './pages/task-detail';
import { TasksListPage } from './pages/tasks-list';

const queryClient = new QueryClient();

// Layout component with navigation
function Layout() {
  return (
    <div>
      <nav class="border-b bg-white shadow-sm">
        <div class="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
          <div class="flex justify-between h-16 items-center">
            <div class="flex items-center gap-6">
              <h1 class="text-xl font-bold">Zerg Tasks</h1>
              <Link
                to="/tasks"
                class="text-sm text-gray-600 hover:text-gray-900"
              >
                Tasks
              </Link>
              <Link
                to="/settings"
                class="text-sm text-gray-600 hover:text-gray-900"
              >
                Settings
              </Link>
              <Link
                to="/registries"
                class="text-sm text-gray-600 hover:text-gray-900"
              >
                Registries
              </Link>
            </div>
            <div class="flex items-center">
              <UserMenu />
            </div>
          </div>
        </div>
      </nav>
      <main>
        <Outlet />
      </main>
    </div>
  );
}

// Create a root route with layout
const rootRoute = createRootRoute({
  component: Layout,
});

// Index route - redirect to tasks
const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/',
  component: () => <Navigate to="/tasks" />,
});

// Public routes
const loginRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/login',
  component: LoginPage,
});

const registerRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/register',
  component: RegisterPage,
});

// Reference screen — static catalog, no auth required
const registriesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/registries',
  component: () => <RegistriesPage />,
});

// Protected routes
const tasksRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/tasks',
  component: () => (
    <ProtectedRoute>
      <TasksListPage />
    </ProtectedRoute>
  ),
});

const taskDetailRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/tasks/$id',
  component: () => (
    <ProtectedRoute>
      <TaskDetailPage />
    </ProtectedRoute>
  ),
});

const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: '/settings',
  component: () => (
    <ProtectedRoute>
      <SettingsPage />
    </ProtectedRoute>
  ),
});

// Build route tree
const routeTree = rootRoute.addChildren([
  indexRoute,
  loginRoute,
  registerRoute,
  registriesRoute,
  tasksRoute,
  taskDetailRoute,
  settingsRoute,
]);

// Create router
const router = createRouter({ routeTree });

const root = document.getElementById('root');

if (import.meta.env.DEV && !(root instanceof HTMLElement)) {
  throw new Error(
    'Root!! element not found. Did you forget to add it to your index.html? Or maybe the id attribute got misspelled?',
  );
}

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
