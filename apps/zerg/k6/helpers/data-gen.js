import http from "k6/http";

const BASE_URL = __ENV.BASE_URL || "http://localhost:3000";

const PRIORITIES = ["low", "medium", "high", "urgent"];
const STATUSES = ["todo", "in_progress", "done"];

function pick(arr) {
  return arr[Math.floor(Math.random() * arr.length)];
}

/**
 * Generate a random CreateTask payload.
 */
export function randomTask(projectId) {
  const task = {
    title: `Task ${__VU}-${__ITER}-${Date.now()}`,
    description: `Auto-generated task for load testing (VU=${__VU}, iter=${__ITER})`,
    priority: pick(PRIORITIES),
    status: pick(STATUSES),
  };
  if (projectId) {
    task.project_id = projectId;
  }
  return task;
}

/**
 * Generate a random UpdateTask payload (partial).
 */
export function randomTaskUpdate() {
  return {
    title: `Updated ${__VU}-${__ITER}-${Date.now()}`,
    priority: pick(PRIORITIES),
    status: pick(STATUSES),
  };
}

/**
 * Create a task via the API and return the response.
 */
export function createTask(task) {
  return http.post(`${BASE_URL}/api/tasks`, JSON.stringify(task), {
    headers: { "Content-Type": "application/json" },
  });
}

/**
 * List all tasks.
 */
export function listTasks() {
  return http.get(`${BASE_URL}/api/tasks`);
}

/**
 * Get a single task by ID.
 */
export function getTask(id) {
  return http.get(`${BASE_URL}/api/tasks/${id}`);
}

/**
 * Update a task by ID.
 */
export function updateTask(id, update) {
  return http.put(`${BASE_URL}/api/tasks/${id}`, JSON.stringify(update), {
    headers: { "Content-Type": "application/json" },
  });
}

/**
 * Delete a task by ID.
 */
export function deleteTask(id) {
  return http.del(`${BASE_URL}/api/tasks/${id}`);
}

/**
 * Create a task via the direct DB endpoint (bypasses gRPC).
 */
export function createTaskDirect(task) {
  return http.post(`${BASE_URL}/api/tasks-direct`, JSON.stringify(task), {
    headers: { "Content-Type": "application/json" },
  });
}

/**
 * List tasks via the direct DB endpoint.
 */
export function listTasksDirect() {
  return http.get(`${BASE_URL}/api/tasks-direct`);
}
