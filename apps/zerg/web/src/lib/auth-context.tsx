import { createMutation, createQuery } from '@tanstack/solid-query';
import { createContext, type ParentComponent, useContext } from 'solid-js';
import type { LoginRequest, RegisterRequest, UserResponse } from './auth-api';
import * as authApi from './auth-api';

interface AuthContextValue {
  user: () => UserResponse | null | undefined;
  isLoading: () => boolean;
  isAuthenticated: () => boolean;
  login: (data: LoginRequest) => Promise<void>;
  register: (data: RegisterRequest) => Promise<void>;
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

  // Auth mutations. On success the login/register pages and the logout handler below
  // perform a hard redirect; the destination page refetches /me. We deliberately do
  // NOT call queryClient.setQueryData(['currentUser'], …) here — mutating the auth
  // query from a mutation callback triggers a synchronous reactive storm in this Solid
  // app and freezes the tab. A full reload yields clean app/query/router state instead.
  const loginMutation = createMutation(() => ({ mutationFn: authApi.login }));

  const registerMutation = createMutation(() => ({
    mutationFn: authApi.register,
  }));

  const logoutMutation = createMutation(() => ({
    mutationFn: authApi.logout,
    onSuccess: () => {
      window.location.href = '/login';
    },
  }));

  const login = async (data: LoginRequest) => {
    await loginMutation.mutateAsync(data);
  };

  const register = async (data: RegisterRequest) => {
    await registerMutation.mutateAsync(data);
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
    register,
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
