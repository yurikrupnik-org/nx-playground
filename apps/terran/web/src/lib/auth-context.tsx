import { createQuery } from '@tanstack/solid-query';
import { createContext, type ParentComponent, useContext } from 'solid-js';
import type { IdpHint, Me } from './auth-api';
import * as authApi from './auth-api';

interface AuthContextValue {
  user: () => Me | undefined;
  isLoading: () => boolean;
  isAuthenticated: () => boolean;
  login: (idp?: IdpHint) => void;
  passwordLogin: (email: string, password: string) => Promise<void>;
  logout: () => Promise<void>;
  refetch: () => void;
}

const AuthContext = createContext<AuthContextValue>();

export const AuthProvider: ParentComponent = (props) => {
  const userQuery = createQuery(() => ({
    queryKey: ['currentUser'],
    queryFn: authApi.getCurrentUser,
    retry: false,
    staleTime: 5 * 60 * 1000,
    // A 401 is "not logged in", not an exceptional error.
    throwOnError: false,
  }));

  const logout = async () => {
    const logoutUrl = await authApi.logout();
    // Full-page navigation for RP-initiated logout (API → Keycloak end-session →
    // post_logout_redirect_uri → SPA → /login). We do NOT mutate the query cache
    // first: the reload discards all state anyway, and flipping auth to
    // unauthenticated mid-page makes ProtectedRoute redirect underneath us.
    window.location.href = logoutUrl;
  };

  const value: AuthContextValue = {
    user: () => userQuery.data ?? undefined,
    isLoading: () => userQuery.isLoading,
    isAuthenticated: () => !!userQuery.data && !userQuery.isError,
    login: (idp) => authApi.login(idp),
    passwordLogin: async (email, password) => {
      await authApi.passwordLogin(email, password);
      await userQuery.refetch();
    },
    logout,
    refetch: () => {
      void userQuery.refetch();
    },
  };

  return (
    <AuthContext.Provider value={value}>{props.children}</AuthContext.Provider>
  );
};

export function useAuth() {
  const context = useContext(AuthContext);
  if (!context) {
    throw new Error('useAuth must be used within an AuthProvider');
  }
  return context;
}
