import { createQuery } from '@tanstack/solid-query';
import { createContext, type ParentComponent, useContext } from 'solid-js';
import type { AuthApi } from './auth-api';

export interface AuthContextValue<TUser, TIdp extends string = string> {
  user: () => TUser | undefined;
  isLoading: () => boolean;
  isAuthenticated: () => boolean;
  /** Redirect-based login; `idp` deep-links a brokered social provider. */
  login: (idp?: TIdp) => void;
  /** Native email+password login, followed by a `/me` refetch. */
  passwordLogin: (email: string, password: string) => Promise<void>;
  logout: () => Promise<void>;
  refetch: () => void;
}

/**
 * Builds an auth provider + hook pair over one app's [`AuthApi`].
 *
 * Returned as a factory rather than a component so each app keeps its own
 * context instance (and its own `TUser`), while the query configuration and the
 * logout navigation semantics stay identical across apps.
 */
export function createAuthProvider<TUser, TIdp extends string = string>(
  api: AuthApi<TUser, TIdp>,
): {
  AuthProvider: ParentComponent;
  useAuth: () => AuthContextValue<TUser, TIdp>;
} {
  const AuthContext = createContext<AuthContextValue<TUser, TIdp>>();

  const AuthProvider: ParentComponent = (props) => {
    const userQuery = createQuery(() => ({
      queryKey: ['currentUser'],
      queryFn: () => api.getCurrentUser(),
      retry: false,
      staleTime: 5 * 60 * 1000,
      // A 401 is "not logged in", not an exceptional error.
      throwOnError: false,
    }));

    const logout = async () => {
      const logoutUrl = await api.logout();
      // Full-page navigation for RP-initiated logout (API → IdP end-session →
      // post-logout redirect → SPA → /login). We deliberately do NOT mutate the
      // query cache first: the reload discards all state anyway, and flipping auth
      // to unauthenticated mid-page makes ProtectedRoute redirect underneath us.
      window.location.href = logoutUrl;
    };

    const value: AuthContextValue<TUser, TIdp> = {
      user: () => userQuery.data ?? undefined,
      // Deliberately not `|| isFetching`: a background revalidation must not put
      // ProtectedRoute back into its loading state and unmount the page.
      isLoading: () => userQuery.isLoading,
      isAuthenticated: () => !!userQuery.data && !userQuery.isError,
      login: (idp) => api.login(idp),
      passwordLogin: async (email, password) => {
        await api.passwordLogin(email, password);
        await userQuery.refetch();
      },
      logout,
      refetch: () => {
        void userQuery.refetch();
      },
    };

    return (
      <AuthContext.Provider value={value}>
        {props.children}
      </AuthContext.Provider>
    );
  };

  function useAuth(): AuthContextValue<TUser, TIdp> {
    const context = useContext(AuthContext);
    if (!context) {
      throw new Error('useAuth must be used within an AuthProvider');
    }
    return context;
  }

  return { AuthProvider, useAuth };
}
