# Candidate anchor hover failure — 2026-10-26

Hosted ARM SDK run 40406733431 on 5c7dcbc compiled the candidate anchor but the
private synthetic interaction probe failed: hover did not expose the popup.
The retained AT-SPI tree showed the Usage button but no popup. The original
receipt did not record pointer position or read count, so it does not establish
why hover failed. A follow-up uses the popup's mapped state for open checks and
adds synthetic pointer/read diagnostics. It requires a new native test.
