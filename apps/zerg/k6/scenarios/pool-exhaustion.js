import { check, sleep } from "k6";
import { Counter, Trend } from "k6/metrics";
import { login, generateTestUser, register } from "../helpers/auth.js";
import { listTasks, createTask, randomTask } from "../helpers/data-gen.js";
import http from "k6/http";

const BASE_URL = __ENV.BASE_URL || "http://localhost:3000";

const exhaustionErrors = new Counter("pool_exhaustion_errors");
const healthLatency = new Trend("health_check_latency", true);
const listLatencyUnderLoad = new Trend("list_latency_under_load", true);

export const options = {
  scenarios: {
    // Slowly ramp connections until pool saturates
    slow_ramp: {
      executor: "ramping-vus",
      startVUs: 0,
      stages: [
        { duration: "30s", target: 50 },   // well within pool limits
        { duration: "30s", target: 100 },
        { duration: "30s", target: 200 },
        { duration: "30s", target: 400 },   // likely exceeding pool
        { duration: "1m", target: 400 },    // sustain at over-capacity
        { duration: "30s", target: 600 },   // push harder
        { duration: "30s", target: 600 },
        { duration: "30s", target: 0 },     // cool-down / recovery test
      ],
      exec: "slowRamp",
    },
    // Health check monitor (runs alongside to detect degradation)
    health_monitor: {
      executor: "constant-arrival-rate",
      rate: 2,
      timeUnit: "1s",
      duration: "5m",
      preAllocatedVUs: 5,
      maxVUs: 10,
      exec: "healthMonitor",
    },
  },
  thresholds: {
    // Under normal load, list should be fast
    "list_latency_under_load": ["p(50)<500"],
    // Health checks should always respond
    "health_check_latency": ["p(99)<2000"],
    // Some 503s are expected, but not too many
    "pool_exhaustion_errors": ["count<100"],
    // Overall error rate should stay reasonable even under overload
    "http_req_failed": ["rate<0.2"],
  },
};

// Scenario 1: Ramp up DB-hitting requests until pool exhausts
export function slowRamp() {
  const user = generateTestUser("pool");
  register(user.email, user.password, user.name);
  login(user.email, user.password);

  // Mix of reads and writes to stress the pool
  const readRes = listTasks();
  listLatencyUnderLoad.add(readRes.timings.duration);

  if (readRes.status === 503 || readRes.status === 504) {
    exhaustionErrors.add(1);
  }

  check(readRes, {
    "pool: responds (any)": (r) => r.status > 0,
    "pool: not panic (500)": (r) => r.status !== 500,
  });

  // Also do a write to increase pool pressure
  if (__ITER % 3 === 0) {
    const writeRes = createTask(randomTask());
    if (writeRes.status === 503 || writeRes.status === 504) {
      exhaustionErrors.add(1);
    }
    check(writeRes, {
      "pool write: responds": (r) => r.status > 0,
    });
  }

  sleep(0.1);
}

// Scenario 2: Lightweight health checks to measure degradation
export function healthMonitor() {
  const healthRes = http.get(`${BASE_URL}/health`);
  healthLatency.add(healthRes.timings.duration);

  check(healthRes, {
    "health: status 200": (r) => r.status === 200,
    "health: under 1s": (r) => r.timings.duration < 1000,
  });

  const readyRes = http.get(`${BASE_URL}/ready`);
  check(readyRes, {
    "ready: responds": (r) => r.status > 0,
  });

  // Log timing for analysis
  if (readyRes.timings.duration > 1000) {
    console.warn(
      `Readiness check degraded: ${readyRes.timings.duration}ms (status: ${readyRes.status})`
    );
  }
}
