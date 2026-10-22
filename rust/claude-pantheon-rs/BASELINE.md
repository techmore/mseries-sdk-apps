# Electron claude-desktop baseline (2026-10-21 19:38, idle with the Code tab open)

Machine: elementary OS on Apple Silicon, 10 cores, 30 GB RAM, aarch64.
Sampled four times over 20 s with `ps -eo rss,pcpu,comm`, plus a live 10 s `/proc/<pid>/stat` delta over the whole process tree.

| Metric | claude-desktop (Electron) |
|---|---|
| Processes | 19 |
| RSS, sum of tree | 2.33 – 2.39 GB |
| CPU, live over 10 s | 1.08 cores busy (108%) |
| Largest single process | GPU process, ~49% of a core, 140 MB |
| Renderer | 0.9 GB |

Targets for the native client: under 120 MB RSS, ~0% CPU idle, under 300 ms startup.
Reproduce with `bench.sh claude-desktop` once that script exists.
