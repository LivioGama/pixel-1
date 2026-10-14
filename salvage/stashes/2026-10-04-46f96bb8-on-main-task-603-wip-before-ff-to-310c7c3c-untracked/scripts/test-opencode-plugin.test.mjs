// Persistent regression test for the opencode-pixel.js plugin logic.
//
// Run:    bun test scripts/test-opencode-plugin.test.mjs
// Runtime: Bun only (no Cargo, no @opencode/plugin). The plugin is imported
// directly from crates/pixel-install/assets/opencode-pixel.js and exercised
// against a fake `pixel` shell script and a mock V2 plugin context matching
// the asset's adapter calls. This verifies the adapter boundary, not that a
// live OpenCode session loaded the plugin or acted on its route.
//
// This suite verifies the adapter boundary with a fake Pixel CLI. It does not
// prove that a live OpenCode session follows the route or uses the returned
// path in a bounded read; those require the post-install real-session pass.
//
// If OpenCode is on PATH, capture its version for comparison with a later
// real-session test. Absence of OpenCode is not a failure.
import { spawnSync } from "node:child_process";
import {
  chmodSync,
  mkdtempSync,
  mkdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";
import { test, describe, beforeAll, afterAll, expect } from "bun:test";

const ASSET = fileURLToPath(
  new URL("../crates/pixel-install/assets/opencode-pixel.js", import.meta.url),
);

// --- capture the installed OpenCode version (real-session pass) ----------
// Printed to stdout once at load; tests do not depend on it. Uses a shell so
// the same PATH that found `opencode` for a human resolves it here too.
{
  const probe = spawnSync("sh", ["-c", "opencode --version 2>&1"], {
    encoding: "utf8",
  });
  if (probe.status === 0 && probe.stdout.trim()) {
    console.log(`OpenCode version: ${probe.stdout.trim()}`);
  } else {
    console.log(
      "OpenCode: not on PATH — version capture deferred to the real-session pass",
    );
  }
}

// --- fixture: a fake repo + a fake pixel binary --------------------------
const SCRATCH = mkdtempSync(join(tmpdir(), "opencode-pixel-regression-"));
const REPO = join(SCRATCH, "repo");
mkdirSync(join(REPO, "src"), { recursive: true });
writeFileSync(join(REPO, "src", "lib.rs"), "fn main() {}\n");

// Fake pixel. Answers the operations the plugin calls. Emits a 🟩 metrics
// box on stderr when not quiet, so the metrics-visibility test can see it.
const PIXEL = join(SCRATCH, "pixel");
writeFileSync(
  PIXEL,
  `#!/bin/sh
# Quiet probe: no metrics box. " $* " lets us match "--metrics off" POSIX-safely.
case " $* " in
  *"--metrics off"*) QUIET=1;;
esac
if [ "$1" = "status" ]; then
  printf '{"index":{"base_files":12},"graph":{"present":true},"facts":{"fresh":true}}'
  [ -n "$QUIET" ] || printf '\\n🟩 pixel · 12 files · saved 4.2s\\n└──────┘' >&2
  exit 0
fi
if [ "$1" = "config" ]; then printf '{"policy":"enforce"}'; exit 0; fi
if [ "$1" = "what-changed" ]; then
  printf '{"changed_files":2,"risk":"low","symbols":[{"change":"modified","name":"main","path":"src/lib.rs","processes_total":3}],"suggested_tests":["t1","t2"]}'
  exit 0
fi
if [ "$1" = "scope-task" ]; then
  printf '{"targets":[{"path":"src/lib.rs","tier":"P0"}]}'
  [ -n "$QUIET" ] || printf '\\n🟩 pixel · scoped · saved 1.0s\\n└──────┘' >&2
  exit 0
fi
if [ "$1" = "execution-brief" ]; then
  printf '{"task":"read guard behavior","steps":[{"order":1,"action":"find-code","command":"pixel find-code guard behavior"},{"order":2,"action":"bounded-read","command":"rtk sed -n 40,80p crates/pixel/src/guard.rs"}],"fallback":"after two unhelpful Pixel calls, use native search","validation":["run focused guard test"]}'
  if [ -z "$QUIET" ] && [ "$PIXEL_METRICS" != "0" ]; then
    printf '\\n🟩 pixel · execution-brief · #a1b2c3\\n└──────┘' >&2
  fi
  exit 0
fi
if [ "$1" = "search-content" ]; then printf '[]'; exit 0; fi
printf '{}'
[ -n "$QUIET" ] || printf '\\n🟩 pixel · ok\\n└──┘' >&2
exit 0
`,
);
chmodSync(PIXEL, 0o755);
process.env.PIXEL_BIN = PIXEL;

const mod = await import(ASSET);
const plugin = mod.default;
const { commandFor, _internals } = mod;

// --- mock OpenCode V2 plugin context -------------------------------------
function mockCtx() {
  const calls = { transform: 0, before: 0, after: 0, disposed: 0 };
  const regs = [];
  let addedTool = null;
  let beforeFn = null;
  let afterFn = null;
  return {
    ctx: {
      location: {
        directory: REPO,
        project: { id: "p", directory: REPO, canonical: REPO },
      },
      tool: {
        transform(editor) {
          calls.transform++;
          editor({
            add: (t) => {
              addedTool = t;
            },
            list: () => [],
            get: () => undefined,
            namespace: () => {},
            update: () => {},
            remove: () => {},
          });
          const reg = { dispose: () => calls.disposed++ };
          regs.push(reg);
          return reg;
        },
        hook(name, fn) {
          if (name === "execute.before") {
            calls.before++;
            beforeFn = fn;
          } else if (name === "execute.after") {
            calls.after++;
            afterFn = fn;
          }
          const reg = { dispose: () => calls.disposed++ };
          regs.push(reg);
          return reg;
        },
      },
    },
    calls,
    addedTool: () => addedTool,
    before: () => beforeFn,
    after: () => afterFn,
  };
}

afterAll(() => {
  rmSync(SCRATCH, { recursive: true, force: true });
});

// --- command building ----------------------------------------------------

describe("command building", () => {
  test("commandFor builds the argv for each action", () => {
    expect(commandFor("scope_task", { goal: "fix bug" }, [], "fix bug")).toEqual([
      "scope-task",
      "fix bug",
      "--json",
    ]);
    expect(commandFor("execution_brief", { task: "fix guard" }, [], "fix guard")).toEqual([
      "execution-brief",
      "fix guard",
      "--json",
    ]);
    expect(commandFor("search_content", { query: "foo" }, ["src"], "foo")).toEqual([
      "search-content",
      "foo",
      "src",
      "--json",
      "--limit",
      "40",
    ]);
    expect(commandFor("impact", { symbol: "main" }, [], "main")).toEqual([
      "impact",
      "main",
      "--json",
    ]);
    expect(commandFor("pack_context", { symbol: "main#fn" }, [], "main#fn")).toEqual([
      "pack-context",
      "main#fn",
      "--json",
      "--budget",
      "1200",
    ]);
    expect(commandFor("what_changed", {}, [], undefined)).toEqual([
      "what-changed",
      "--json",
      "--tests",
    ]);
    expect(commandFor("review_changes", {}, [], undefined)).toEqual([
      "review-changes",
      "--json",
    ]);
  });

  test("commandFor rejects an unknown action", () => {
    expect(() => commandFor("bogus", {}, [], undefined)).toThrow(/unknown pixel action/);
  });
});

// --- pixel tool result & metrics visibility ------------------------------

describe("pixel tool result & metrics visibility", () => {
  test("setup registers a pixel tool plus both hooks and returns a cleanup", async () => {
    const m = mockCtx();
    const cleanup = await plugin.setup(m.ctx);
    expect(plugin.id).toBe("pixel");
    expect(m.calls.transform).toBe(1);
    expect(m.calls.before).toBe(1);
    expect(m.calls.after).toBe(1);
    expect(typeof cleanup).toBe("function");
    expect(m.addedTool()).not.toBeNull();
    expect(m.addedTool().name).toBe("pixel");
    expect(m.addedTool().input.properties.action).toBeDefined();
    cleanup();
    expect(m.calls.disposed).toBe(3);
  });

  test("pixel tool execute returns JSON evidence and surfaces the metrics box", async () => {
    const m = mockCtx();
    await plugin.setup(m.ctx);
    const tool = m.addedTool();
    const res = await tool.execute(
      { action: "scope_task", goal: "fix bug" },
      { sessionID: "s1" },
    );
    const texts = res.content.filter((c) => c.type === "text").map((c) => c.text);
    const evidence = JSON.parse(texts.find((t) => t.startsWith("{")));
    expect(evidence.action).toBe("scope_task");
    expect(evidence.evidence).toEqual({
      targets: [{ path: "src/lib.rs", tier: "P0" }],
    });
    expect(texts.some((t) => t.includes("🟩 pixel · scoped"))).toBe(true);
  });

  test("execution_brief returns the ordered route and its exact per-call metrics separately", async () => {
    const m = mockCtx();
    await plugin.setup(m.ctx);
    const res = await m.addedTool().execute(
      { action: "execution_brief", task: "read guard behavior" },
      { sessionID: "route1" },
    );
    expect(res.content).toHaveLength(2);
    const route = JSON.parse(res.content[0].text);
    expect(route.action).toBe("execution_brief");
    expect(route.evidence.steps.map((step) => step.order)).toEqual([1, 2]);
    expect(route.evidence.steps[0].command).toBe("pixel find-code guard behavior");
    expect(route.evidence.steps[1].command).toMatch(/sed -n 40,80p crates\/pixel\/src\/guard\.rs/);
    expect(res.content[1]).toEqual({ type: "text", text: "🟩 pixel · execution-brief · #a1b2c3\n└──────┘" });
  });

  test("execution_brief passes PIXEL_METRICS=0 through and emits no metrics item", async () => {
    process.env.PIXEL_METRICS = "0";
    try {
      const m = mockCtx();
      await plugin.setup(m.ctx);
      const res = await m.addedTool().execute(
        { action: "execution_brief", task: "read guard behavior" },
        { sessionID: "route-quiet" },
      );
      expect(res.content).toHaveLength(1);
      expect(res.content[0].text).not.toContain("🟩 pixel");
    } finally {
      delete process.env.PIXEL_METRICS;
    }
  });
});

// --- enforce rewrite -----------------------------------------------------

describe("enforce rewrite", () => {
  test("before-hook replaces a repo-search shell command under enforce", async () => {
    delete process.env.PIXEL_POLICY; // fake pixel answers {"policy":"enforce"}
    const m = mockCtx();
    await plugin.setup(m.ctx);
    const before = m.before();
    const input = { command: "rg foo src" };
    before({ tool: "bash", sessionID: "e1", input });
    expect(input.command).toMatch(
      /\[pixel\] repository search: use `pixel search-content/,
    );
  });

  test("before-hook classifies the common repo-read leaves", async () => {
    const m = mockCtx();
    await plugin.setup(m.ctx);
    const before = m.before();
    const cases = [
      ["grep -rn foo src/lib.rs", /repository search/],
      ["git status", /pixel repo-state/],
      ["git diff", /pixel review-changes/],
      ["git log", /pixel commit-history/],
      ["cat src/lib.rs", /repository read/],
      ["ls src", /repository discovery/],
      ["find . -name x", /pixel find-code/],
    ];
    for (const [cmd, re] of cases) {
      const input = { command: cmd };
      before({ tool: "bash", sessionID: "e2", input });
      expect(input.command).toMatch(re);
    }
  });
});

// --- advisory / off fallbacks -------------------------------------------

describe("advisory / off fallbacks", () => {
  test("advisory policy does not rewrite the command", async () => {
    process.env.PIXEL_POLICY = "advisory";
    const m = mockCtx();
    await plugin.setup(m.ctx);
    const before = m.before();
    const input = { command: "rg foo src" };
    before({ tool: "bash", sessionID: "a1", input });
    expect(input.command).toBe("rg foo src");
    delete process.env.PIXEL_POLICY;
  });

  test("off policy never touches the command even for a repo search", async () => {
    process.env.PIXEL_POLICY = "off";
    const m = mockCtx();
    await plugin.setup(m.ctx);
    const before = m.before();
    const input = { command: "rg foo src" };
    before({ tool: "bash", sessionID: "o1", input });
    expect(input.command).toBe("rg foo src");
    delete process.env.PIXEL_POLICY;
  });

  test("non-shell tools are left untouched under enforce", async () => {
    // fake pixel answers enforce; read/grep native tools stay native.
    const m = mockCtx();
    await plugin.setup(m.ctx);
    const before = m.before();
    const read = { path: "src/lib.rs", limit: 50 };
    before({ tool: "read", sessionID: "n1", input: read });
    expect(read.path).toBe("src/lib.rs");
  });

  test("a credential path stays native (no redirect)", async () => {
    const m = mockCtx();
    await plugin.setup(m.ctx);
    const before = m.before();
    const input = { command: "cat ~/.ssh/id_rsa" };
    before({ tool: "bash", sessionID: "c1", input });
    expect(input.command).toBe("cat ~/.ssh/id_rsa");
  });

  test("a pipeline that reads from a pipe stays native", async () => {
    const m = mockCtx();
    await plugin.setup(m.ctx);
    const before = m.before();
    const input = { command: "echo hi | rg foo" };
    before({ tool: "bash", sessionID: "p1", input });
    expect(input.command).toBe("echo hi | rg foo");
  });
});

// --- unavailable Pixel (no hook failure) ---------------------------------

describe("unavailable Pixel without hook failure", () => {
  test("the pixel tool returns a repair hint instead of throwing", async () => {
    process.env.PIXEL_BIN = "/definitely/not/a/real/pixel-binary";
    const m = mockCtx();
    await plugin.setup(m.ctx);
    const tool = m.addedTool();
    const res = await tool.execute(
      { action: "execution_brief", task: "x" },
      { sessionID: "u1" },
    );
    const evidence = JSON.parse(res.content[0].text);
    expect(evidence.error).toMatch(/Pixel index unavailable|pixel exited|ENOENT|not a real/);
    expect(evidence.next_action).toMatch(/use native tools/);
    process.env.PIXEL_BIN = PIXEL;
  });

  test("the before-hook does not throw and does not rewrite when Pixel is down", async () => {
    process.env.PIXEL_BIN = "/definitely/not/a/real/pixel-binary";
    const m = mockCtx();
    await plugin.setup(m.ctx);
    const before = m.before();
    const input = { command: "rg foo src" };
    expect(() => before({ tool: "bash", sessionID: "u2", input })).not.toThrow();
    // Pixel down → s.healthy set false → no rewrite (enforce skips).
    expect(input.command).toBe("rg foo src");
    process.env.PIXEL_BIN = PIXEL;
  });

  test("the after-hook does not throw when Pixel is down and omits the blast radius", async () => {
    process.env.PIXEL_BIN = "/definitely/not/a/real/pixel-binary";
    const m = mockCtx();
    await plugin.setup(m.ctx);
    const after = m.after();
    const event = {
      tool: "edit",
      status: "completed",
      sessionID: "u3",
      input: {},
      result: { content: [{ type: "text", text: "edited" }] },
    };
    expect(() => after(event)).not.toThrow();
    const texts = event.result.content.map((c) => c.text);
    expect(texts).toEqual(["edited"]); // no blast-radius append when Pixel is down
    process.env.PIXEL_BIN = PIXEL;
  });
});

// --- post-edit follow-through -------------------------------------------

describe("post-edit follow-through", () => {
  test("after-hook appends the blast radius to a completed edit", async () => {
    const m = mockCtx();
    await plugin.setup(m.ctx);
    const after = m.after();
    const event = {
      tool: "edit",
      status: "completed",
      sessionID: "f1",
      input: {},
      result: { content: [{ type: "text", text: "edited" }] },
    };
    after(event);
    const texts = event.result.content.map((c) => c.text);
    expect(texts.some((t) => t.includes("PIXEL BLAST RADIUS"))).toBe(true);
    expect(texts.some((t) => t.includes("modified  main  src/lib.rs"))).toBe(true);
  });

  test("after-hook ignores non-edit tools and error results", async () => {
    const m = mockCtx();
    await plugin.setup(m.ctx);
    const after = m.after();
    const read = {
      tool: "read",
      status: "completed",
      sessionID: "f2",
      input: {},
      result: { content: [] },
    };
    after(read);
    expect(read.result.content).toEqual([]);
    const err = {
      tool: "edit",
      status: "error",
      sessionID: "f3",
      input: {},
      error: { message: "x" },
    };
    expect(() => after(err)).not.toThrow();
    expect(err.error.message).toBe("x");
  });
});

// --- session isolation ---------------------------------------------------

describe("session isolation", () => {
  test("a pixel success in session A and a pixel error in session B isolate per session", async () => {
    delete process.env.PIXEL_POLICY; // enforce from the fake pixel
    const m = mockCtx();
    await plugin.setup(m.ctx);
    const before = m.before();
    const after = m.after();

    // Session A: a completed pixel call marks it healthy.
    after({
      tool: "pixel",
      status: "completed",
      sessionID: "A",
      input: {},
      result: { content: [] },
    });
    // Session B: a failed pixel call marks it unhealthy.
    after({
      tool: "pixel",
      status: "error",
      sessionID: "B",
      input: {},
      error: { message: "down" },
    });

    // Same repo-search command in each session.
    const a = { command: "rg foo src" };
    before({ tool: "bash", sessionID: "A", input: a });
    const b = { command: "rg foo src" };
    before({ tool: "bash", sessionID: "B", input: b });

    // A (healthy) is rewritten; B (unhealthy) is not.
    expect(a.command).toMatch(/\[pixel\] repository search/);
    expect(b.command).toBe("rg foo src");
  });

  test("a pixel shell call in one session does not mark another session as called", async () => {
    const m = mockCtx();
    await plugin.setup(m.ctx);
    const before = m.before();
    // A direct `pixel ...` shell call in session C marks C, not D.
    before({
      tool: "bash",
      sessionID: "C",
      input: { command: "pixel search-content foo" },
    });
    // Session D starts fresh: a repo search under enforce still rewrites
    // (D's health is unknown, so the hook probes and then classifies).
    const d = { command: "rg foo src" };
    expect(() => before({ tool: "bash", sessionID: "D", input: d })).not.toThrow();
    expect(d.command).toMatch(/\[pixel\] repository search/);
  });
});

// --- installed-binary checks (only when pixel + opencode are on PATH) -----
//
// Pixel checks below exercise the real CLI's footer behavior. OpenCode checks
// capture the installed version and exercise the plugin against the mock
// adapter above; they do not load the plugin in a live session.

const REAL_PIXEL = (() => {
  const r = spawnSync("sh", ["-c", "command -v pixel"], { encoding: "utf8" });
  return r.status === 0 ? r.stdout.trim() : null;
})();
const REAL_OPENCODE = (() => {
  const r = spawnSync("sh", ["-c", "command -v opencode"], { encoding: "utf8" });
  return r.status === 0 ? r.stdout.trim() : null;
})();
const REPO_ROOT = resolve(import.meta.dir, "..");

describe("real binary: pixel footer contract", () => {
  beforeAll(() => {
    if (!REAL_PIXEL) return; // suite no-ops when pixel is absent
    // Point the plugin at the real binary and clear any opt-out.
    process.env.PIXEL_BIN = REAL_PIXEL;
    delete process.env.PIXEL_METRICS;
  });

  test("search-content --json keeps stdout clean (JSON) and the footer on stderr", () => {
    if (!REAL_PIXEL) return; // bun:test counts a no-op return as pass
    const args = commandFor("search_content", { query: "opencode_config" }, [], "opencode_config");
    const { stdout, box } = _internals.runSync(REPO_ROOT, args);
    // stdout is pure JSON (one object per line), never the footer.
    expect(stdout.trim().startsWith("{")).toBe(true);
    expect(stdout.includes("🟩")).toBe(false);
    // The footer is the exact stderr box, one per this invocation.
    expect(box.startsWith("🟩 pixel")).toBe(true);
    expect(box.includes("❀")).toBe(true);
    // Unique per-invocation op #id (not a global latest): runs differ.
    const ids = new Set();
    for (let i = 0; i < 2; i++) {
      const { box: b } = _internals.runSync(REPO_ROOT, args);
      const m = b.match(/#([0-9a-f]{6})/);
      if (m) ids.add(m[1]);
    }
    expect(ids.size).toBe(2);
  });

  test("PIXEL_METRICS=0 suppresses the footer (no synthesis, no global latest)", () => {
    if (!REAL_PIXEL) return;
    process.env.PIXEL_METRICS = "0";
    try {
      const args = commandFor("search_content", { query: "opencode_config" }, [], "opencode_config");
      const { box } = _internals.runSync(REPO_ROOT, args);
      expect(box).toBe("");
    } finally {
      delete process.env.PIXEL_METRICS;
    }
  });

  test("quiet probes (--metrics off) never emit a footer", () => {
    if (!REAL_PIXEL) return;
    const { box } = _internals.runSync(REPO_ROOT, ["status", "--json"], true);
    expect(box).toBe("");
  });

  test("the pixel tool surfaces exactly one footer per invocation", async () => {
    if (!REAL_PIXEL) return;
    const m = mockCtx();
    await plugin.setup(m.ctx);
    const tool = m.addedTool();
    const res = await tool.execute(
      { action: "search_content", query: "opencode_config" },
      { sessionID: "real1" },
    );
    const footers = res.content.filter((c) => c.type === "text" && c.text.includes("🟩 pixel"));
    expect(footers.length).toBe(1);
    expect(footers[0].text.includes("❀")).toBe(true);
  });
});

describe("installed OpenCode version and simulated adapter contract", () => {
  test("the installed opencode is V2 and the asset exposes its current adapter shape", () => {
    if (!REAL_OPENCODE) return;
    // opencode --version reports v2.x.
    const v = spawnSync(REAL_OPENCODE, ["--version"], { encoding: "utf8" });
    expect(v.status).toBe(0);
    expect(v.stdout.trim()).toMatch(/^opencode v2\./);
    // These assertions describe the asset, not a live plugin load.
    expect(plugin.id).toBe("pixel");
    expect(typeof plugin.setup).toBe("function");
    // The mock captures the asset's calls; a real load remains necessary to
    // verify OpenCode compatibility.
  });

  test("setup registers the pixel tool and hooks expected by the mock adapter", async () => {
    if (!REAL_OPENCODE) return;
    const m = mockCtx();
    const cleanup = await plugin.setup(m.ctx);
    expect(m.calls.transform).toBe(1);
    expect(m.calls.before).toBe(1);
    expect(m.calls.after).toBe(1);
    expect(typeof cleanup).toBe("function");
    cleanup();
  });

  test("the simulated before-hook input shape is rewritten", async () => {
    if (!REAL_OPENCODE) return;
    // The V2 event shape is {tool, sessionID, agent, messageID, id, input};
    // This fixture mimics the current asset's assumptions; it is not an
    // independently verified OpenCode SDK payload.
    const m = mockCtx();
    await plugin.setup(m.ctx);
    const before = m.before();
    const event = { tool: "bash", sessionID: "s", agent: "a", messageID: "m", id: "c", input: { command: "rg foo src" } };
    before(event);
    expect(event.tool).toBe("bash"); // readonly fields untouched
    expect(event.sessionID).toBe("s");
    expect(event.input.command).toMatch(/\[pixel\] repository search/);
  });

  test("the simulated after-hook result gets a text content item", async () => {
    if (!REAL_OPENCODE) return;
    const m = mockCtx();
    await plugin.setup(m.ctx);
    const after = m.after();
    const event = {
      tool: "edit",
      status: "completed",
      sessionID: "s",
      agent: "a",
      messageID: "m",
      id: "c",
      input: {},
      result: { content: [{ type: "text", text: "edited" }] },
    };
    after(event);
    // The content is exercised locally; runtime compatibility is unverified.
    const appended = event.result.content.find((c) => c.type === "text" && c.text.includes("PIXEL BLAST RADIUS"));
    expect(appended).toBeDefined();
    expect(typeof appended.text).toBe("string");
  });
});
