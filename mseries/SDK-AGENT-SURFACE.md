# SDK agent surface: app intents, entitlements and consent (design, 2026-10-23)

**Status: design only. Nothing in this note is built or installed.** It joins two things that are already on `dev` but do not yet
know about each other: the SDK ([RUST-PANTHEON-SDK.md](RUST-PANTHEON-SDK.md), [SDK-INTROSPECTION-AND-STREAMING.md](SDK-INTROSPECTION-AND-STREAMING.md))
and the agent broker ([local-ai/PANTHEON-MCP-DESIGN.md](local-ai/PANTHEON-MCP-DESIGN.md)). Owner direction: this should work like Apple's
API permissions and scopes. An app says what it can do and what it needs, the system checks it, and the user (not the model)
grants access.

## Owner decision (2026-10-23)

**The SDK is where the MCP surface lives, and the desktop apps are rebuilt on the SDK so they get it.** An app's agent-facing tools are
no longer written by hand per app and per broker adapter: they are generated from its description. The Pantheon MCP design's broker
stays as the enforcement and consent point (grants, jobs, receipts, quiet admission), but for SDK apps its capability registry is
fed by the SDK instead of by hand-written adapters. Apps that are not SDK apps keep the adapter, AT-SPI and portal paths. How I read
"merge the MCP to the SDK": one source of truth in the description, not two parallel systems; correct me if you meant folding the
broker itself into the SDK runtime.

Consequences for the rebuild order: the SDK apps that already exist carry the first manifests (the T3 port, then the others built on
the SDK), and each desktop app that is rebuilt on the SDK ships with its intents, scopes and fixtures as part of "done", not as a later
integration.

## The idea in one paragraph

An SDK app already declares what it **needs** (`needs "network:localhost", "notifications", "filesystem:data"`) and already has typed
**messages** and **state**. Add two things to the description: **intents** (the actions an agent may ask the app to perform, typed
like Apple's App Intents) and **entities** (the objects an agent may refer to, like App Entities), each carrying a **scope** and a
**risk class**. `msdk check` validates them, `msdk agent-manifest` generates the MCP tool list and the broker's registry entry from
them, and the broker enforces the scopes against grants the user made. Because the manifest is generated from the same description
that builds the app, the agent surface cannot drift from what the app really does.

## Apple's model, mapped

| Apple | Here | Where it lives |
|---|---|---|
| Entitlements in the app bundle (what the app may touch) | `needs` in the `.msdk` description, extended with agent scopes | Description; checked by `msdk check` |
| App Intents (typed actions an app offers to Siri/Shortcuts) | `intent` declarations bound to existing `message`s | Description; generated into MCP tools |
| App Entities / queries (things an intent can name) | `entity` declarations over existing `type`s | Description |
| TCC prompts ("App X wants to access Y"), per app, revocable | Grants: per agent, per app, per scope, with expiry and revoke | Broker store; Settings page |
| Sandbox: the app cannot exceed its entitlements | The broker refuses any call outside a declared intent or an unexpired grant; the app runtime refuses a message an intent does not map to | Broker and SDK runtime |
| App Store review / notarisation | `msdk check` plus gates; a manifest hash recorded in the grant, so a changed manifest asks again | CI and broker |
| Privacy "nutrition labels" | The generated manifest is readable: what each intent reads, writes and sends | Settings page |

## 1. What an app declares

Illustrative syntax (the real grammar is decided with the `.msdk` text form; names here are proposals):

```
app "Notes" id org.techmore.Notes icon notes
needs "filesystem:data"

entity Note from Note { title, modified }          // the fields an agent may read; body is not listed, so it is not exposed
intent CreateNote(title: Text, body: Text) -> Note
  as CreateNote                                    // the existing message it triggers
  scope "notes.create"  risk reversible
  verify note exists with that title               // readback the broker runs after the call
intent FindNotes(query: Text) -> [Note]
  scope "notes.read"    risk read
intent EmailNote(note: Note, to: Text) -> Draft
  scope "notes.share"   risk external  confirm
```

Rules the checker enforces:

- An intent maps to exactly one existing message, and its parameters are the message's typed payload. An agent cannot invoke anything
  the app's own UI could not.
- Every intent names a **scope** and a **risk class**; every scope the app uses is listed once and described in a sentence the
  Settings page will show.
- **Risk classes** are the ones the MCP design already uses: `read`, `reversible` (local creation that can be undone),
  `external` (mail, invitations, anything leaving the machine) and `privileged` (system changes). `external` and `privileged`
  intents must say `confirm`: the broker shows the concrete final action to the user, never a generic "allow".
- Every `reversible` and `external` intent needs a `verify` readback, because the MCP design requires independently verifiable outcomes.
- Fields not listed in an `entity` are not readable by an agent. Password and secret fields are never listable (the SDK already masks
  them in the tree, the log and screenshots).
- An app that wants no agent access writes nothing; the default is **no intents, nothing exposed**. A lock screen can additionally
  refuse introspection, as the introspection note already allows.

## 2. What the SDK generates and checks

| Command | Output | Status |
|---|---|---|
| `msdk check` | Adds agent-surface rules: unmapped intents, missing scope or risk, `external` without `confirm`, write intents without `verify`, entities that expose a masked field | Not built |
| `msdk agent-manifest app.msdk` | A versioned JSON manifest: intents with JSON-Schema parameters, entities, scopes with their descriptions, risk classes, and a content hash | Not built |
| `msdk agent-manifest --mcp` | The MCP `tools/list` shape for the same intents, for clients that speak MCP directly | Not built |
| Gates | A fixture per intent: given this state, calling the intent with these arguments produces this state and this readback; plus a hostile-argument case (control characters, oversize text, path-like strings) that must be refused | Not built |
| `msdk render` / L1 tree | Unchanged. The agent surface is separate from the visible tree, so a screen change does not silently widen what an agent can do | Built (L1) |

The existing introspection API (L2/L3) is **not** the agent surface. It is the developer and accessibility view of any SDK app
(`tree`, `state`, `click`, `type`) and is read-only unless started with input enabled. Agents use intents, which are typed,
scoped and logged. L2 `click`/`type` stay a debugging and accessibility tool and are not granted to agents by default.

## 3. Grants and consent (the broker's half)

The broker is the enforcement point; neither the app nor the model grants anything.

- **A grant** is `(agent identity, app id, scope, expiry, manifest hash)`. Identity is the calling client the broker already
  distinguishes (local IPC user, a paired device, a headless agent). Role names confer nothing, as in the MCP design.
- **Prompting** works like TCC: the first time an agent calls an intent whose scope it lacks, the broker pauses the call and asks
  the owner, naming the agent, the app, the scope's sentence and, for `external` and `privileged`, the concrete action (recipient,
  subject, target). Choices: allow once, allow for a duration, always for this agent and scope, deny. Denial is remembered so an agent
  cannot nag.
- `read` and `reversible` scopes can be pre-granted in bulk for trusted local agents; `external` and `privileged` are always
  per-action unless the owner explicitly widens a named scope, and widening is logged.
- **Manifest hash:** a grant is bound to the manifest it was made against. An app update that adds or widens an intent invalidates
  the affected grants, so a new version cannot quietly gain access.
- **Model output and tool annotations cannot create or widen a grant.** Conversation and page text is data, never an instruction
  (the same rule as the Smart Reply design).
- **Revocation and expiry** are immediate. Receipts survive revocation.
- **Settings page** ("Privacy & Agents", in the style of the Work Mode panel): per agent, the apps and scopes it holds; per app,
  who holds what; the generated manifest in plain language; a log of every call (intent, outcome, never the content of entities);
  a global stop that ends pending agent work without touching the apps.

## 4. Where Smart Reply fits

Smart Reply ([SMART-REPLY-DESIGN.md](SMART-REPLY-DESIGN.md)) is the first user of this model for a different reason: it reads other
apps. Its chat marker (`X-MSeries-Chat`) becomes, for SDK apps, a declaration in the description (a `Transcript` plus a composer is a
chat by definition, as that design already says), and its allow/block rules become the **user-side** policy over the same scopes: the
app declares what it exposes, the owner's rules say which apps, contacts and field kinds are allowed. Apps that are not SDK apps
(Signal Desktop, Brave, Flatpaks) have no manifest, so they stay on the desktop-entry marker and the AT-SPI path with the
ask-first default.

## 5. What this does not claim

- No grammar, checker rule, manifest generator, broker enforcement or Settings page exists yet.
- It does not give agents a way around an app's own confirmation dialogs; `confirm` intents surface the broker's prompt, and an app
  may still add its own.
- Non-SDK apps (the majority) are outside the manifest model and need the separate adapter, AT-SPI and portal work in the MCP design.
- The permission UX and the exact scope vocabulary need the owner's review before any grammar is fixed.

## 6. Order

0. (Decided) SDK owns the agent surface; new and rebuilt desktop apps are SDK apps.

1. Agree the vocabulary here: `intent`, `entity`, `scope`, `risk`, `confirm`, `verify`, and the scope naming scheme.
2. `msdk check` rules and `msdk agent-manifest` against the T3 port (it already has typed messages and a permission flow:
   `AllowOnce`, `Deny`, `AllowOldest`), with fixtures per intent and the hostile-argument cases.
3. Broker grant store and enforcement with synthetic apps (MCP design phase 1: identities, scopes, denied scopes, two-agent
   concurrency, no automatic inference).
4. The Settings page, once the SDK's standard `settings` surface exists.
5. Wire Smart Reply's rules to read scopes from manifests for SDK apps.

## Open questions for the owner

- Scope naming: one level (`notes.create`) or hierarchical with inheritance?
- Should `reversible` intents ever prompt on first use, or only `external` and `privileged`?
- How long may "always" last before it asks again?
- Do you want an app-level "agents may never use this app" switch visible in the dock or the app's own settings?
- Should the manifest be shipped inside the package and also published in the repo for review (the Apple-notarisation analogue)?
