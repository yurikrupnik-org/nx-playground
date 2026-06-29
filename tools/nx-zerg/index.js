// @zerg/nx — thin Nx shim over the `zergctl` Rust binary.
//
// Nx plugins load in Node, so this file stays tiny: it forwards every matched
// Tiltfile to `zergctl graph nodes`, the real inference engine (tools/cli, see
// graph.rs), and returns the createNodesV2 JSON it prints.
//
// Marker: any Tiltfile. Deployability — a sibling Cargo.toml `[package]` (Rust)
// or a package.json `build` script (static) — decides the targets, not the path.
//
// Conventions (image prefix, dockerfile paths, registry, apps dir) come from this
// plugin's `options` in nx.json and are forwarded to the binary verbatim, so one
// published plugin + binary serves any Nx workspace with no code changes.
//
// Binary resolution (first hit wins):
//   1. options.binary           — explicit path/name override
//   2. local {dist/,}target/    — dev builds in this (the tools) repo
//   3. `zergctl` on PATH        — `cargo install`ed in a consuming repo
//   4. `cargo run -p zergctl`   — last-resort dev fallback (needs the crate)

const { execFileSync } = require("node:child_process");
const { join, delimiter } = require("node:path");
const { existsSync } = require("node:fs");

const MARKER = "**/Tiltfile";

// Custom target-dir is dist/target per .cargo/config.toml; plain target/ too.
const BIN_CANDIDATES = [
  "dist/target/release/zergctl",
  "dist/target/debug/zergctl",
  "target/release/zergctl",
  "target/debug/zergctl",
];

function findOnPath(name) {
  for (const dir of (process.env.PATH || "").split(delimiter)) {
    if (dir && existsSync(join(dir, name))) return join(dir, name);
  }
  return null;
}

function resolveRunner(workspaceRoot, options) {
  const exe = process.platform === "win32" ? ".exe" : "";
  if (options && options.binary) return { cmd: options.binary, pre: [] };
  const local = BIN_CANDIDATES.map((p) => join(workspaceRoot, p + exe)).find(existsSync);
  if (local) return { cmd: local, pre: [] };
  const onPath = findOnPath("zergctl" + exe);
  if (onPath) return { cmd: onPath, pre: [] };
  return { cmd: "cargo", pre: ["run", "-q", "-p", "zergctl", "--"] };
}

function runEngine(workspaceRoot, options, files) {
  const { cmd, pre } = resolveRunner(workspaceRoot, options);
  const args = [...pre, "graph", "nodes", "--workspace", workspaceRoot];
  if (options && Object.keys(options).length > 0) {
    args.push("--config-json", JSON.stringify(options));
  }
  args.push(...files);

  const out = execFileSync(cmd, args, {
    cwd: workspaceRoot,
    encoding: "utf8",
    maxBuffer: 32 * 1024 * 1024,
  });
  return JSON.parse(out);
}

const createNodesV2 = [
  MARKER,
  async (files, options, context) => {
    if (!files || files.length === 0) return [];
    return runEngine(context.workspaceRoot, options, files);
  },
];

module.exports = { createNodesV2 };
