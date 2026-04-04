import { check, group, sleep } from "k6";
import { Counter } from "k6/metrics";
import { register, login, logout, generateTestUser, me } from "../helpers/auth.js";
import {
  randomTask,
  createTask,
  listTasks,
  getTask,
  deleteTask,
} from "../helpers/data-gen.js";

const isolationViolations = new Counter("data_isolation_violations");
const crossTenantAccess = new Counter("cross_tenant_access_attempts");

export const options = {
  scenarios: {
    // Each VU is a distinct tenant performing isolated operations
    tenant_isolation: {
      executor: "per-vu-iterations",
      vus: 30,
      iterations: 5,
      exec: "tenantIsolation",
    },
    // Attempt cross-tenant access (should always fail)
    cross_tenant: {
      executor: "per-vu-iterations",
      vus: 10,
      iterations: 10,
      exec: "crossTenantAccess",
      startTime: "2m",
    },
    // Expired/invalid token probing
    token_abuse: {
      executor: "per-vu-iterations",
      vus: 20,
      iterations: 5,
      exec: "tokenAbuse",
      startTime: "3m",
    },
  },
  thresholds: {
    data_isolation_violations: ["count==0"],  // ZERO tolerance
    cross_tenant_access_attempts: ["count>0"], // We should be attempting them
    http_req_failed: ["rate<0.3"], // Higher tolerance since we expect 401s/403s
  },
};

// Scenario 1: Each VU = one tenant. Create tasks, verify only own data visible.
export function tenantIsolation() {
  const user = generateTestUser(`tenant-${__VU}`);
  register(user.email, user.password, user.name);
  login(user.email, user.password);

  // Verify identity
  const meRes = me();
  check(meRes, {
    "tenant: authenticated": (r) => r.status === 200,
  });
  if (meRes.status !== 200) return;

  const myEmail = JSON.parse(meRes.body).user.email;

  group("create tenant tasks", () => {
    const taskIds = [];
    // Create 3 tasks
    for (let i = 0; i < 3; i++) {
      const task = randomTask();
      task.title = `TENANT-${__VU}-task-${i}`;
      const res = createTask(task);
      check(res, { "tenant create: 201": (r) => r.status === 201 });
      if (res.status === 201) {
        taskIds.push(JSON.parse(res.body).id);
      }
    }

    // List tasks and verify we can see our own
    const listRes = listTasks();
    if (listRes.status === 200) {
      const tasks = JSON.parse(listRes.body);
      const myTasks = tasks.filter((t) =>
        t.title.startsWith(`TENANT-${__VU}-`)
      );
      check(null, {
        "tenant: can see own tasks": () => myTasks.length === taskIds.length,
      });
    }

    // Clean up
    for (const id of taskIds) {
      deleteTask(id);
    }
  });

  sleep(0.5);
}

// Scenario 2: User A creates tasks, User B tries to access them
export function crossTenantAccess() {
  crossTenantAccess.add(1);

  // User A: create a task
  const userA = generateTestUser("cross-a");
  register(userA.email, userA.password, userA.name);
  login(userA.email, userA.password);

  const task = randomTask();
  task.title = `PRIVATE-${__VU}-${__ITER}`;
  const createRes = createTask(task);
  if (createRes.status !== 201) return;
  const taskId = JSON.parse(createRes.body).id;

  // User B: try to access User A's task
  const userB = generateTestUser("cross-b");
  register(userB.email, userB.password, userB.name);
  login(userB.email, userB.password);

  const getRes = getTask(taskId);

  // If the API has tenant isolation, User B should get 403 or 404
  // If we get 200, that's a data isolation violation
  if (getRes.status === 200) {
    const body = JSON.parse(getRes.body);
    if (body.title && body.title.startsWith(`PRIVATE-${__VU}-`)) {
      isolationViolations.add(1);
      console.error(
        `ISOLATION VIOLATION: User B accessed User A's task ${taskId}`
      );
    }
  }

  check(getRes, {
    "cross-tenant: not 500": (r) => r.status !== 500,
  });

  // Clean up as User A
  login(userA.email, userA.password);
  deleteTask(taskId);
}

// Scenario 3: Attempt API access with invalid/expired/tampered tokens
export function tokenAbuse() {
  const user = generateTestUser("abuse");
  register(user.email, user.password, user.name);
  login(user.email, user.password);

  // Now logout (invalidates token in Redis whitelist)
  logout();

  // k6 cookie jar still has the old cookie — test that server rejects it
  const afterLogout = listTasks();
  check(afterLogout, {
    "expired token: rejected (401)": (r) => r.status === 401,
    "expired token: not 200": (r) => r.status !== 200,
  });

  if (afterLogout.status === 200) {
    isolationViolations.add(1);
    console.error("SECURITY: Invalidated token still accepted!");
  }
}
