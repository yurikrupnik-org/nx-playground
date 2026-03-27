import http from "k6/http";

const BASE_URL = __ENV.BASE_URL || "http://localhost:3000";

/**
 * Register a new user and return the response.
 */
export function register(email, password, name) {
  const payload = JSON.stringify({ email, password, name });
  return http.post(`${BASE_URL}/api/auth/register`, payload, {
    headers: { "Content-Type": "application/json" },
  });
}

/**
 * Login and return the response (cookies are set automatically by k6 jar).
 */
export function login(email, password) {
  const payload = JSON.stringify({ email, password });
  return http.post(`${BASE_URL}/api/auth/login`, payload, {
    headers: { "Content-Type": "application/json" },
  });
}

/**
 * Logout the current session.
 */
export function logout() {
  return http.post(`${BASE_URL}/api/auth/logout`, null);
}

/**
 * Get the current authenticated user.
 */
export function me() {
  return http.get(`${BASE_URL}/api/auth/me`);
}

/**
 * Generate a unique test user for the current VU and iteration.
 */
export function generateTestUser(prefix) {
  const id = `${prefix || "k6"}-${__VU}-${__ITER}-${Date.now()}`;
  return {
    email: `${id}@test.local`,
    password: "Test1234!@#$",
    name: `Test User ${id}`,
  };
}
