# Spike: T3 Code server as the backend (2026-10-21)

Question: should the native Pantheon client talk to the T3 Code server instead of the claude CLI directly?
Answer: yes. The server is cheap, headless, and its client protocol is plain JSON over a WebSocket.

## What T3 Code is
MIT, TypeScript + Effect, 29k stars, pushed daily. "Agent harness control surface": one local server
drives Claude Code (via the Claude Agent SDK), Codex, Cursor, Grok Build, OpenCode, Antigravity and Pi.
Its web, Electron and mobile clients are React; there is no native UI to port. The server is the asset.

## Install on this machine
- `npm install t3` into `~/.local/share/t3code` (7 KB bootstrap) which downloads the signed
  `t3-0.1.1-linux-arm64.tar.gz` (86 MB, SHA256-verified) into `T3CODE_HOME=~/.local/share/t3code/home`
  (286 MB on disk).
- Run headless: `T3CODE_HOME=~/.local/share/t3code/home t3 serve --host 127.0.0.1 --port 3773 --auto-bootstrap-project-from-cwd <project>`.
  It prints a pairing token and QR. `t3 pair` mints new tokens. `t3 service install` makes it a user service.

## Idle footprint, measured over 20 s
| Process tree | Processes | RSS | CPU |
|---|---|---|---|
| Electron claude-desktop (BASELINE.md) | 19 | 2.4 GB | 1.08 cores |
| T3 server, headless, one project | 2 | 395 MB (node 383 + resource-monitor 4) | 0.015 cores |

A 60 MB GTK client on top of the server is ~450 MB and ~0 CPU: five times less RAM and seventy times
less CPU than Electron, with every provider instead of one.

## Client protocol
- WebSocket at `http://127.0.0.1:3773/ws?<ORCHESTRATION_PROTOCOL_QUERY_PARAM>=<ORCHESTRATION_PROTOCOL_VERSION>`
  (constants in `packages/contracts/src/environment.ts`); a stale version gets a JSON error telling the client to update.
- Effect RPC with `RpcSerialization.layerJson` (`apps/server/src/ws.ts`): JSON envelopes, request / chunk / exit
  messages, streaming subscriptions. Message shapes live in the vendored
  `.repos/effect-smol/packages/effect/src/rpc/{RpcMessage,RpcSerialization,RpcClient}.ts`.
- 198 RPC methods in `packages/contracts/src/rpc.ts`: projects, threads, orchestration commands/events,
  provider instances, models, checkpoints/diffs, terminals, usage, keybindings, theme.
- Auth: pairing token exchanged for a scoped session (`docs/internals/environment-auth.md`); sockets
  authenticate at upgrade. Also a standard MCP endpoint at `/mcp` for outside agents (narrower).
- Subscriptions are per-thread, so a client viewing one thread does not pay for all history.

## Usage / token tracking
The server already aggregates token use, cache savings, model breakdown and API-equivalent cost across
Codex, Claude Code, Grok, OpenCode, Antigravity, Pi and Cursor (`docs/user/usage.md`), read from each
provider's local history. The native client can render this in the CodexBar style
(`~/Projects/mseries-codexbar-dense`) and feed a Wingpanel indicator.

## Risks
- Protocol version is pinned; the project moves fast (orchestration-v2). Mitigation: read the version
  from the contracts package at build time, show the server's update message when it mismatches.
- 383 MB for an idle node server is heavy; `--max-old-space-size` and the service's restart policy are
  the knobs. Still a fraction of Electron.
- One more background process (user service) instead of none.

## Decision
Build the Rust GTK app as a T3 client: native sidebar of projects and threads, provider and model
picker (Claude, Codex, OpenCode, Grok, …), streamed turns, permission prompts, diffs, a Usage page.
Keep the JSONL transcript viewer as offline fallback. Keep the Vala CLI-direct build as the
lightweight comparator.
