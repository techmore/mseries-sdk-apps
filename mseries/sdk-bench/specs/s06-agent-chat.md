# s06 Agent chat client (hard: streaming, rich content)

Golden reference: `rust/claude-pantheon-rs`. Backend contract: its `docs/T3-SPIKE.md` and `contracts/`.

A chat window against a local WebSocket server. Sidebar of threads with relative times; a transcript
rendering Markdown with syntax-highlighted code blocks; a multi-line composer (Enter sends, Shift+Enter
newline); streaming replies; permission requests shown as inline cards (Allow once, Allow for this
thread, Deny).

Capabilities: network (127.0.0.1 WebSocket), keyring (pairing token), notifications.

Acceptance (a scripted fake server is supplied):
1. Connects, lists threads, opens one, and shows history.
2. A streamed reply appears incrementally without losing scroll position when the user has scrolled up.
3. Code blocks are highlighted and have a copy button with a toast.
4. A permission request blocks the turn until answered; the answer is sent and the card shows the outcome.
5. Dropped connection shows a banner and reconnects with back-off; the composer draft is kept.
6. The pairing token is read from and written to the keyring, never to disk.
