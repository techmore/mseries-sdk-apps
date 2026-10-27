# OS SDK Dock usage-monitor specification

Status: normative target for SDK implementation and T3 conformance; not a claim
that the module, host adapters, or qualification suite already exist.

## Purpose and reference

Distill the successful native CodexBar presentation and interactions into reusable
OS SDK Dock standards. Build T3's provider/account monitor to those standards.
This is not a request to embed a broken CodexBar executable or replace its native
presentation with another bespoke dashboard.

The reference is the native CodexBar implementation, inspected at `dev` revision
`659e17ba30f02309f4d72c4a129df220984b0abd`:

- [Indicator.vala](codexbar-wingpanel/Indicator.vala): provider-led quota cards,
  compact account rows, Quota/Sessions/Spend organization, bounded scrolling,
  and a shared freshness/action footer.
- [Presentation.vala](codexbar-wingpanel/Presentation.vala): readable values,
  account labels and native presentation conventions.
- [QuotaEta.vala](codexbar-wingpanel/QuotaEta.vala) and
  [RefreshPolicy.vala](codexbar-wingpanel/RefreshPolicy.vala): existing reset,
  stale-data and refresh concepts to qualify rather than blindly copy.
- [TaskbarCodexBar.vala](dock-taskbar/TaskbarCodexBar.vala): the existing status-area
  anchor and hover/click dismissal behavior.

Pin reference source and capture its native appearance in the qualification
receipt. A source reference is not proof its current backend works. Missing
`codexbar-linux`, incomplete provider preferences, stale snapshots and failed
quota reads are defects, not design conventions to standardize. Draft PR #599's
rejected replacement is not the visual baseline or an accepted implementation.

## 1. Surface ownership

- The module is a **status-area control**, not an application-launcher replacement.
- In Windows/taskbar mode it occupies the existing bottom-right status cluster,
  alongside the other utility controls. A newly pinned app icon does not satisfy
  this requirement.
- In elementary mode it uses the elementary desktop profile's corresponding
  status-control host. The SDK must document and resolve that adapter explicitly;
  application code must not guess between a floating Dock and a panel integration.
- The OS SDK owns placement, anchor identity, mode transitions, popup ownership,
  focus, dismissal and theme integration. The application supplies data and
  explicitly supported actions.
- Both adapters must expose one identifiable T3 usage control and the same
  provider/account information. An unrelated top-bar indicator cannot substitute
  for the Windows bottom-right control. Duplicate monitors must not be installed
  to conceal a missing adapter.

## 2. Native presentation standard

Preserve CodexBar's information hierarchy rather than merely using GTK widgets:

1. Provider identity and useful quota/token information lead the view.
2. Saved accounts remain distinguishable within their provider, including
   inactive accounts. Selection is an action/context, not a visibility filter.
3. Compact quota rows pair a value with its actual window/reset. Provider token
   totals are shown once at provider scope, not copied onto each account.
4. Freshness, refresh progress and utility actions share a restrained footer.
5. Detailed sessions/spending remain secondary. CodexBar's established native
   page organization may be reused; an arbitrary new tab/dashboard hierarchy is
   not an equivalent implementation.

Requirements:

- Use the existing native GTK/Granite styling, shared OS sizing/text tokens,
  provider marks and native controls. Honor light/dark and larger-text settings.
- Keep labels, spacing, alignment, account differentiation and value formatting
  consistent with the reference. Format large token counts readably; make exact
  counts available without requiring users to interpret an unformatted digit wall.
- Do not repeat source disclaimers, timestamps, raw instance IDs or diagnostic
  prose inside every card. Required provenance must remain discoverable through
  the shared footer, accessible details or secondary view.
- Primary provider coverage and account access must be obvious on opening. Do
  not bury account discovery below a long stack of diagnostic cards.
- Bound scrolling to variable content, avoid nested scrolling where possible,
  and keep primary controls reachable without clipping or horizontal scrolling.
- Unavailable providers must not displace all useful observations above the fold.
  Preserve user ordering and show partial/unavailable coverage explicitly.

Owner visual approval is required. Compilation, native toolkit use and automated
checks cannot certify that a replacement matches the reference.

## 3. SDK module contract

The SDK must expose a discoverable, documented usage-monitor module with:

| Responsibility | Owner / required capability |
| --- | --- |
| Status host | SDK adapters for Windows/taskbar and elementary profiles |
| Presentation | Shared native provider, account, quota and token primitives |
| Lifecycle | Open/close, hover/pin, keyboard, refresh, cancellation and disposal |
| Data binding | Versioned snapshot and action contracts; explicit source ownership |
| Provider access | Supported provider/client adapters, not renderer-owned auth logic |
| Discovery | Module description, schema, supported capabilities and a runnable example |
| Qualification | Semantic fixtures, real-host interaction checks and visual reference evidence |

The names above describe required capabilities, not existing SDK APIs or CLI
commands. Do not instruct an LLM to invoke an unimplemented module or generator.
The implemented module's actual identifiers, imports, services, schema and
unsupported capabilities must be published in SDK discovery and examples before
claiming it can be requested from the SDK.

An application must be able to bind its supported observations and actions to the
module without reimplementing provider-card rendering or host placement. Native
T3 and SDK-hosted T3 must use the same presentation/data semantics; separately
compiled executables do not establish parity by themselves.

## 4. Observation and identity contract

Every observation must carry enough information to distinguish:

| Field group | Required meaning |
| --- | --- |
| Provider | Stable provider key and human-readable label |
| Account | Opaque stable account key, display label and active/selected state |
| Source | Local client, paired server or provider API; server identity when relevant |
| Scope | Account or provider aggregate; never implied by the selected account |
| Measurement | Typed value and units, nullable when not observed |
| Window | Observation period or quota-window identity and optional reset timestamp |
| Freshness | Source observation time, state and applicable age/reset validity |
| Coverage | Complete/partial status and unavailable or unsupported portions |

Rules:

- Include every configured/authorized account promised by the integration across
  Codex, Claude, Grok, OpenCode and other supported providers. Never silently
  collapse distinct accounts of the same provider.
- Distinguish server instances from computer-local saved identities. Do not merge
  them by display name or presume a remote workspace/account is client-local.
- Provider-wide historical tokens are not per-account usage or remaining quota.
  Do not attribute them to the selected account or double-count overlapping local
  and server observations.
- Input, output, cached input and reasoning retain their backend definitions.
  Reasoning already contained in output must not be added again to total tokens.
- Quota fractions, percent used and percent remaining are distinct units. A
  conversion requires an explicit source contract; an absent value is not zero.
- Loading, empty, unknown, stale, unavailable, failed and unsupported are distinct
  states. A genuine observed zero is allowed; a defaulted or failed read is not.
- Never present an expired window as live capacity. A past reset requires a new
  observation, not `0 hours`, a countdown clamped to zero, or an invented reset.
- Response receipt time is not source observation time. Show source age honestly.
- Cost estimates must be identified as estimates with currency and pricing scope;
  API-equivalent estimates must not be described as actual billed charges.
- Credentials, tokens, raw auth records and private filesystem paths must never
  enter the public presentation contract or diagnostic receipts.

## 5. Interaction, lifecycle and authority

- Hover opens the native status view without stealing focus. Pointer travel from
  anchor to popup must not close it prematurely.
- Clicking pins/activates the view. Test hover-to-click transitions, not only a
  direct programmatic open. Outside click and Escape dismiss correctly.
- Keyboard users can reach, identify and activate the status control, traverse its
  actions, dismiss it and recover focus. Use meaningful accessible names/roles.
- Mode changes, scale/text changes, display changes, app relaunch and host restart
  must not duplicate controls or strand popups.
- Refresh is asynchronous, bounded, visibly busy and recoverable. Preserve
  explicitly stale last-known values after failure; do not silently erase them.
- Late responses cannot overwrite a new account/server context. Coalesce duplicate
  refresh requests and release monitors, timers and disposable processes on teardown.
- Closed or disabled optional monitors must not add polling. Reuse an existing
  supported collector/connection where its lifecycle is required; document that
  dependency rather than creating an accidental second daemon or T3 connection.
- Read-only monitoring must not switch accounts, log in, start a model turn, file
  issues or mutate Git. Any supported write action needs separate explicit consent.

## 6. T3 conformance requirements

T3 must be rebuilt using these standards, not treated as conformant because the
existing draft already shows token strings.

- Bind the actual status-area host in both desktop profiles to the shared SDK
  presentation. Identify the concrete host component in the evidence.
- Supply all supported accounts/providers through real adapters. Keep selected
  chat provider, saved account identity and provider aggregate scope separate.
- Preserve the native CodexBar hierarchy and account readability. Do not use the
  rejected generic cards/tabs as a shortcut to claim SDK reuse.
- Restore or replace any broken backend through a supported adapter. Missing
  executables/configuration/permissions remain visible failures until repaired.
- A missing provider in the actual dropdown is a failed acceptance criterion even
  if its data exists in a file, another window, another icon or another process.
- Installation includes the required assets, executable/helper dependencies and
  activation metadata. Verify source, installed artifact and loaded host identity.
- Uninstall/reinstall owns its controls and artifacts, preserves unrelated apps
  and user accounts, and does not resurrect retired monitors or duplicate pins.
- Native and SDK-hosted implementations must demonstrate equivalent observation,
  state and interaction semantics before claiming SDK parity.

## 7. Required verification and acceptance evidence

The implementation must provide a repeatable conformance harness. Until that
harness exists, record the exact existing commands and host actions used; do not
invent a passing SDK gate or claim these checks are already implemented.

| Check | Pass condition |
| --- | --- |
| Multi-account fixture | At least two Codex accounts, two Claude accounts, one Grok and one OpenCode account retain distinct identities, differing nonzero observations and unknown values through the actual rendered control |
| Provider coverage | Every supported fixture provider and account is reachable; changing selection does not hide the others |
| Semantics | Exact token arithmetic, explicit aggregate scope, correct used/remaining conversion, no unknown-to-zero conversion or duplicate counting |
| Failure transitions | Missing executable, permission denial, malformed/missing values, stale source, expired reset, offline and recovery produce the specified states |
| Races | Switching context during refresh cannot display the prior context's late response |
| Real adapters | At least one real read per configured provider, or an explicit provider-specific blocker; record returned coverage without publishing credentials |
| Exact host interaction | Open the Windows bottom-right control and the elementary profile's corresponding control; verify hover, hover-to-click pinning, outside click, Escape and keyboard focus on those controls |
| Mode lifecycle | Switch profiles in both directions and restart/relaunch; one correct control remains and observations retain correct scope |
| Visual matrix | Actual host screenshots in both profiles, light/dark, default/larger text and supported scales; no clipped actions or content and reference reviewed by the owner |
| Disabled lifecycle | Closing/disabling the feature introduces no additional periodic provider reads; teardown releases its resources |
| Installation | Loaded executable/module/assets match the recorded source and installed build; fresh install and reinstall preserve host ownership |
| SDK authoring | A named moderate-capability model follows the pinned SDK guidance/example to build the T3 integration without expert UI reconstruction or private undocumented APIs; record model, prompt, commands, first/final results, repairs and human interventions |

A generic GtkButton wrapping a view, a fixture-only window, parsed JSON, a pinned
launcher, a binary hash or a successful build may support a check but cannot
replace actual host interaction and visual approval.

Evidence must state **implemented, tested, packaged, installed, loaded and
owner-accepted** separately. An owner's report of missing/wrong/poor presentation
invalidates the corresponding acceptance claim; do not respond by repeating the
same lower-level checks or closing the issue.

## 8. Mandatory authoring guidance

SDK guidance and the reusable app-building prompt must direct agents to:

- Identify the requested desktop surface before editing: status control, app
  launcher, panel item and full application window are not interchangeable.
- Inspect the reference and module inventory first. Reuse the qualified native
  module and supported adapters instead of generating a bespoke replacement UI.
- Report a missing/broken module or backend as such. Documentation and a manifest
  do not constitute an implementation; missing functionality must be qualified.
- Verify the exact user-facing control early. Stop expanding scope when observed
  behavior contradicts the requested outcome; repair the mismatch first.
- Keep builds focused and preserve existing settings, identities and integrations.
  Do not change kernel, boot, audio, network configuration or account credentials
  as a shortcut for a monitoring UI task.
- Never claim the current specification repairs CodexBar, completes the SDK,
  validates T3's appearance or authorizes deployment. Those are implementation
  and acceptance outcomes, not documentation outcomes.
