// pixel-managed: opencode-pixel.js — managed by `pixel install`; replaced on every install.
//
// OpenCode V2 plugin. Auto-loaded from ~/.config/opencode/plugins/ (and
// .opencode/plugins/): OpenCode discovers direct .ts and .js files there
// without a `plugins` entry in opencode.json, so install deploys only this
// file — no config edit, no double-load.
//
// What it adds (issue #603): a first-class `pixel` tool (the shortest route
// from a task to evidence), Pixel-first enforcement on native shell retrieval
// via `tool.execute.before` (input replacement — V2 before-hooks cannot block,
// only replace input), and post-edit blast-radius + metrics delivery via
// `tool.execute.after`. The AGENTS.md block (deployed by the same install
// step) carries the doctrine; this plugin carries the enforcement and the
// metrics the doctrine cannot. Pixel is resolved from $PIXEL_BIN or PATH at
// runtime (install does not bake the binary path into this file), and every
// pixel call is best-effort: an unavailable Pixel leaves native tools and the
// session intact.
//
// Standalone: only `node:*` imports, no `@opencode/plugin` dependency, so the
// file loads without a package.json. V2 reads the default export's `id` and
// `setup(ctx)`; `Plugin.define` is not required.
import { spawnSync } from "node:child_process";
import { realpathSync, statSync } from "node:fs";
import { isAbsolute, relative, resolve, sep } from "node:path";

const READ_LIMIT = 200;
const MAX_OUTPUT = 16000;
const POLICIES = new Set(["advisory", "enforce", "off"]);
const EDIT_TOOLS = new Set([
  "edit", "write", "apply_patch", "edit_file", "replace_file_content",
  "write_to_file", "notebook_edit", "multiedit",
]);
const SHELL_TOOLS = new Set([
  "bash", "run_command", "shell", "local_shell", "unified_exec", "exec_command",
]);
// A repo-read leaf pixel owns. Mirrors the Rust guard's block table; only the
// clear, single-segment shapes are redirected — compositions stay native.
const REPO_SEARCH_REASONS = {
  rg: "repository search: use `pixel search-content <pattern>`",
  grep: "repository search: use `pixel search-content <pattern>`",
};
const REPO_READ_REASONS = {
  cat: "repository read: use `pixel search-content -F <id>` or `pixel pack-context <uid>`",
  head: "repository read: use `pixel search-content -F <id>` or `pixel pack-context <uid>`",
  tail: "repository read: use `pixel search-content -F <id>` or `pixel pack-context <uid>`",
};
const GIT_REASONS = {
  status: "repository inspection: use `pixel repo-state`",
  diff: "repository inspection: use `pixel review-changes`",
  log: "repository inspection: use `pixel commit-history`",
};
const LS_REASON = "repository discovery: use `pixel list-areas` or `pixel find-code`";
const FIND_REASON = "repository discovery: use `pixel find-code`";
const CREDENTIAL_PATH =
  /(?:^|\/)(?:\.env(?:\.[^/]*)?|\.ssh|\.aws|\.npmrc|\.netrc|id_(?:rsa|ed25519|ecdsa|dsa)[^/]*|[^/]*\.(?:pem|key|p12|pfx)|[^/]*(?:credentials|secrets?)[^/]*)(?:\/|$)/i;

// --- pixel invocation -----------------------------------------------------

// Resolve the pixel binary at call time (not module load) so a PIXEL_BIN set
// after the plugin loads — or changed between sessions — takes effect.
function pixelBin() {
  return process.env.PIXEL_BIN || "pixel";
}

// Pixel prints its metrics box (🟩 pixel …) on stderr. Keep only the box lines
// so the user sees the savings line, never diagnostics. Mirrors pi's helper.
function metricsBox(stderr) {
  const lines = String(stderr ?? "").split("\n");
  const start = lines.findIndex((l) => l.includes("🟩") && l.toLowerCase().includes("pixel"));
  if (start < 0) return "";
  const rest = lines.slice(start);
  const closing = rest.findIndex((l) => l.trimStart().startsWith("└"));
  const blank = rest.findIndex((l) => !l.trim());
  const end = closing >= 0 ? closing + 1 : blank >= 0 ? blank : rest.length;
  return rest.slice(0, end).join("\n").trim();
}

// Synchronous pixel run for tool-facing calls (keeps the metrics box) and
// quiet probes (no box). Throws on non-zero exit so callers can mark health.
//
// `env` is passed explicitly: bun's `spawnSync` does not always propagate a
// `process.env` mutation made after import to the child the way node does,
// so a runtime `PIXEL_METRICS=0` opt-out set by the host would be lost if we
// relied on default inheritance. `--metrics off` (quiet probes) still wins
// because pixel applies the explicit flag over the env var.
function runSync(root, args, quiet = false) {
  const full = [operationFor(args[0]), ...args.slice(1), ...(quiet ? ["--metrics", "off"] : [])];
  const result = spawnSync(pixelBin(), full, {
    cwd: root, encoding: "utf8", timeout: 15000, maxBuffer: 2_000_000,
    env: { ...process.env },
  });
  if (result.error || result.status !== 0) {
    const detail = result.error?.message ?? result.stderr?.trim() ?? `pixel exited ${result.status}`;
    throw new Error(detail);
  }
  return { stdout: result.stdout, box: quiet ? "" : metricsBox(result.stderr) };
}

function operationFor(name) {
  return name; // V2-era operation names; legacy aliases are not remapped here.
}

function pixelHealthy(root) {
  try {
    const { stdout } = runSync(root, ["status", "--json"], true);
    const status = JSON.parse(stdout);
    if (!status?.index?.base_files) {
      return { ok: false, reason: "Pixel index unavailable; run `pixel build-index --history .`" };
    }
    return { ok: true, status };
  } catch (error) {
    return { ok: false, reason: String(error) };
  }
}

function policyFor(root) {
  if (["0", "false", "off"].includes((process.env.PIXEL_TARGETS_GUARD ?? "").trim().toLowerCase())) {
    return "off";
  }
  const override = (process.env.PIXEL_POLICY ?? "").trim().toLowerCase();
  if (override) return POLICIES.has(override) ? override : "advisory";
  try {
    const { stdout } = runSync(root, ["config", "policy", "--json"], true);
    const reported = JSON.parse(stdout).policy;
    return POLICIES.has(reported) ? reported : "advisory";
  } catch {
    return "advisory"; // Unreadable configuration keeps the advisory default.
  }
}

// --- path containment (port of the Rust guard's readable_repo_file) ------

function inRepo(root, path) {
  if (!path) return false;
  const abs = isAbsolute(path) ? path : resolve(root, path);
  let canonicalRoot, canonical;
  try {
    canonicalRoot = realpathSync(root);
    canonical = realpathSync(abs);
  } catch {
    return false;
  }
  return canonical === canonicalRoot || canonical.startsWith(canonicalRoot + sep);
}

function readableRepoFile(root, path) {
  if (!path || path === "-") return false;
  let canonical, canonicalRoot;
  try {
    canonicalRoot = realpathSync(root);
    canonical = realpathSync(isAbsolute(path) ? path : resolve(root, path));
  } catch {
    return false;
  }
  const rel = relative(canonicalRoot, canonical);
  if (!rel || rel === ".." || rel.startsWith("../") || isAbsolute(rel)) return false;
  const head = rel.split(sep)[0];
  if (head === ".git" || head === ".pixel") return false;
  if (CREDENTIAL_PATH.test(rel)) return false;
  try {
    return statSync(canonical).isFile();
  } catch {
    return false;
  }
}

// --- shell leaf classification --------------------------------------------

function baseName(name) {
  return (name ?? "").split("/").at(-1);
}

// A bounded shell tokenizer: quotes are accepted, expansions/escapes/redir/
// unquoted-globs are not. Mirrors pi's tokenizeShell; returns undefined for
// anything the bounded grammar rejects (the command then stays native).
function tokenize(segment) {
  const args = [];
  let current = "";
  let started = false;
  let quote = null;
  for (let i = 0; i < segment.length; i++) {
    const c = segment[i];
    if (c === "\0") return undefined;
    if (quote === "'") {
      if (c === "'") quote = null;
      else current += c;
      continue;
    }
    if (c === "\n" || c === "\r" || c === "\\" || c === "$" || c === "`") return undefined;
    if (quote !== null) {
      if (c === quote) quote = null;
      else current += c;
      continue;
    }
    if (c === "'" || c === '"') { quote = c; started = true; continue; }
    if (";|<>&()*?[]{}~#".includes(c)) return undefined;
    if (c === " " || c === "\t") {
      if (started) { args.push(current); current = ""; started = false; }
      continue;
    }
    if (/\s/.test(c)) return undefined;
    current += c;
    started = true;
  }
  if (quote !== null) return undefined;
  if (started) args.push(current);
  return args;
}

// Split only unquoted | ; & && || and newlines. Returns undefined for an
// empty segment or an unterminated quote (the compound stays native). Each
// segment carries `piped` (true when it follows a single `|`) so an rg/grep
// without an explicit path in a pipeline stays native — mirroring pi's guard.
function splitSegments(text) {
  const segments = [];
  let quote = null, start = 0, i = 0, piped = false;
  while (i < text.length) {
    const c = text[i];
    if (quote !== null) { if (c === quote) quote = null; i++; continue; }
    if (c === "'" || c === '"') { quote = c; i++; continue; }
    if (c === "|" || c === ";" || c === "&" || c === "\n") {
      const seg = text.slice(start, i).trim();
      if (!seg) return undefined;
      segments.push({ text: seg, piped });
      const doubled = (c === "|" || c === "&") && text[i + 1] === c;
      piped = c === "|" && !doubled;
      i = doubled ? i + 2 : i + 1;
      start = i;
      continue;
    }
    i++;
  }
  if (quote !== null) return undefined;
  const last = text.slice(start).trim();
  if (!last) return undefined;
  segments.push({ text: last, piped });
  return segments;
}

// One shell leaf at a time. Returns a redirect reason string when the leaf
// is a repo read/search/inspection pixel owns, else undefined (stay native).
// Conservative on purpose: rtk wrappers, cd compounds, unknown binaries and
// anything the bounded grammar rejects all stay native.
function leafReason(command, root) {
  const trimmed = command.trim();
  if (!trimmed) return undefined;
  const segs = splitSegments(trimmed);
  if (!segs) return undefined;
  for (const seg of segs) {
    const words = tokenize(seg.text);
    if (!words) return undefined;
    if (words[0] === "cd" || words[0] === "command" || words[0] === "builtin") return undefined;
    const reason = classifyLeaf(words, root, seg.piped);
    if (reason) return reason;
  }
  return undefined;
}

function classifyLeaf(words, root, piped) {
  let [bin, ...args] = words;
  if (bin === "rtk" && args.length > 0 && ["cat", "head", "tail", "awk", "sed", "read"].includes(args[0])) {
    bin = args[0];
    args = args.slice(1);
  }
  const nonFlag = args.filter((a) => !a.startsWith("-"));
  switch (baseName(bin)) {
    case "rg":
    case "grep": {
      const explicitPath = nonFlag.length >= 2;
      const path = explicitPath ? nonFlag.at(-1) : ".";
      if (piped && !explicitPath) return undefined; // pipeline; stay native
      if (CREDENTIAL_PATH.test(path)) return undefined;
      if (!inRepo(root, path)) return undefined;
      return REPO_SEARCH_REASONS[baseName(bin)];
    }
    case "git": {
      const sub = nonFlag[0];
      if (!sub || !GIT_REASONS[sub]) return undefined;
      return GIT_REASONS[sub];
    }
    case "cat":
    case "head":
    case "tail": {
      const paths = nonFlag;
      if (paths.some((p) => CREDENTIAL_PATH.test(p))) return undefined;
      if (!paths.some((p) => inRepo(root, p))) return undefined;
      return REPO_READ_REASONS[baseName(bin)];
    }
    case "ls":
    case "tree": {
      const path = [...args].reverse().find((a) => !a.startsWith("-")) ?? ".";
      if (CREDENTIAL_PATH.test(path)) return undefined;
      if (!inRepo(root, path)) return undefined;
      return LS_REASON;
    }
    case "find": {
      const path = args[0];
      if (!path || CREDENTIAL_PATH.test(path)) return undefined;
      if (!inRepo(root, path)) return undefined;
      return FIND_REASON;
    }
    default:
      return undefined;
  }
}

// --- post-edit blast radius (port of pi's formatWhatChanged) --------------

function parseEvidence(text) {
  try { return JSON.parse(text); }
  catch {
    return text.split("\n").filter(Boolean).slice(0, 40).map((l) => {
      try { return JSON.parse(l); } catch { return l; }
    });
  }
}

function formatWhatChanged(stdout) {
  const j = parseEvidence(stdout);
  if (!j || typeof j !== "object") return null;
  const lines = [`changed_files: ${j.changed_files ?? 0}  risk: ${j.risk ?? "?"}`];
  const syms = j.symbols ?? [];
  for (const s of syms.slice(0, 15)) {
    const procs = s.processes_total ? `  (${s.processes_total} flows)` : "";
    lines.push(`  ${s.change}  ${s.name}  ${s.path}${procs}`);
  }
  if (syms.length > 15) lines.push(`  …+${syms.length - 15} more`);
  const tests = j.suggested_tests ?? [];
  if (tests.length) lines.push(`suggested tests: ${tests.slice(0, 10).join(", ")}`);
  if (j.envelope_note) lines.push(`note: ${j.envelope_note}`);
  return lines.length > 1 ? lines.join("\n") : null;
}

// --- content append helper ------------------------------------------------

function appendText(result, text) {
  if (!text) return result;
  const next = { ...result };
  const content = result?.content;
  if (typeof content === "string") {
    next.content = [{ type: "text", text: content }, { type: "text", text }];
  } else if (Array.isArray(content)) {
    next.content = [...content, { type: "text", text }];
  } else {
    next.content = [{ type: "text", text }];
  }
  return next;
}

// --- the plugin -----------------------------------------------------------

export default {
  id: "pixel",

  async setup(ctx) {
    // Per-session state. The hooks carry sessionID, so key on it; a missing
    // id (older runtime) falls back to a single shared slot.
    const state = new Map();
    const slot = (id) => {
      const key = id ?? "__default__";
      let s = state.get(key);
      if (!s) { s = { healthy: undefined, called: false }; state.set(key, s); }
      return s;
    };
    const root = () => ctx.location?.directory ?? process.cwd();

    const regs = [];

    // A first-class `pixel` tool: the structured shortest route from a task
    // to evidence. Optional — if the tool transform API is unavailable the
    // hooks still work.
    try {
      const reg = await ctx.tool.transform((editor) => {
        editor.add({
          name: "pixel",
          description:
            "Deterministic repository retrieval and Git workflow via the `pixel` CLI. " +
            "Actions: execution_brief (ordered task-to-evidence route; use this first), " +
            "scope_task (P0/P1 file list), search_content (exact id), find_code (behavior), " +
            "impact (caller/callee), pack_context (budgeted context), what_changed " +
            "(blast radius), review_changes. Returns JSON evidence and the exact Pixel " +
            "metrics line for this invocation.",
          input: {
            type: "object",
            additionalProperties: false,
            properties: {
              action: {
                type: "string",
                enum: [
                  "execution_brief", "scope_task", "search_content", "find_code", "impact",
                  "pack_context", "what_changed", "review_changes",
                ],
              },
              task: { type: "string", description: "task description for the ordered execution_brief route" },
              goal: { type: "string", description: "task/goal for scope_task" },
              query: { type: "string", description: "pattern/term for search_content/find_code" },
              symbol: { type: "string", description: "symbol or path#name#kind uid for impact/pack_context" },
              path: { type: "string", description: "optional repo-relative path scope" },
            },
            required: ["action"],
          },
          options: {},
          async execute(input, context) {
            const cwd = ctx.location?.directory ?? process.cwd();
            const action = input?.action;
            const s = slot(context?.sessionID);
            try {
              const health = pixelHealthy(cwd);
              if (!health.ok) {
                s.healthy = false;
                return {
                  content: [{ type: "text", text: JSON.stringify({
                    action, error: health.reason,
                    next_action: "Repair Pixel (`pixel build-index --history .`) or use native tools",
                  }) }],
                };
              }
              const target = String(input.query ?? input.task ?? input.goal ?? "").trim();
              const scope = input.path ? [String(input.path)] : [];
              const subject = action === "impact" || action === "pack_context"
                ? (input.symbol ?? target) : target;
              if (["execution_brief", "scope_task", "search_content", "find_code", "impact", "pack_context"].includes(action) && !String(subject).trim()) {
                return { content: [{ type: "text", text: JSON.stringify({
                  action, error: `${action} requires a goal, query, or symbol`,
                }) }] };
              }
              const args = commandFor(action, input, scope, subject);
              const { stdout, box } = runSync(cwd, args);
              const evidence = parseEvidence(stdout.slice(0, MAX_OUTPUT));
              const truncated = stdout.length > MAX_OUTPUT;
              s.healthy = true;
              s.called = true;
              const result = {
                action, evidence, truncated,
                next_action: truncated ? "Narrow the scope or query" : undefined,
              };
              const content = [{ type: "text", text: JSON.stringify(result) }];
              if (box) content.push({ type: "text", text: box });
              return { content };
            } catch (error) {
              s.healthy = false;
              return {
                content: [{ type: "text", text: JSON.stringify({
                  action, error: String(error),
                  next_action: "Repair Pixel availability or narrow the request; native tools remain available",
                }) }],
              };
            }
          },
        });
      });
      if (reg?.dispose) regs.push(reg);
    } catch {
      // Tool registration is a bonus; the hooks are the core.
    }

    // tool.execute.before — Pixel-first enforcement on native shell retrieval.
    // V2 before-hooks cannot block a tool; they can only replace its input, so
    // enforcement rewrites a repo-read shell command into an `echo` of the
    // Pixel redirect. Read/grep/glob native tools are left alone (input
    // replacement cannot produce a useful redirect for them); the AGENTS.md
    // doctrine and the `pixel` tool carry the guidance there.
    try {
      const before = await ctx.tool.hook("execute.before", (event) => {
        const s = slot(event.sessionID);
        if (event.tool === "pixel") { s.called = true; return; }
        const mode = policyFor(root());
        if (mode === "off") return;
        if (!SHELL_TOOLS.has(event.tool)) return;
        const cmd = String(event.input?.command ?? event.input?.cmd ?? "");
        if (!cmd) return;
        // A pixel shell call is the model using pixel directly: count it.
        if (baseName(cmd.split(/\s+/)[0]) === "pixel" || baseName(cmd.split(/\s+/)[0]) === "pixel-dev") {
          s.called = true;
          return;
        }
        if (mode !== "enforce" || s.healthy === false) return;
        const health = pixelHealthy(root());
        if (!health.ok) { s.healthy = false; return; }
        s.healthy = true;
        const reason = leafReason(cmd, root());
        if (!reason) return;
        // Replace the command so the tool still runs (no block) but returns
        // the Pixel redirect instead of performing the repo operation.
        const redirect = `echo ${JSON.stringify(`[pixel] ${reason}`)}`;
        if (typeof event.input === "object" && event.input !== null) {
          if ("command" in event.input) event.input.command = redirect;
          else if ("cmd" in event.input) event.input.cmd = redirect;
        }
      });
      if (before?.dispose) regs.push(before);
    } catch {
      // Enforcement is best-effort.
    }

    // tool.execute.after — post-edit blast radius + metrics observability.
    try {
      const after = await ctx.tool.hook("execute.after", (event) => {
        const s = slot(event.sessionID);
        if (event.tool === "pixel") {
          if (event.status === "completed") { s.healthy = true; s.called = true; }
          else if (event.status === "error") s.healthy = false;
          return;
        }
        if (event.status !== "completed") return;
        if (!EDIT_TOOLS.has(event.tool)) return;
        // Post-edit blast radius: a compact `what-changed` note appended to
        // the edit's own result. Quiet (no metrics box) so it never double-
        // counts. Failure is silent — edits stay allowed when Pixel is down.
        try {
          const { stdout } = runSync(root(), ["what-changed", "--json", "--tests"], true);
          const compact = formatWhatChanged(stdout);
          if (compact) event.result = appendText(event.result, `PIXEL BLAST RADIUS (pixel what-changed):\n\n${compact}\n\nEdits are unverified until the project's build/tests run.`);
        } catch {
          s.healthy = false;
        }
      });
      if (after?.dispose) regs.push(after);
    } catch {
      // Post-edit follow-through is best-effort.
    }

    return () => { for (const r of regs) { try { r.dispose(); } catch {} } };
  },
};

// Build the pixel CLI argv for a tool action. Kept here (not in setup) so a
// focused JS test can exercise it without spinning the plugin.
export function commandFor(action, input, scope, subject) {
  const p = scope ?? [];
  switch (action) {
    case "execution_brief":
      return ["execution-brief", String(input.task ?? input.goal ?? subject), ...p, "--json"];
    case "scope_task":
      return ["scope-task", String(input.goal ?? subject), ...p, "--json"];
    case "search_content":
      return ["search-content", String(input.query ?? subject), ...p, "--json", "--limit", "40"];
    case "find_code":
      return ["find-code", String(input.query ?? subject), ...p, "--json", "--limit", "5"];
    case "impact":
      return ["impact", String(input.symbol ?? subject), ...p, "--json"];
    case "pack_context":
      return ["pack-context", String(input.symbol ?? subject), ...p, "--json", "--budget", "1200"];
    case "what_changed":
      return ["what-changed", ...p, "--json", "--tests"];
    case "review_changes":
      return ["review-changes", ...p, "--json"];
    default:
      throw new Error(`unknown pixel action: ${action}`);
  }
}

export const _internals = {
  metricsBox, runSync, pixelHealthy, policyFor, leafReason, tokenize,
  splitSegments, formatWhatChanged, appendText, baseName, inRepo, readableRepoFile,
};
