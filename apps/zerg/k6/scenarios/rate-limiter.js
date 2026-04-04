import { check, sleep } from "k6";
import { Counter, Rate } from "k6/metrics";
import { login, generateTestUser, register } from "../helpers/auth.js";
import { listTasks } from "../helpers/data-gen.js";

const rateLimited = new Counter("rate_limited_responses");
const rateLimitRate = new Rate("rate_limit_hit_rate");

export const options = {
  scenarios: {
    // Single user exceeding rate limit
    single_user_flood: {
      executor: "constant-arrival-rate",
      rate: 200,           // 200 req/s from a single user context
      timeUnit: "1s",
      duration: "30s",
      preAllocatedVUs: 50,
      maxVUs: 100,
      exec: "singleUserFlood",
    },
    // Many users each sending moderate traffic (should NOT trigger rate limits)
    multi_user_normal: {
      executor: "per-vu-iterations",
      vus: 50,
      iterations: 20,
      exec: "multiUserNormal",
      startTime: "45s",
    },
    // Unauthenticated flood (tests IP-based rate limiting)
    unauthenticated_flood: {
      executor: "constant-arrival-rate",
      rate: 300,
      timeUnit: "1s",
      duration: "30s",
      preAllocatedVUs: 100,
      maxVUs: 200,
      exec: "unauthenticatedFlood",
      startTime: "2m",
    },
  },
  thresholds: {
    // Single user flood should trigger rate limiting
    "rate_limit_hit_rate{scenario:single_user_flood}": ["rate>0.3"],
    // Multi user normal should NOT trigger rate limiting
    "rate_limit_hit_rate{scenario:multi_user_normal}": ["rate<0.05"],
    http_req_failed: ["rate<0.1"],
  },
};

// Scenario 1: Flood from a single authenticated user
export function singleUserFlood() {
  // Login once per VU (cookies persist across iterations within a VU)
  if (__ITER === 0) {
    const user = generateTestUser("ratelimit-single");
    register(user.email, user.password, user.name);
    login(user.email, user.password);
  }

  const res = listTasks();
  const is429 = res.status === 429;

  rateLimitRate.add(is429);
  if (is429) {
    rateLimited.add(1);
  }

  check(res, {
    "flood: responds (not 500)": (r) => r.status !== 500,
    "flood: 200 or 429": (r) => r.status === 200 || r.status === 429,
  });

  // If rate limited, check for Retry-After header
  if (is429) {
    check(res, {
      "429: has retry-after or rate limit headers": (r) =>
        r.headers["Retry-After"] !== undefined ||
        r.headers["X-RateLimit-Remaining"] !== undefined ||
        r.headers["X-Ratelimit-Remaining"] !== undefined,
    });
  }
}

// Scenario 2: Normal traffic from many distinct users (should pass through)
export function multiUserNormal() {
  const user = generateTestUser("ratelimit-normal");
  register(user.email, user.password, user.name);
  login(user.email, user.password);

  const res = listTasks();
  const is429 = res.status === 429;

  rateLimitRate.add(is429);
  if (is429) {
    rateLimited.add(1);
  }

  check(res, {
    "normal: status 200": (r) => r.status === 200,
    "normal: not rate limited": (r) => r.status !== 429,
  });

  sleep(0.5); // Normal pacing between requests
}

// Scenario 3: Unauthenticated requests (tests IP-level rate limiting)
export function unauthenticatedFlood() {
  const res = listTasks();
  const is429 = res.status === 429;

  rateLimitRate.add(is429);
  if (is429) {
    rateLimited.add(1);
  }

  check(res, {
    "unauth flood: responds": (r) => r.status > 0,
    "unauth flood: not 500": (r) => r.status !== 500,
  });
}
