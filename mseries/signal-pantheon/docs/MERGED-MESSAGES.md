# Merged Messages + Signal — findings and plan (2026-10-27)

## Why sending lagged and the chat jumped (both apps)
Signal Pantheon (Vala):
- A message only appeared after `signal-cli` finished the network send; nothing showed
  while it was in flight (no optimistic message).
- The composer was disabled for the whole send, so focus was lost and typing stopped.
- Every incoming message, edit and send called `render_history()`, which destroys and
  rebuilds every bubble, then scrolls from an idle callback: that is the "jump".
- Caption/time/preview text was 0.8–0.9em on top of the default size: too small.

Rust Messages (`rust/mseries-messages`): sends already show an outbox entry, but
`render_conversation()` still removes and rebuilds all children on each update and
restores the scroll position afterwards, so a height change produces a visible hop.

## Fixed in Signal Pantheon (branch `smooth-send-and-larger-text`)
- "Sending…" bubble appears instantly; composer is cleared and stays usable; on failure the
  text is put back. Only the Send button waits for the in-flight message (serial send keeps order).
- One new message at the end is appended in place (stamp included); the view follows only if
  you were already near the bottom. Edits, deletions, reactions, search and older history still redraw.
- Text 10–15% larger (bubbles 1.12em, captions/times 0.92em, previews 1.02em, composer 1.12em).

## Merged app plan
Base: the Rust app in `mseries-sdk-apps/rust/mseries-messages` (GTK 4, SDK, BlueFerry adapter,
outbox, pins, media previews). Add Signal as a second channel, not a second window:
1. Channel trait: `list_threads / history / send / mark_read / events`, implemented by
   `BlueFerryChannel` (existing D-Bus adapter) and `SignalChannel`.
2. `SignalChannel` talks to the same bundled `signal-cli` JSON-RPC socket the Vala client uses
   (`listAccounts, listContacts, listGroups, send, receive events, startLink/finishLink`) and
   reads/writes the existing encrypted `message-history.db` format (see `mseries-signal-import`).
   Official-client boundary note: signal-cli is unofficial; keep Signal Desktop as the fallback.
3. One conversation list with a channel badge (iMessage/SMS vs Signal), per-channel send state,
   unified search; Signal-only features (view-once, disappearing timers, calls) shown only on Signal threads.
4. Fix the shared rendering model once: diff-based list (append/replace by message id),
   never clear-and-rebuild; stick-to-bottom only when already at bottom; optimistic outbox for both channels.
5. Reading size: follow the SDK's reading-size setting; defaults ≥ 1.1em body, 0.92em metadata.
6. Port order: shared message list widget → SignalChannel read path (history, contacts) →
   send path → link flow → media/avatars (already reconstructed in Vala) → calls last.
