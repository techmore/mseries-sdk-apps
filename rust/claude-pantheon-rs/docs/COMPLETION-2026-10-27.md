# T3 client completion matrix (2026-10-27)

Scope: `rust/claude-pantheon-rs` (GTK4 + Granite client of the T3 Code server) at `origin/main` a2a2c30.
Method: docs (README, INTEGRATIONS, WORKFLOW, PLUGINS, T3-SPIKE), OS-repo ledger (`docs/capabilities.json` entry 60,
ROADMAP 2026-10-26 sections), OS draft PR #521, source grep, test/build runs, read-only probes of the live service.
No tokens were minted or printed, no prompts sent, no provider quota spent.

Status: **Done** = code + automated evidence here; **Done-unverified** = code exists, no automated or live evidence in
this audit; **Partial**; **Missing**. Effort: S ≤ half a day, M ≤ 2 days, L more.

## Health and compatibility

| Check | Result |
|---|---|
| `cargo test -p claude-pantheon-rs` | 87 lib tests pass; render integration test self-skips outside the sandbox |
| Isolated GUI test `tests/run-render-sandbox.sh` (Xvfb + private D-Bus) | PASS (live parts skipped: "not connected to a T3 server") |
| `cargo clippy --all-targets -- -D warnings` | **FAILS**: `new_without_default` on `plugin_accounts::Accounts::new` (one error) |
| CI for this crate | **None** in mseries-sdk-apps (`msdk.yml` only mentions it); nothing guards fmt/clippy/tests |
| Live service | `t3code.service` active, 0.1.3, `GET /` 200, `/.well-known/t3/environment` reports `serverVersion 0.1.3`, `orchestrationProtocolVersion 2` |
| Protocol the client targets | `contracts/PROTOCOL_VERSION` = 2, 208 RPC tags; `scripts/refresh-contracts.sh --check` against pingdotgg/t3code `d1c4b52b7c` (today): **no drift** |
| Upstream | pingdotgg/t3code latest release v0.1.3 (matches installed); main is 85 commits past it. Fork techmore/t3code is 8 ahead (mseries docs/scripts only, no contract change), 31 behind |
| Installed binary | `~/.local/bin/claude-pantheon-rs` and `-sdk` built 2026-10-27 07:29, after the last crate commit (07:19): current, hash not compared |
| Wingpanel | `libwingpanel-dev` installed; `libmseries-t3.so` (OS repo `mseries/t3-wingpanel`) present in the indicators dir. The crate's own Vala `wingpanel-indicator/` is the legacy one |
| Bug found | `claude-pantheon-rs --version` (or any unknown flag) starts the GUI instead of printing/failing; the probe hung until killed |

## Matrix

| # | Capability | Status | Evidence | Effort |
|---|---|---|---|---|
| 1 | Pairing, credential in keyring, WS ticket, 426 message | Done | `t3/auth.rs`, `t3/ws.rs`, README; used by the live service | |
| 2 | Effect-RPC client without tokio | Done | `t3/rpc.rs`, unit tests, contracts gate | |
| 3 | Contract/drift gate (`build.rs`, `refresh-contracts.sh`) | Done | check run today: no drift; no scheduled CI | S (CI) |
| 4 | Chat: streaming turns, markdown, code blocks, tool rows, diffs | Done-unverified | `chat.rs`, `render.rs`, render test; live send not run (quota) | |
| 5 | Composer: attachments, `@` file picker, drafts, voice | Done-unverified | `composer.rs`, `voice.rs` | |
| 6 | Permission prompts + Y/N, Stop/interrupt | Done-unverified | `chat.rs`, `app.rs`; ledger says live coverage incomplete | S (live check, owner) |
| 7 | Sidebar: projects, pin, rename, archive, unread dot, order | Done | `sidebar.rs`, render test | |
| 8 | Offline transcripts (Claude Code JSONL) | Done | `transcript.rs`, `index.rs`, fixtures | |
| 9 | Usage page, plan-window gauges, rotation decision | Done | `usage.rs`, `quota*.rs`, tests | |
| 10 | Dock/panel status feed (`status.json`) and unified panel | Done-unverified | `status.rs`; OS `t3-wingpanel` loaded; ROADMAP 2026-10-26 | |
| 11 | Repos/issues runner, webhook task, per-run rotation | Done-unverified | `workflow.rs`, `repos.rs`; webhook needs a reachable endpoint | |
| 12 | Hand-off to repo, Activity recap, Repo monitor, Fleet (SSH) | Done | `handoff.rs`, `activity*.rs`, `repo_monitor*.rs`, `fleet.rs`, tests | |
| 13 | Changes page (diff per file, restore to checkpoint) | Partial | `changes.rs`; no branch compare, no `pullRequests.*` | M |
| 14 | Terminal pane (VTE over `terminal.attach`) | Done-unverified | `terminal.rs` (INTEGRATIONS table "Done") | |
| 15 | Visual replies (WebKitGTK on demand) | Done-unverified | `webview.rs`; INTEGRATIONS table still says "Not built" (stale) | S (docs) |
| 16 | SnapShot (portal screenshot into draft) | Done-unverified | `snapshot.rs`, Ctrl+Shift+S; table says "Not built" (stale) | S (docs) |
| 17 | Environments switcher (Direct/Tailscale/SSH/Connect) | Done-unverified | `environments.rs`; table says "Not built" (stale) | S (docs) |
| 18 | Diagnostics (resource telemetry) | Done | `diagnostics.rs` | |
| 19 | Plugin registry (accounts, shiplog, repos, usage) | Done-unverified | `plugin*.rs`, PLUGINS.md, commit a2a2c30 | |
| 20 | SDK build `claude-pantheon-sdk` + gates | Done-unverified | `sdk/`, `sdk_*.rs`; issue #596 gates (visual, keyboard, owner acceptance) open | L (owner) |
| 21 | Command palette, text zoom, window state, a11y names | Done | `palette.rs`, `winstate.rs`, README | |
| 22 | Notifications | Done-unverified | `notify.rs` (22 lines) | S |
| 23 | New thread in a worktree | Done (PR #26, unit-tested, not live-launched) | PR #26 | M |
| 24 | Older-history paging (cursor endpoint) | Done (PR #25) | PR #521 `t3/history.rs`; main loads the snapshot only | M |
| 25 | Settled-thread cache + resume with `afterSequence` | Design proposed (DESIGN-thread-cache.md), awaiting owner | PR #521 `thread_cache.rs` (LZ4, atomic projection+cursor); main reloads whole threads on reconnect | M-L |
| 26 | Reconnect keeps scroll position | Done (PR #24) | PR #521 `fdd006ec` | S |
| 27 | Provider status page (PR #27); sign-in and ACP registry | Partial: read-only page done, manager deferred (L) | PR #521 `provider_setup.rs`; `acp` absent in main | M |
| 28 | Pull-request status/checks (read-only, PR #28); PR actions | Partial | INTEGRATIONS 4c | M |
| 29 | Reset credits consume (Codex) | Partial | parsed in `model.rs`, no action | S |
| 30 | Keybindings / theme sync with server | Missing | no hit | S, low value |
| 31 | `--version` / `--help` | Done (PR #23) | see above | S |
| 32 | Clippy clean + crate CI (fmt, clippy, test, Xvfb render) | Done (PR #23) | `.github/workflows/claude-pantheon-rs.yml` | S |
| 33 | Dock quick-popover (T3-R) | Done-unverified, OS repo | #521 `TaskbarT3RPopover.vala`; superseded by the unified panel/dock menu (#607/#608) | OS repo |
| 34 | Docs/ledger truth (INTEGRATIONS table, README test count, capabilities.json) | Partial | table stale, README says 77 tests, now 87 | S |
| 35 | Packaging: user-level install | Done | `install.sh`, `build-deb.sh`; `.deb` epoch rule: check `build-deb.sh` before any package | S |
| 36 | Libwingpanel / Wingpanel module loaded | Needs owner | module install is sudo; panel loads it at next login | owner |

## PR #521 (OS repo, draft since 2026-10-22): what to salvage

The branch is a Vala/dock-heavy super-PR on the frozen OS copy (100 commits, 70 files). Dock popover and Wingpanel parts are
superseded by the unified panel (#607/#608). Worth porting into this repo as clean small PRs, with its tests:
`t3/history.rs` (cursor paging + dedupe + prepend with kept scroll), `thread_cache.rs` + `persistence.rs` (cache and
`afterSequence` resume, synchronized gating of send/stop/approve), reconnect scroll restore, `provider_setup.rs` (ACP
registry and generic ACP form). Do not port: DeepSeek-specific shortcut text, branding rename, dock popover, package versions.
