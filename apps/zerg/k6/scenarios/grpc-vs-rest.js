import { check, group, sleep } from "k6";
import { Trend } from "k6/metrics";
import { login, generateTestUser, register } from "../helpers/auth.js";
import {
  randomTask,
  createTask,
  listTasks,
  getTask,
  createTaskDirect,
  listTasksDirect,
} from "../helpers/data-gen.js";

// Separate latency tracking per path
const grpcCreateLatency = new Trend("grpc_proxy_create_latency", true);
const directCreateLatency = new Trend("direct_db_create_latency", true);
const grpcListLatency = new Trend("grpc_proxy_list_latency", true);
const directListLatency = new Trend("direct_db_list_latency", true);

export const options = {
  scenarios: {
    // Side-by-side comparison: gRPC-proxied vs direct DB
    grpc_path: {
      executor: "ramping-vus",
      startVUs: 0,
      stages: [
        { duration: "15s", target: 30 },
        { duration: "1m", target: 100 },
        { duration: "2m", target: 100 },
        { duration: "30s", target: 0 },
      ],
      exec: "grpcPath",
    },
    direct_path: {
      executor: "ramping-vus",
      startVUs: 0,
      stages: [
        { duration: "15s", target: 30 },
        { duration: "1m", target: 100 },
        { duration: "2m", target: 100 },
        { duration: "30s", target: 0 },
      ],
      exec: "directPath",
    },
  },
  thresholds: {
    grpc_proxy_create_latency: ["p(95)<500"],
    direct_db_create_latency: ["p(95)<300"],
    grpc_proxy_list_latency: ["p(95)<400"],
    direct_db_list_latency: ["p(95)<200"],
    http_req_failed: ["rate<0.05"],
  },
};

function setupAuth() {
  const user = generateTestUser("grpc-rest");
  register(user.email, user.password, user.name);
  login(user.email, user.password);
}

// gRPC-proxied path: API → gRPC → tasks service → DB
export function grpcPath() {
  setupAuth();

  group("grpc: create", () => {
    const task = randomTask();
    const res = createTask(task);
    grpcCreateLatency.add(res.timings.duration);
    check(res, {
      "grpc create: status 201": (r) => r.status === 201,
    });

    if (res.status === 201) {
      const taskId = JSON.parse(res.body).id;
      const getRes = getTask(taskId);
      check(getRes, {
        "grpc get: status 200": (r) => r.status === 200,
      });
    }
  });

  group("grpc: list", () => {
    const res = listTasks();
    grpcListLatency.add(res.timings.duration);
    check(res, {
      "grpc list: status 200": (r) => r.status === 200,
    });
  });

  sleep(0.3);
}

// Direct DB path: API → PostgreSQL (no gRPC hop)
export function directPath() {
  setupAuth();

  group("direct: create", () => {
    const task = randomTask();
    const res = createTaskDirect(task);
    directCreateLatency.add(res.timings.duration);
    check(res, {
      "direct create: status 201": (r) => r.status === 201,
    });
  });

  group("direct: list", () => {
    const res = listTasksDirect();
    directListLatency.add(res.timings.duration);
    check(res, {
      "direct list: status 200": (r) => r.status === 200,
    });
  });

  sleep(0.3);
}
