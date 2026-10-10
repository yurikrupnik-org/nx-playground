#!/usr/bin/env bun
/**
 * PreToolUse hook: deny Bash git operations that bypass the pull-request path.
 *
 * Everything Claude changes — locally or in the Claude Maintenance workflow —
 * lands on a branch and goes through a PR. `--allowedTools` in the workflow
 * cannot express "git push, but never to main", so the command string is
 * inspected here. Blocked:
 *   - `git push` whose refspec names main/master, or that uses --force/-f/
 *     --force-with-lease/--delete
 *   - `git commit` / `git merge` / `git rebase` while HEAD is main or master
 *     (checked by the hook, no git call needed: reads .git/HEAD of the cwd
 *     the command runs in — `cd <dir> && git …` is honoured)
 *   - `gh pr merge`, `gh repo delete`, `gh release delete`
 *   - `git commit` in a repo with a lefthook config whose prepare-commit-msg /
 *     commit-msg hooks are not lefthook's: without them the commit gets no
 *     `Assisted-by:` trailer and CI cannot tell it from a human one
 *     (tools/authorship). Asks for `lefthook install`; one git call locates
 *     the hooks dir (honours core.hooksPath and worktrees).
 *
 * Same contract as guard-secrets.js: tool-call JSON on stdin; on a match prints
 * a PreToolUse deny decision and exits 0; fails open on unexpected input.
 */

import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";

const LEFTHOOK_CONFIGS = [
  "lefthook.yml",
  ".lefthook.yml",
  "lefthook.yaml",
  ".lefthook.yaml",
  "lefthook.toml",
  ".lefthook.toml",
  "lefthook.json",
  ".lefthook.json",
];
const ATTRIBUTION_HOOKS = ["prepare-commit-msg", "commit-msg"];

const PROTECTED = /^(?:main|master)$/;

function currentBranch(dir) {
  // Walk up to the enclosing repo, follow a gitfile (worktrees), read HEAD.
  let cur = dir;
  for (;;) {
    const dotGit = resolve(cur, ".git");
    if (existsSync(dotGit)) {
      let gitDir = dotGit;
      try {
        const stat = readFileSync(dotGit, "utf8");
        const m = /^gitdir:\s*(.+)$/m.exec(stat);
        if (m) gitDir = resolve(cur, m[1].trim());
      } catch {
        // a directory: readFileSync throws EISDIR, keep dotGit
      }
      try {
        const head = readFileSync(resolve(gitDir, "HEAD"), "utf8").trim();
        const ref = /^ref:\s*refs\/heads\/(.+)$/.exec(head);
        return ref ? ref[1] : null; // detached HEAD → null
      } catch {
        return null;
      }
    }
    const parent = resolve(cur, "..");
    if (parent === cur) return null;
    cur = parent;
  }
}

// Attribution hooks missing from the repo at `dir`, or [] when installed, when
// the repo has no lefthook config, or when git cannot tell (fail open).
function missingLefthookHooks(dir) {
  const r = spawnSync("git", ["-C", dir, "rev-parse", "--show-toplevel", "--git-path", "hooks"], {
    encoding: "utf8",
  });
  if (r.status !== 0 || typeof r.stdout !== "string") return [];
  const [top, hooks] = r.stdout.trim().split("\n");
  if (!top || !hooks || !LEFTHOOK_CONFIGS.some((f) => existsSync(resolve(top, f)))) return [];
  return ATTRIBUTION_HOOKS.filter((name) => {
    try {
      return !readFileSync(resolve(dir, hooks, name), "utf8").includes("lefthook");
    } catch {
      return true; // not installed
    }
  });
}

function deny(reason, advice = "Open a pull request from a branch instead.") {
  return {
    hookSpecificOutput: {
      hookEventName: "PreToolUse",
      permissionDecision: "deny",
      permissionDecisionReason: `Denied by git-guard hook: ${reason} ${advice}`,
    },
  };
}

// Effective directory of a `cd X && git …` prefix, else the hook cwd.
function effectiveDir(command, cwd) {
  const m = /^\s*cd\s+(?:"([^"]+)"|'([^']+)'|(\S+))\s*&&/.exec(command);
  const dir = m ? m[1] ?? m[2] ?? m[3] : null;
  return dir ? resolve(cwd, dir.replace(/^~/, process.env.HOME ?? "~")) : cwd;
}

function decide(command, cwd) {
  const segments = command.split(/&&|\|\||;|\|/);
  for (const raw of segments) {
    const seg = raw.trim();

    const push = /^git\s+(?:-C\s+\S+\s+)?push\b(.*)$/.exec(seg);
    if (push) {
      const args = push[1];
      if (/(?:^|\s)(?:--force(?:-with-lease)?|-f|--delete|-d)(?:\s|$|=)/.test(args)) {
        return deny("force pushes and remote branch deletion are not allowed.");
      }
      // Any positional refspec naming a protected branch: `push origin main`,
      // `push origin HEAD:main`, `push -u origin main`.
      const words = args.split(/\s+/).filter(Boolean);
      for (const w of words) {
        const target = w.includes(":") ? w.split(":").pop() : w;
        if (PROTECTED.test(target.replace(/^refs\/heads\//, ""))) {
          return deny(`pushing to '${target}' is not allowed.`);
        }
      }
      // No refspec: resolves to the current branch.
      const branch = currentBranch(effectiveDir(command, cwd));
      if (words.every((w) => w.startsWith("-") || w === "origin") && branch && PROTECTED.test(branch)) {
        return deny(`HEAD is '${branch}'; pushing it is not allowed.`);
      }
    }

    if (/^git\s+(?:-C\s+\S+\s+)?(?:commit|merge|rebase|cherry-pick)\b/.test(seg)) {
      const branch = currentBranch(effectiveDir(command, cwd));
      if (branch && PROTECTED.test(branch)) {
        return deny(`HEAD is '${branch}'; committing on it is not allowed.`);
      }
    }

    const commit = /^git\s+(?:-C\s+(\S+)\s+)?commit\b/.exec(seg);
    if (commit) {
      const base = effectiveDir(command, cwd);
      const missing = missingLefthookHooks(commit[1] ? resolve(base, commit[1]) : base);
      if (missing.length > 0) {
        return deny(
          `the lefthook git hooks are not installed in this repo (${missing.join(", ")}), so the commit would not get its Assisted-by trailer.`,
          "Run `lefthook install` in the repository, then retry the commit.",
        );
      }
    }

    if (/^gh\s+pr\s+merge\b/.test(seg)) return deny("merging pull requests is a human decision.");
    if (/^gh\s+(?:repo|release)\s+delete\b/.test(seg)) return deny("deleting repositories or releases is not allowed.");
  }
  return null;
}

let raw = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", (chunk) => {
  raw += chunk;
});
process.stdin.on("end", () => {
  let data;
  try {
    data = JSON.parse(raw);
  } catch {
    process.exit(0);
  }
  if (!data || data.tool_name !== "Bash") process.exit(0);
  const command =
    data.tool_input && typeof data.tool_input.command === "string" ? data.tool_input.command : "";
  if (!command) process.exit(0);
  const cwd = typeof data.cwd === "string" && data.cwd ? data.cwd : process.cwd();

  const decision = decide(command, cwd);
  if (decision) process.stdout.write(JSON.stringify(decision) + "\n");
  process.exit(0);
});
