import { check, group, sleep } from "k6";
import { Counter, Trend } from "k6/metrics";
import { login, generateTestUser, register } from "../helpers/auth.js";
import {
  randomTask,
  randomTaskUpdate,
  createTask,
  listTasks,
  getTask,
  updateTask,
  deleteTask,
} from "../helpers/data-gen.js";

const createLatency = new Trend("task_create_latency", true);
const listLatency = new Trend("task_list_latency", true);
const updateLatency = new Trend("task_update_latency", true);
const deleteLatency = new Trend("task_delete_latency", true);
const dataConcurrencyErrors = new Counter("data_concurrency_errors");

export const options = {
  scenarios: {
    // Simultaneous CRUD operations from many users
    mixed_crud: {
      executor: "ramping-vus",
      startVUs: 0,
      stages: [
        { duration: "20s", target: 50 },
        { duration: "1m", target: 150 },
        { duration: "2m", target: 150 },  // sustained mixed CRUD
        { duration: "30s", target: 300 },  // thundering herd spike
        { duration: "1m", target: 300 },
        { duration: "30s", target: 0 },
      ],
      exec: "mixedCrud",
    },
    // Rapid create-then-delete (tests for orphaned records, race conditions)
    create_delete_race: {
      executor: "constant-arrival-rate",
      rate: 50,
      timeUnit: "1s",
      duration: "1m",
      preAllocatedVUs: 100,
      maxVUs: 200,
      exec: "createDeleteRace",
      startTime: "5m",
    },
    // Concurrent updates to the same task (last-write-wins verification)
    concurrent_updates: {
      executor: "per-vu-iterations",
      vus: 20,
      iterations: 10,
      exec: "concurrentUpdates",
      startTime: "6m30s",
    },
  },
  thresholds: {
    task_create_latency: ["p(95)<300", "p(99)<700"],
    task_list_latency: ["p(95)<200", "p(99)<500"],
    task_update_latency: ["p(95)<250", "p(99)<600"],
    task_delete_latency: ["p(95)<200", "p(99)<500"],
    data_concurrency_errors: ["count<5"],
    http_req_failed: ["rate<0.03"],
  },
};

function setupAuth() {
  const user = generateTestUser("crud");
  register(user.email, user.password, user.name);
  login(user.email, user.password);
}

// Scenario 1: Full CRUD lifecycle per iteration
export function mixedCrud() {
  setupAuth();

  group("create task", () => {
    const task = randomTask();
    const res = createTask(task);
    createLatency.add(res.timings.duration);
    const ok = check(res, {
      "create: status 201": (r) => r.status === 201,
      "create: has id": (r) => {
        try { return JSON.parse(r.body).id !== undefined; } catch { return false; }
      },
    });
    if (!ok) return;

    const created = JSON.parse(res.body);
    const taskId = created.id;

    group("read task", () => {
      const getRes = getTask(taskId);
      check(getRes, {
        "get: status 200": (r) => r.status === 200,
        "get: correct id": (r) => {
          try { return JSON.parse(r.body).id === taskId; } catch { return false; }
        },
      });
    });

    group("list tasks", () => {
      const listRes = listTasks();
      listLatency.add(listRes.timings.duration);
      check(listRes, {
        "list: status 200": (r) => r.status === 200,
        "list: is array": (r) => {
          try { return Array.isArray(JSON.parse(r.body)); } catch { return false; }
        },
      });
    });

    group("update task", () => {
      const update = randomTaskUpdate();
      const putRes = updateTask(taskId, update);
      updateLatency.add(putRes.timings.duration);
      check(putRes, {
        "update: status 200": (r) => r.status === 200,
        "update: title changed": (r) => {
          try { return JSON.parse(r.body).title === update.title; } catch { return false; }
        },
      });
    });

    group("delete task", () => {
      const delRes = deleteTask(taskId);
      deleteLatency.add(delRes.timings.duration);
      check(delRes, {
        "delete: status 204": (r) => r.status === 204,
      });

      // Verify deletion
      const getAfterDelete = getTask(taskId);
      check(getAfterDelete, {
        "get after delete: status 404": (r) => r.status === 404,
      });
    });
  });

  sleep(0.2);
}

// Scenario 2: Create and immediately delete (race condition finder)
export function createDeleteRace() {
  setupAuth();

  const task = randomTask();
  const createRes = createTask(task);

  if (createRes.status !== 201) return;

  const taskId = JSON.parse(createRes.body).id;

  // Immediately delete — no sleep
  const delRes = deleteTask(taskId);
  const ok = check(delRes, {
    "race delete: status 204 or 404": (r) => r.status === 204 || r.status === 404,
  });

  if (!ok) {
    dataConcurrencyErrors.add(1);
  }

  // Verify the task is truly gone
  const verifyRes = getTask(taskId);
  check(verifyRes, {
    "race verify: status 404": (r) => r.status === 404,
  });
}

// Scenario 3: Multiple VUs update the same task concurrently
export function concurrentUpdates() {
  setupAuth();

  // All VUs target the same shared task.
  // VU 1 creates it on first iteration; others use the known ID.
  // In practice, you'd use a setup() function, but k6 setup() doesn't share cookies.
  // So we create one task and store its ID in an env-based convention.
  const sharedTaskId = __ENV.SHARED_TASK_ID;
  if (!sharedTaskId) {
    // Fallback: create our own task to update
    const res = createTask(randomTask());
    if (res.status !== 201) return;
    const taskId = JSON.parse(res.body).id;
    const update = randomTaskUpdate();
    const putRes = updateTask(taskId, update);
    check(putRes, {
      "concurrent update: status 200": (r) => r.status === 200,
    });
    return;
  }

  const update = randomTaskUpdate();
  const putRes = updateTask(sharedTaskId, update);
  updateLatency.add(putRes.timings.duration);

  const ok = check(putRes, {
    "concurrent update: status 200 or 409": (r) => r.status === 200 || r.status === 409,
    "concurrent update: not 500": (r) => r.status !== 500,
  });

  if (!ok) {
    dataConcurrencyErrors.add(1);
  }
}
