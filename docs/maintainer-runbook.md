# Maintainer runbook

What needs the maintainer's machine, accounts or decisions, with the exact
commands. Nothing here blocks a pull request; each item says what it unlocks.
Numbers from any run go into the pull request or issue with the command and the
run identity beside them (`.agents/rules/measuring.md`).

## 1. Repository settings (once)

| Setting | Where | Unlocks |
|---|---|---|
| GitHub Advanced Security (code scanning) or a public repository | Settings → Code security | `codeql.yml` (the job is skipped while the repository is private) |
| Homebrew tap | Not configured | `pixel.rb` is attached to each release as an asset |
| Branch protection on `main`: require `fmt, clippy, test`, `MSRV`, `cargo deny`, `SPDX and action pins`, `website and motion`; squash merge only | Settings → Branches | nothing merges red |

## 2. Host verification (each host, once per host version)

`specs/hosts.md` marks cells "to verify". For every host you have installed:

```sh
cargo install --path crates/pixel --locked
pixel setup --agent <claude|codex|devin|gemini|antigravity|opencode|pi> --yes
pixel doctor --only 'install.*'
scripts/verify/capture.sh on            # wraps the installed pixel; hooks leave their payloads
# in the host, in a scratch repository: a prompt that names a symbol; have the
# agent edit a file that has callers; let the turn finish
scripts/verify/capture.sh show          # event, stdin payload, stdout envelope, exit code
scripts/verify/capture.sh off
```

For each "to verify" cell of the host the capture must show the event name the
host sent, the payload fields `pixel-setup` reads, and, for an envelope, its effect
in the host (the model's next message cites the injected text; the 🟩 line is
drawn). Then edit `specs/hosts.md`: the cell becomes "measured" with the date and
host version, or the fallback is recorded. **Codex** also needs `/hooks` to review
the new entries before they fire. **pi**: verify the event names and the
`triggerTurn` continue semantics against the installed
`@earendil-works/pi-coding-agent`. The captures hold the typed prompt: read them
before sharing.

## 3. Acceptance run (the bar for v0.1.0)

Pixel's arena (read-only source at `e8fd304`) runs PIXEL through a shim called
`pixel`. Claude Code and Codex, three repetitions each, ripgrep at `3fce3b5`, the
13 questions of `single` (`docs/bench/2026-10-22-token-ab.md` in the Pixel
repository). Bar: tool calls and provider-reported tokens no worse than Pixel
0.9.0 (Claude 59 calls / 4.10M tokens, Codex 32 / 1.50M), blind answer grade no
worse (2.85 / 2.95).

```sh
cargo build --release -p pixel
git clone https://github.com/<you>/pixel pixel-src && git -C pixel-src switch --detach e8fd304
mkdir -p /tmp/pixel-shim && cat > /tmp/pixel-shim/pixel <<'SHIM'
#!/usr/bin/env bash
# Maps the commands Pixel's arena calls onto pixel, and the names it greps for.
pixel=${PIXEL_BIN:-/path/to/pixel/target/release/pixel}
export PIXEL_BRIEF=${PIXEL_BRIEF:-1}
case "${1:-}" in
  run-hook)                      # run-hook task-event --provider P --event prompt-submit [--part K/N]
    shift 2; provider=""; event=""; part=()
    while [ $# -gt 0 ]; do case "$1" in
      --provider) provider=$2; shift 2;; --event) event=$2; shift 2;;
      --part) part=(--part "$2"); shift 2;; *) shift;; esac; done
    "$pixel" hook "$event" --host "$provider" "${part[@]}" | sed 's/\[PIXEL:BRIEF\]/[PIXEL:BRIEF]/g' ;;
  prepare-repo) shift; "$pixel" prepare "$@" ;;
  build-index)  shift; "$pixel" prepare "$@" ;;       # build-index --history PATH
  *)            exec "$pixel" "$@" ;;
esac
SHIM
chmod +x /tmp/pixel-shim/pixel
cd pixel-src
S=.agents/skills/arena/arena.sh
for rep in 1 2 3; do
  $S run-set single --agent claude --pixel-bin /tmp/pixel-shim/pixel
  $S run-set single                --pixel-bin /tmp/pixel-shim/pixel    # Codex
done
scripts/bench/token-ab/tokens.py                                       # provider-reported tokens
```

The shim renames the brief tag back to `[PIXEL:BRIEF]` because the arena's
preflight and receipt parsers look for it; the hook output otherwise is PIXEL's.
Record, per agent: tool calls, provider-reported tokens, the blind grade of the
answers, the binary's sha256 and `pixel --version`, and the commands above. The
numbers replace Pixel's on the README and the website (`website/data/results.toml`)
only when they come from this run. `pixel brief --without <step>` is the control
for any step the run suggests is not removing calls; a step that does not is
deleted.

## 4. Edit-hooks A/B (decides the default of `edit_hooks`)

Edit hooks ship off. The default turns on only on a correctness or turn gain.
Three arms on an edit task set (a signature change with N callers, a removed
parameter, a renamed field), on Claude Code and Codex: vanilla
(`PIXEL_BRIEF=0 PIXEL_EDIT_HOOKS=0`), brief (`PIXEL_EDIT_HOOKS=0`), brief + edit
(`pixel config --global edit_hooks on`). Measure turns, tool calls, provider-reported
tokens, and whether `cargo build && cargo test` is green at the Stop. Before the
runs, replay real Claude and Codex edit payloads against a ripgrep clone with
`pixel hook pre-edit|post-edit|stop` and check the card names the right callers, the
post-edit message lists exactly the stale sites and the Stop block fires once.

## 4b. Semantic search quality (decides whether `semantic` ships)

The container that built `semantic` could not reach the model host, so only the
plumbing is tested. On your machine:

```sh
cargo build --release -p pixel --features semantic
pixel search-meaning --download            # prints repository, revision, destination first
cd <a repository you know> && pixel prepare
pixel search-meaning "where is the retry limit read from config" --limit 5
```

Judge ten questions you know the answer to: the right file in the first three,
tests and docs below code. Record the question list, the build commit and the
result beside the command (`measuring.md`). The tokenizer stack must also build
for the musl lanes (`cross-build.yml` with `--features semantic`) before the
release build turns the feature on.

## 5. Decisions

- **Domain** and `baseURL` of the website; Cloudflare Pages is the alternative to
  GitHub Pages, which needs a paid plan for a private repository.
- **Palette and type**: the site ships the indigo-and-orange pair of the launch
  image as a default (first block of `website/assets/css/main.css`; the token names
  keep their original spelling). Run the `section-redesign` artifact vote to confirm
  or replace it.
- **The site's publisher.** The Privacy Policy and Terms of Use say the site is
  published by "the maintainers of PIXEL"; a policy needs a named controller, so name
  one in both files (`website/content/legal/`).
- **Visit counting.** `cf_beacon` in `website/hugo.toml` is empty (no analytics). A
  token turns the Cloudflare beacon on; the Privacy Policy then needs its
  Cloudflare row and paragraph back (see `website/README.md`).
- **Making the repository public** (turns on CodeQL, the release verification
  commands and `install.sh`).

## Publishing the site

`pages.yml` builds the site on every push to `main` that touches it and keeps it as
the `site` artifact; nothing is deployed. After the decisions above:

1. Set `baseURL` in `website/hugo.toml` and publish the site on a domain chosen
   by the Pixel maintainers; no public site domain has been configured.
2. Add a deploy job to `pages.yml` that downloads `site` and publishes it with a
   pinned `actions/deploy-pages` (or Cloudflare Pages), then set the custom domain
   and DNS (four `A`, four `AAAA` and a `www` `CNAME`, "DNS only").
3. Re-render the animations only with the recipe in `.agents/rules/readme-webp.md`
   once the PIXEL compositions (`docs/motion`) exist.

## 6. Cutting v0.1.0

After sections 1 to 3: run the `release` skill (`.agents/skills/release/SKILL.md`).
It prepares `release-0.1.0`, waits for CI on the exact head, tags `v0.1.0`, and
`release.yml` builds, attests, publishes and smoke-tests. Verify with the commands
of `.github/SECURITY.md` ("Verifying a release").
