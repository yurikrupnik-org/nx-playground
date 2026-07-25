import { createMutation, createQuery } from '@tanstack/solid-query';
import { createContext, type ParentComponent, useContext } from 'solid-js';
import type { UserResponse } from './auth-api';
import * as authApi from './auth-api';

interface AuthContextValue {
  user: () => UserResponse | null | undefined;
  isLoading: () => boolean;
  isAuthenticated: () => boolean;
  login: (email: string, password: string) => Promise<void>;
  logout: () => Promise<void>;
  checkAuth: () => void;
}

const AuthContext = createContext<AuthContextValue>();

export const AuthProvider: ParentComponent = (props) => {
  // Query for current user
  const userQuery = createQuery(() => ({
    queryKey: ['currentUser'],
    queryFn: authApi.getCurrentUser,
    retry: false,
    staleTime: 5 * 60 * 1000, // 5 minutes
    // Don't throw errors, just return undefined
    throwOnError: false,
  }));

  // Auth mutations. On success the login page and the logout handler below perform
  // a hard redirect; the destination page refetches /me. We deliberately do NOT
  // call queryClient.setQueryData(['currentUser'], …) here — mutating the auth
  // query from a mutation callback triggers a synchronous reactive storm in this
  // Solid app and freezes the tab. A full navigation yields clean state instead.
  const loginMutation = createMutation(() => ({
    mutationFn: (creds: { email: string; password: string }) =>
      authApi.passwordLogin(creds.email, creds.password),
  }));

  const logoutMutation = createMutation(() => ({
    mutationFn: authApi.logout,
    onSuccess: (logoutUrl) => {
      // Full-page navigation for RP-initiated logout (API → WorkOS end-session →
      // return_to → /login). The reload discards all client state.
      window.location.href = logoutUrl;
    },
  }));

  const login = async (email: string, password: string) => {
    await loginMutation.mutateAsync({ email, password });
  };

  const logout = async () => {
    await logoutMutation.mutateAsync();
  };

  const checkAuth = () => {
    userQuery.refetch();
  };

  const isAuthenticated = () => {
    return !!userQuery.data && !userQuery.isError;
  };

  const isLoading = () => {
    return userQuery.isLoading || userQuery.isFetching;
  };

  const value: AuthContextValue = {
    user: () => userQuery.data,
    isLoading,
    isAuthenticated,
    login,
    logout,
    checkAuth,
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
