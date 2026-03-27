export const TEST_USER = {
  email: "e2e-test@zerg.local",
  password: "Test1234!@#$",
  name: "E2E Test User",
};

export function uniqueUser(prefix = "e2e") {
  const id = `${prefix}-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
  return {
    email: `${id}@test.local`,
    password: "Test1234!@#$",
    name: `Test User ${id}`,
  };
}
