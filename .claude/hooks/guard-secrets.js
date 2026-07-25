#!/usr/bin/env bun
/**
 * PreToolUse hook: deny Bash commands that read secret / credential files.
 *
 * Claude Code's `Read(...)` deny rules only govern the Read / Grep / Glob tools.
 * They do NOT stop the Bash tool from reading the same files via `cat`, `grep`,
 * `sed`, `awk`, redirection (`< .env`), or an interpreter one-liner such as
 * `node -e "require('fs').readFileSync('.env')"`. This hook inspects the actual
 * Bash command string and blocks anything that names a secret path — which
 * catches every reader uniformly, because they all must reference the file.
 *
 * Scope / limits: defense-in-depth, NOT a hard boundary. Env-var indirection,
 * base64/hex, or heavy obfuscation can still evade a string match. Pair with
 * OS/sandbox isolation, secrets kept out of the tree, and a system-level
 * managed-settings policy (see the repo security notes).
 *
 * Contract: reads the tool-call JSON on stdin; on a match prints a PreToolUse
 * `permissionDecision: deny` object to stdout and exits 0. Fails open (exit 0,
 * no decision) on unexpected input so it never wedges the tool loop.
 *
 * Runtime-agnostic: works under `bun` or `node` (stdin via data/end events).
 */

// [pattern, human label] — matched case-insensitively against the full command.
// Deliberately file/path oriented to limit false positives.
const PATTERNS = [
  [/(?:^|[\s=<>|&(;"'\/])\.env(?:\.[\w.-]+)?(?![\w.-])/i, ".env file"],
  [/(?:^|[\s=<>|&(;"'\/])\.envrc(?![\w.-])/i, ".envrc"],
  [/\/secrets?\//i, "secrets directory"],
  [/\bsecrets?\.(?:env|ya?ml|json)\b/i, "secrets file"],
  [/[\w./-]+\.(?:pem|pfx|p12)(?![\w])/i, "certificate/key file"],
  [/[\w./-]+\.key(?![\w])/i, "key file"],
  [/\b(?:id_rsa|id_ed25519|[\w-]+_rsa)\b/i, "private key"],
  [/(?:^|[\s=<>|&(;"'\/])\.aws\//i, "AWS credentials directory"],
  [/\bcredentials\.json\b/i, "credentials.json"],
  [/\bprintenv\b/i, "environment dump (printenv)"],
  [/(?:process\.env\b|os\.environ)/i, "process environment access"],
];

function decide(command) {
  for (const [pattern, label] of PATTERNS) {
    if (pattern.test(command)) {
      const reason =
        `Denied by secret-guard hook: the command references a ${label}. ` +
        "Reading secret/credential files via Bash is blocked by repo policy " +
        "(Read-tool denies do not cover Bash). Use the intended secret " +
        "workflow instead of reading the file directly.";
      return {
        hookSpecificOutput: {
          hookEventName: "PreToolUse",
          permissionDecision: "deny",
          permissionDecisionReason: reason,
        },
      };
    }
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
    process.exit(0); // fail open: never block on unparseable hook input
  }
  if (!data || data.tool_name !== "Bash") process.exit(0);
  const command =
    data.tool_input && typeof data.tool_input.command === "string"
      ? data.tool_input.command
      : "";
  if (!command) process.exit(0);

  const decision = decide(command);
  if (decision) process.stdout.write(JSON.stringify(decision) + "\n");
  process.exit(0);
});
