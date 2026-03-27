import { check, sleep } from "k6";
import { Counter, Trend } from "k6/metrics";
import { register, login, logout, me, generateTestUser } from "../helpers/auth.js";

// Custom metrics
const loginLatency = new Trend("login_latency", true);
const registerLatency = new Trend("register_latency", true);
const authCheckLatency = new Trend("auth_check_latency", true);
const failedLogins = new Counter("failed_logins");
const failedRegistrations = new Counter("failed_registrations");

export const options = {
  scenarios: {
    // Phase 1: Gradual ramp-up of register + login + use + logout cycles
    full_auth_cycle: {
      executor: "ramping-vus",
      startVUs: 0,
      stages: [
        { duration: "30s", target: 50 },   // warm-up
        { duration: "1m", target: 200 },    // ramp to 200 concurrent users
        { duration: "2m", target: 200 },    // sustained load
        { duration: "1m", target: 500 },    // spike to 500
        { duration: "30s", target: 500 },   // hold spike
        { duration: "30s", target: 0 },     // cool-down
      ],
      exec: "fullAuthCycle",
    },
    // Phase 2: Burst login attempts (simulates credential stuffing)
    login_burst: {
      executor: "constant-arrival-rate",
      rate: 100,            // 100 requests per second
      timeUnit: "1s",
      duration: "1m",
      preAllocatedVUs: 200,
      maxVUs: 500,
      exec: "loginBurst",
      startTime: "5m30s",   // starts after full_auth_cycle winds down
    },
    // Phase 3: Token validation storm (many concurrent /me calls)
    token_validation: {
      executor: "ramping-arrival-rate",
      startRate: 10,
      timeUnit: "1s",
      stages: [
        { duration: "30s", target: 50 },
        { duration: "1m", target: 200 },
        { duration: "30s", target: 200 },
      ],
      preAllocatedVUs: 300,
      maxVUs: 500,
      exec: "tokenValidation",
      startTime: "7m",
    },
  },
  thresholds: {
    "http_req_duration{scenario:full_auth_cycle}": ["p(95)<500", "p(99)<1000"],
    "http_req_duration{scenario:login_burst}": ["p(95)<300"],
    login_latency: ["p(95)<200", "p(99)<500"],
    register_latency: ["p(95)<300", "p(99)<800"],
    auth_check_latency: ["p(95)<100", "p(99)<200"],
    failed_logins: ["count<50"],
    failed_registrations: ["count<20"],
    http_req_failed: ["rate<0.05"], // <5% error rate overall
  },
};

// Scenario 1: Full register → login → /me → logout cycle
export function fullAuthCycle() {
  const user = generateTestUser("auth-storm");

  // Register
  const regRes = register(user.email, user.password, user.name);
  registerLatency.add(regRes.timings.duration);
  const regOk = check(regRes, {
    "register: status 200": (r) => r.status === 200,
    "register: has user id": (r) => {
      try { return JSON.parse(r.body).user.id !== undefined; } catch { return false; }
    },
  });
  if (!regOk) {
    failedRegistrations.add(1);
    return;
  }

  sleep(0.1);

  // Login
  const loginRes = login(user.email, user.password);
  loginLatency.add(loginRes.timings.duration);
  const loginOk = check(loginRes, {
    "login: status 200": (r) => r.status === 200,
    "login: has user": (r) => {
      try { return JSON.parse(r.body).user.email === user.email; } catch { return false; }
    },
  });
  if (!loginOk) {
    failedLogins.add(1);
    return;
  }

  // Authenticated request: GET /me
  const meRes = me();
  authCheckLatency.add(meRes.timings.duration);
  check(meRes, {
    "me: status 200": (r) => r.status === 200,
    "me: correct email": (r) => {
      try { return JSON.parse(r.body).user.email === user.email; } catch { return false; }
    },
  });

  sleep(0.5);

  // Logout
  const logoutRes = logout();
  check(logoutRes, {
    "logout: status 204": (r) => r.status === 204,
  });

  // Verify token is invalidated after logout
  const meAfterLogout = me();
  check(meAfterLogout, {
    "me after logout: status 401": (r) => r.status === 401,
  });
}

// Scenario 2: Rapid-fire login attempts (tests rate limiting + Redis token whitelist)
export function loginBurst() {
  const user = generateTestUser("burst");

  // Most of these will fail (user doesn't exist) — that's intentional.
  // We're testing how the server handles a flood of auth attempts.
  const res = login(user.email, user.password);
  check(res, {
    "burst login: responds (any status)": (r) => r.status > 0,
    "burst login: not 500": (r) => r.status !== 500,
    "burst login: responds under 1s": (r) => r.timings.duration < 1000,
  });
}

// Scenario 3: Hammer /me with valid tokens to stress Redis token whitelist
export function tokenValidation() {
  // Each VU registers + logs in once during setup, then hammers /me
  const user = generateTestUser("token-val");
  register(user.email, user.password, user.name);
  login(user.email, user.password);

  const meRes = me();
  authCheckLatency.add(meRes.timings.duration);
  check(meRes, {
    "token validation: status 200": (r) => r.status === 200,
    "token validation: under 100ms": (r) => r.timings.duration < 100,
  });
}
