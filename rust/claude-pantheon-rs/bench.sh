#!/usr/bin/env bash
# bench.sh NAME [SECONDS] [LABEL]
#
# Sums RSS over every process whose command line matches NAME (a pgrep -f regex; anchor it with ^/path to
# avoid matching shells that mention the name) plus their descendants, and
# averages CPU% over SECONDS (default 20) from /proc/<pid>/stat deltas. Prints one Markdown table row:
#   | label | processes | RSS avg MB | RSS peak MB | CPU % | note |
# Examples:
#   ./bench.sh '^/home/me/.cache/cargo-target/release/claude-pantheon-rs' 20 "Rust client"
#   ./bench.sh '^/usr/lib/claude-desktop/claude-desktop' 20 "Electron"
#   ./bench.sh '^/home/me/.local/share/t3code/home/runtime' 20 "T3 server"
set -uo pipefail

name=${1:?process name or cmdline substring}
secs=${2:-20}
label=${3:-$name}
clk=$(getconf CLK_TCK)
ncpu=$(nproc)

tree() {
    local roots pid
    roots=$(pgrep -f -- "$name" | grep -vx "$$" | grep -vx "$BASHPID")
    # Drop this script and its subshells from the match.
    for pid in $roots; do
        case "$(cat "/proc/$pid/cmdline" 2>/dev/null | tr '\0' ' ')" in *bench.sh*|"") continue;; esac
        echo "$pid"
        descendants "$pid"
    done | sort -un
}

descendants() {
    local child
    for child in $(pgrep -P "$1" 2>/dev/null); do
        echo "$child"
        descendants "$child"
    done
}

# "ticks rss_kB count" over the tree.
sample() {
    local ticks=0 rss=0 n=0 pid rest pages
    for pid in $(tree); do
        rest=$(sed 's/^[^)]*) //' "/proc/$pid/stat" 2>/dev/null) || continue
        set -- $rest
        ticks=$((ticks + ${12} + ${13}))
        pages=$(awk '{print $2}' "/proc/$pid/statm" 2>/dev/null || echo 0)
        rss=$((rss + pages * 4))
        n=$((n + 1))
    done
    echo "$ticks $rss $n"
}

read -r t0 r0 n0 < <(sample)
if (( n0 == 0 )); then
    echo "| $label | 0 | – | – | – | no process matched \"$name\" |"
    exit 0
fi
peak=$r0; sum_rss=$r0; samples=1
start=$(date +%s.%N)
for ((i = 0; i < secs; i++)); do
    sleep 1
    read -r _ r _ < <(sample)
    sum_rss=$((sum_rss + r)); samples=$((samples + 1))
    (( r > peak )) && peak=$r
done
read -r t1 _ n1 < <(sample)
end=$(date +%s.%N)
elapsed=$(awk -v a="$start" -v b="$end" 'BEGIN{print b-a}')
cpu=$(awk -v d=$((t1 - t0)) -v clk="$clk" -v e="$elapsed" 'BEGIN{printf "%.1f", d / clk / e * 100}')
avg_mb=$(awk -v s="$sum_rss" -v n="$samples" 'BEGIN{printf "%.0f", s / n / 1024}')
peak_mb=$(awk -v p="$peak" 'BEGIN{printf "%.0f", p / 1024}')
echo "| $label | $n1 | $avg_mb | $peak_mb | $cpu | ${secs} s on $ncpu cores |"
