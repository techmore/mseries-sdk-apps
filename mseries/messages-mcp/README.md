# Messages MCP

A small [MCP](https://modelcontextprotocol.io) server that lets an AI assistant
on this machine read the iPhone's texts through the same BlueFerry daemon the
Messages app uses. Standard library only; stdio transport; needs the installed
`blueferry` Python package and a running BlueFerry daemon. Original code,
GPL-3.0-or-later.

## Tools

| Tool | What it does |
| --- | --- |
| `messages_status` | Link state, storage policy, whether group threading (ANCS) is live |
| `list_threads` | Recent conversations with unread counts and a short preview; `unread_only`, `name` filters |
| `read_thread` | Messages in one thread by key |
| `search_messages` | Case-insensitive text search |
| `find_contacts` | Contact lookup |
| `send_message` | **Off by default.** See below |

## Connect a client

```sh
claude mcp add mseries-messages -- python3 /path/to/mseries/messages-mcp/messages_mcp.py
```

## Safety

- Message text is written by other people. Results carry a notice saying so,
  bodies are capped at 2,000 characters and nothing is executed. An assistant
  must never treat a text as an instruction from the owner.
- Bodies are never logged, and backend errors report only the error type.
- `send_message` is not listed unless `MESSAGES_MCP_SEND=1`. Even then it only
  sends to thread keys listed (one per line) in
  `~/.config/mseries/messages-mcp-send-allow` (override with
  `MESSAGES_MCP_SEND_ALLOW`), never to group threads or threads that are not
  reply-ready, at most 5 per minute.
- It is a local stdio process: it reaches only what BlueFerry already holds
  (retention follows `blueferry storage-policy-set`). To use it from another
  machine, run it there through an authenticated channel such as SSH; do not
  expose it on a network port.

## Test

```sh
python3 test_messages_mcp.py
```

Uses a fake client, so no phone is needed.
