# Signal and Messages: INCOMPLETE — DEFERRED

Owner decision: 2026-10-22. This is the current status on dev and takes precedence
over historical implementation notes, test receipts and installation reports.

| App/workstream | Status | Meaning |
| --- | --- | --- |
| Signal Pantheon integration | **INCOMPLETE — DEFERRED** | Official Signal may remain installed, but the requested integration and user experience are not accepted or finished. No native Rust Signal client is claimed. |
| Messages app build-out / Rust Messages | **INCOMPLETE — DEFERRED** | The new client and combined messaging experience are not accepted or finished. Existing fallback behavior does not qualify the new app. |
| Messages & Signal tab launcher | **INCOMPLETE — DEFERRED** | The owner rejected the delivered experience. A launcher opening separate windows does not establish a polished integrated experience. |

## Instructions for the next agent

- Do not resume this app build-out, promote a preview, replace a default client,
  or mark either app complete without a new explicit owner request.
- Installed, compiled and fixture-tested are distinct from loaded, reviewed and
  accepted. Do not infer completion from green checks or old handoff entries.
- Preserve existing clients, accounts, data and rollback paths. Deferral is not
  authorization to uninstall apps or remove owner data.
- On 2026-10-23, the owner explicitly authorized merging PR #526 into dev. The
  merge makes source available; it does not accept or complete the Signal/Messages
  experience. Inspect current source and runtime evidence before further work.
- Begin with the actual visible outcome and early running example required by
  [APP-BUILDING-GUIDE.md](APP-BUILDING-GUIDE.md).

## Work remaining before completion can be claimed

Revisit the rejected integration design and qualify the actual desktop experience;
complete live client/paired-phone behavior and phone identity checks; verify
screenshots and Signal attachment composition after the appropriate code is
loaded; finish the applicable parity, accessibility, localization and release
gates. Existing receipts are scoped evidence, not blanket app acceptance.
