#!/usr/bin/env bash
# Shipped-asset + cold-start measurer — the ONE byte basis for this repo.
#
#   scripts/bench/assets.sh [--runs N] [--timeout S] [arm ...]
#
# With no arm arguments it measures the standard arm set (STANDARD_ARMS below:
# the Solid SPA dist, the Leptos WASM dist, the `x` CLI/TUI binary and the
# axum+htmx binary). An arm is either a web dist DIRECTORY (measured as a
# first-render closure) or a compiled BINARY (size + cold start + peak RSS).
# A missing artifact is reported as a `not built` row carrying the command
# that builds it, and never fails the run — one un-built arm must not hide
# the numbers for the others.
#
# BASIS — read this before quoting any number out of this output:
#
#   * A web arm is NOT the sum of its dist directory. It is the FIRST-RENDER
#     CLOSURE: the entry `index.html` plus every local `.js` / `.mjs` / `.css`
#     / `.wasm` that index.html references, plus the `.wasm` those JS files
#     reference (the wasm-bindgen glue names its module in the JS, not in the
#     HTML). Route-split / lazily imported chunks are deliberately NOT counted
#     — a chunk the first render never fetches is not first-render cost
#     (`docs/todo-state-management.md`: visiting `/` downloads neither the
#     xstate nor the effect route). The whole-directory total is reported too,
#     in its own clearly-labelled table, because it is a different basis.
#   * Every file is compressed INDIVIDUALLY, `gzip -9 -c` and
#     `brotli -q 11 -c`, because one response per file is what a static server
#     actually sends. Never a server's negotiated output (that tracks the
#     server's compression level, not the asset), never one archive of the
#     whole dist. Both columns are mandatory: a raw-vs-gzip-only comparison
#     flatters JS and penalises WASM, and every modern static server
#     negotiates brotli.
#   * Both compressors read STDIN (`<"$f"`), never a filename. `gzip -9 -c
#     FILE` stores the FNAME field in the member header and so reports
#     (strlen(name) + 1) bytes MORE than a server ever sends — measured on
#     this dist: 1230 vs 1219, 1559 vs 1533, 7515 vs 7479, 135077 vs 135036.
#     That is the whole class of error this harness exists to stop: a
#     hand-measured number wrong by a small, plausible-looking amount.
#   * A binary arm's bytes are NOT comparable to a web arm's: a binary is
#     installed once, not downloaded per render. The number that compares a
#     CLI/TUI to a browser app is the per-operation wire cost —
#     `scripts/bench/ops.sh`.
#   * A binary arm times ONE OR MORE named invocations, and the `invocation`
#     column says which. `--version` is NOT assumed to be a no-op: a
#     spec-driven CLI that builds its command tree from embedded documents
#     pays that cost before clap can answer, so the arm declares a note and,
#     where useful, a second invocation that does real work — which is how
#     the table separates "startup cost of the design" from "cost of doing
#     something".
#
# PLATFORM — peak RSS comes from `/usr/bin/time`, DETECTED not assumed: on
# Darwin it reports `maximum resident set size` in BYTES (`-l`), GNU time
# reports `Maximum resident set size (kbytes)` (`-v`). With neither, the RSS
# column reads `n/a` and the header says why. Wall-clock timing uses bash's
# $EPOCHREALTIME (bash >= 4.4, microsecond resolution); on older bash it falls
# back to perl Time::HiRes, which adds one fork per timestamp — the header
# records which timer produced the numbers.
#
# Invoked by `just bench-assets`.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

# kind|label|path|build command|invocations|arm note
#
# `invocations` (bin arms only) is a `;`-separated list of `args::note`. It
# defaults to `--version::`. Each entry becomes its own timed row, so an arm
# whose startup is NOT a no-op can show both what startup costs and what
# doing something costs. `arm note` qualifies the artifact itself and is
# carried into the cross-arm summary's basis cell.
STANDARD_ARMS=(
  "web|todo-web · Solid SPA|apps/todo/web/dist|cd apps/todo/web && bun run build"
  "web|todo-web-leptos · Leptos WASM|apps/todo/web-leptos/dist|bun nx build todo-web-leptos (= trunk build --release in apps/todo/web-leptos)"
  "bin|x · CLI+TUI|dist/target/release/x|cargo build --release -p x_cli|--version::NOT a no-op: the clap command tree is built from the three embedded OpenAPI documents before clap can answer, so this is the startup cost of being spec-driven;api list::same parse + tree build, plus output formatting — no network|ONE binary serving BOTH surfaces: the CLI and the \`x ui\` TUI subcommand. The TUI is not free and not separate — it is these same bytes."
  "bin|todo_web_htmx · axum SSR|dist/target/release/todo_web_htmx|cargo build --release -p todo_web_htmx"
)

RUNS=10       # timed invocations per binary arm invocation row
TIMEOUT=5     # seconds one invocation may take before it is killed
ARGS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --runs) RUNS="${2:?--runs needs a number}"; shift 2 ;;
    --timeout) TIMEOUT="${2:?--timeout needs seconds}"; shift 2 ;;
    -h | --help)
      sed -n '2,/^set -euo/p' "$0" | sed 's/^# \{0,1\}//; $d'
      exit 0
      ;;
    -*) echo "FATAL: unknown option $1 (see --help)" >&2; exit 2 ;;
    *) ARGS+=("$1"); shift ;;
  esac
done

# --- prerequisites ----------------------------------------------------------
need() { # need <bin> <why>
  command -v "$1" >/dev/null 2>&1 || {
    echo "FATAL: '$1' not found in PATH. $2" >&2
    exit 1
  }
}
need gzip "The gzip column is not optional; install gzip (macOS ships it, Debian: apt-get install gzip)."
need brotli "The brotli column is not optional — a raw-vs-gzip comparison flatters JS and penalises WASM, and every modern static server negotiates brotli. Install it: brew install brotli / apt-get install brotli."
need awk "Used for byte/percentage arithmetic."

GZIP_VERSION="$(gzip --version 2>&1 | awk 'NR==1')"
BROTLI_VERSION="$(brotli --version 2>&1 | awk 'NR==1')"

# --- host / timer detection -------------------------------------------------
case "$(uname -s)" in
  Darwin)
    CPU="$(sysctl -n machdep.cpu.brand_string 2>/dev/null || echo unknown-cpu)"
    CORES="$(sysctl -n hw.ncpu 2>/dev/null || echo '?')"
    OS_DESC="Darwin $(uname -r)$(sw_vers -productVersion 2>/dev/null | sed 's/^/ (macOS /; s/$/)/')"
    ;;
  Linux)
    CPU="$(awk -F': ' '/^model name/{print $2; exit}' /proc/cpuinfo 2>/dev/null || echo unknown-cpu)"
    CORES="$(nproc 2>/dev/null || echo '?')"
    OS_DESC="Linux $(uname -r)"
    ;;
  *)
    CPU="unknown-cpu"
    CORES="?"
    OS_DESC="$(uname -sr)"
    ;;
esac
HOST_DESC="${CPU} · $(uname -m) · ${CORES} cores · ${OS_DESC}"

# Peak-RSS mechanism: probe, do not assume.
TIME_MODE=none
if /usr/bin/time -l true >/dev/null 2>&1; then
  TIME_MODE=darwin # `maximum resident set size` in bytes
elif /usr/bin/time -v true >/dev/null 2>&1; then
  TIME_MODE=gnu # `Maximum resident set size (kbytes)`
fi

if [[ -n "${EPOCHREALTIME:-}" ]]; then
  TIMER_DESC='bash $EPOCHREALTIME (microsecond)'
  now_us() { printf '%s' "$EPOCHREALTIME"; }
elif command -v perl >/dev/null 2>&1; then
  TIMER_DESC='perl Time::HiRes (adds one fork per timestamp, ~ms bias)'
  now_us() { perl -MTime::HiRes=time -e 'printf "%.6f", time()'; }
else
  TIMER_DESC='none (bash < 4.4 and no perl) — cold-start columns read n/a'
  now_us() { printf ''; }
fi

# --- byte helpers -----------------------------------------------------------
raw_bytes() { wc -c <"$1" | tr -d ' '; }
gz_bytes() { gzip -9 -c <"$1" | wc -c | tr -d ' '; }
br_bytes() { brotli -q 11 -c <"$1" | wc -c | tr -d ' '; }

kb() { awk -v b="$1" 'BEGIN { printf "%.1f", b / 1000 }'; }

median() { # median <n> [n ...]
  printf '%s\n' "$@" | sort -n | awk '
    { a[NR] = $1 }
    END {
      if (NR == 0) { printf "n/a"; exit }
      if (NR % 2) { printf "%.1f", a[(NR + 1) / 2] }
      else { printf "%.1f", (a[NR / 2] + a[NR / 2 + 1]) / 2 }
    }'
}

minimum() { printf '%s\n' "$@" | sort -n | awk 'NR==1 { printf "%.1f", $1 }'; }

# --- first-render closure ---------------------------------------------------
# Local .js/.mjs/.css/.wasm references, from any quoting style, so this covers
# vite's `<script src>` / `<link href>` AND trunk's preload links plus the
# inline `init('/pkg_bg-<hash>.wasm')` module shim.
local_refs() { # local_refs <file>
  grep -oE "[\"'][^\"']+\.(js|mjs|css|wasm)([?#][^\"']*)?[\"']" "$1" 2>/dev/null |
    sed -E "s/^[\"']//; s/[\"']$//; s/[?#].*$//" |
    grep -vE '^(https?:)?//|^data:' || true
}

# --- report state -----------------------------------------------------------
FIRST_RENDER_ROWS=() # arm|kind|files|raw|gz|br|note
PER_FILE_ROWS=()     # arm|file|kind|raw|gz|br
DIST_TOTAL_ROWS=()   # arm|files|raw|gz|br
BIN_ROWS=()          # arm|path|size|stripped|min|median|runs|rss|note
SUMMARY_ROWS=()      # arm|basis|raw|gz|br
FINDINGS=()

measure_web() { # measure_web <label> <dist> <build cmd>
  local label=$1 dist=$2 build=$3

  if [[ ! -f "$dist/index.html" ]]; then
    local why="no $dist/index.html"
    [[ -d "$dist" ]] || why="no $dist"
    FIRST_RENDER_ROWS+=("$label|not built|—|—|—|—|$why — build: \`$build\`")
    SUMMARY_ROWS+=("$label|not built|—|—|—")
    return 0
  fi

  # 1. index.html's own references, in document order.
  local refs=() r
  while IFS= read -r r; do
    [[ -n "$r" ]] && refs+=("$r")
  done < <(local_refs "$dist/index.html")

  # 2. Resolve to dist-relative paths that exist; keep only the first sighting.
  local resolved=() rel
  for r in "${refs[@]:-}"; do
    [[ -n "$r" ]] || continue
    rel="${r#/}" # public_url defaults to "/", so refs are root-relative
    rel="${rel#./}"
    if [[ -f "$dist/$rel" ]]; then
      resolved+=("$rel")
    else
      FINDINGS+=("$label: index.html references '$r' which is not in the dist — counted as 0 bytes")
    fi
  done

  # 3. One level deep, .wasm ONLY: wasm-bindgen glue names its module in the
  #    JS. Not .js/.css — those would drag lazily-imported route chunks into a
  #    first-render total they are excluded from on purpose.
  local js w
  for js in "${resolved[@]:-}"; do
    [[ "$js" == *.js || "$js" == *.mjs ]] || continue
    while IFS= read -r w; do
      [[ -n "$w" && "$w" == *.wasm ]] || continue
      w="${w#/}"
      w="${w#./}"
      [[ -f "$dist/$w" ]] && resolved+=("$w")
    done < <(local_refs "$dist/$js")
  done

  local files=("index.html")
  while IFS= read -r rel; do
    [[ -n "$rel" && "$rel" != "index.html" ]] && files+=("$rel")
  done < <(printf '%s\n' "${resolved[@]:-}" | awk 'NF && !seen[$0]++')

  # 4. Per-file + per-kind + total, each file compressed on its own.
  local kinds="html css js wasm other"
  local k f ext kind n raw gz br
  local t_n=0 t_raw=0 t_gz=0 t_br=0
  local kn_html=0 kraw_html=0 kgz_html=0 kbr_html=0
  local kn_css=0 kraw_css=0 kgz_css=0 kbr_css=0
  local kn_js=0 kraw_js=0 kgz_js=0 kbr_js=0
  local kn_wasm=0 kraw_wasm=0 kgz_wasm=0 kbr_wasm=0
  local kn_other=0 kraw_other=0 kgz_other=0 kbr_other=0

  for f in "${files[@]}"; do
    ext="${f##*.}"
    case "$ext" in
      html | htm) kind=html ;;
      css) kind=css ;;
      js | mjs) kind=js ;;
      wasm) kind=wasm ;;
      *) kind=other ;;
    esac
    raw="$(raw_bytes "$dist/$f")"
    gz="$(gz_bytes "$dist/$f")"
    br="$(br_bytes "$dist/$f")"
    PER_FILE_ROWS+=("$label|$f|$kind|$raw|$gz|$br")
    eval "kn_$kind=\$((kn_$kind + 1))"
    eval "kraw_$kind=\$((kraw_$kind + raw))"
    eval "kgz_$kind=\$((kgz_$kind + gz))"
    eval "kbr_$kind=\$((kbr_$kind + br))"
    t_n=$((t_n + 1))
    t_raw=$((t_raw + raw))
    t_gz=$((t_gz + gz))
    t_br=$((t_br + br))
  done

  for k in $kinds; do
    eval "n=\$kn_$k; raw=\$kraw_$k; gz=\$kgz_$k; br=\$kbr_$k"
    [[ "$n" -gt 0 ]] || continue
    FIRST_RENDER_ROWS+=("$label|$k|$n|$raw|$gz|$br|")
  done
  FIRST_RENDER_ROWS+=("$label|**first render**|$t_n|**$t_raw**|**$t_gz**|**$t_br**|")
  SUMMARY_ROWS+=("$label|first render (browser downloads this)|$(kb "$t_raw")|$(kb "$t_gz")|$(kb "$t_br")")

  # 5. Whole-directory total — a DIFFERENT basis, reported separately.
  local d_n=0 d_raw=0 d_gz=0 d_br=0
  while IFS= read -r f; do
    [[ -n "$f" ]] || continue
    d_n=$((d_n + 1))
    d_raw=$((d_raw + $(raw_bytes "$f")))
    d_gz=$((d_gz + $(gz_bytes "$f")))
    d_br=$((d_br + $(br_bytes "$f")))
  done < <(find "$dist" -type f | sort)
  DIST_TOTAL_ROWS+=("$label|$d_n|$d_raw|$d_gz|$d_br")
}

run_guarded() { # run_guarded <cmd...> — kills the child after $TIMEOUT; 137 = killed
  local rc=0 pid wd
  "$@" >/dev/null 2>&1 &
  pid=$!
  {
    sleep "$TIMEOUT"
    kill -9 "$pid" 2>/dev/null || true
  } >/dev/null 2>&1 &
  wd=$!
  wait "$pid" 2>/dev/null || rc=$?
  kill "$wd" 2>/dev/null || true
  wait "$wd" 2>/dev/null || true
  return "$rc"
}

peak_rss() { # peak_rss <binary> <args...> -> bytes | n/a
  local out
  case "$TIME_MODE" in
    darwin)
      out="$( (/usr/bin/time -l "$@" 2>&1 >/dev/null) || true)"
      awk '/maximum resident set size/ { print $1; exit }' <<<"$out"
      ;;
    gnu)
      out="$( (/usr/bin/time -v "$@" 2>&1 >/dev/null) || true)"
      awk -F': *' '/Maximum resident set size/ { printf "%d", $2 * 1024; exit }' <<<"$out"
      ;;
    *) printf 'n/a' ;;
  esac
}

# One invocation, timed RUNS times -> "min|median|runs|rss|note"
time_invocation() { # time_invocation <binary> <args...>
  local bin=$1
  shift
  local argv=("$@") shown="$*" times=() note="" rc=0 t0 t1 i

  if [[ -z "$(now_us)" ]]; then
    printf '—|—|0|—|no high-resolution timer'
    return 0
  fi

  for ((i = 0; i < RUNS; i++)); do
    t0="$(now_us)"
    rc=0
    run_guarded "$bin" "${argv[@]}" || rc=$?
    t1="$(now_us)"
    if [[ "$rc" -ge 128 ]]; then
      note="\`$shown\` did not exit within ${TIMEOUT}s (killed) — not a terminating invocation for this binary"
      times=()
      break
    fi
    if [[ "$rc" -ne 0 && "$note" != *"exited $rc"* ]]; then
      note="${note:+$note; }\`$shown\` exited $rc — timing is of the failure path"
    fi
    times+=("$(awk -v a="$t0" -v b="$t1" 'BEGIN { printf "%.2f", (b - a) * 1000 }')")
  done

  local mn="—" md="—" rss="—"
  if [[ "${#times[@]}" -gt 0 ]]; then
    mn="$(minimum "${times[@]}")"
    md="$(median "${times[@]}")"
    rss="$(peak_rss "$bin" "${argv[@]}")"
    [[ -n "$rss" ]] || rss="n/a"
    if [[ "$rss" == "n/a" ]]; then
      note="${note:+$note; }no /usr/bin/time -l (Darwin) or -v (GNU) on this host"
    fi
  fi
  printf '%s|%s|%s|%s|%s' "$mn" "$md" "${#times[@]}" "$rss" "$note"
}

measure_bin() { # measure_bin <label> <path> <build cmd> [invocations] [arm note]
  local label=$1 bin=$2 build=$3 invocations=${4:-} arm_note=${5:-}
  [[ -n "$invocations" ]] || invocations='--version::'

  if [[ ! -f "$bin" ]]; then
    BIN_ROWS+=("$label|$bin|not built|—|—|—|—|—|—|build: \`$build\`${arm_note:+ — $arm_note}")
    SUMMARY_ROWS+=("$label|not built|—|—|—")
    return 0
  fi

  local size stripped tmp strip_note="" strip_ok=0
  size="$(raw_bytes "$bin")"
  stripped="$size"
  if command -v strip >/dev/null 2>&1; then
    tmp="$(mktemp -t bench-strip)"
    if cp "$bin" "$tmp" && strip "$tmp" >/dev/null 2>&1; then
      stripped="$(raw_bytes "$tmp")"
      strip_ok=1
    fi
    rm -f "$tmp"
  fi
  if [[ "$strip_ok" == 0 ]]; then
    strip_note="\`strip\` unavailable or refused this file — stripped column is the size on disk"
  elif [[ "$stripped" -ge "$size" ]]; then
    # `[profile.release] strip = true` already stripped this, and a second
    # pass can pad the Mach-O back up: never report a stripped size ABOVE the
    # size on disk, because the shipped artifact is the smaller of the two.
    stripped="$size"
    strip_note="\`strip\` removed nothing — already stripped (\`[profile.release] strip = true\`)"
  fi

  # One row per declared invocation. The artifact facts (size, arm note) ride
  # on the FIRST row only, so a two-invocation arm still reads as one artifact.
  local specs=() spec args inv_note result first=1 row_note mn md runs rss run_note
  IFS=';' read -ra specs <<<"$invocations"
  for spec in "${specs[@]}"; do
    args="${spec%%::*}"
    inv_note=""
    [[ "$spec" == *::* ]] && inv_note="${spec#*::}"
    result="$(time_invocation "$bin" $args)"
    IFS='|' read -r mn md runs rss run_note <<<"$result"
    row_note="$inv_note"
    [[ -n "$run_note" ]] && row_note="${row_note:+$row_note; }$run_note"
    if [[ "$first" == 1 ]]; then
      [[ -n "$strip_note" ]] && row_note="${row_note:+$row_note; }$strip_note"
      [[ -n "$arm_note" ]] && row_note="${row_note:+$row_note; }$arm_note"
      BIN_ROWS+=("$label|$bin|$size|$stripped|$args|$mn|$md|$runs|$rss|$row_note")
      first=0
    else
      BIN_ROWS+=("$label|↳ same artifact|—|—|$args|$mn|$md|$runs|$rss|$row_note")
    fi
  done

  SUMMARY_ROWS+=("$label|artifact on disk (installed once, not per render)${arm_note:+ — $arm_note}|$(kb "$size")|—|—")
}

# --- arm selection ----------------------------------------------------------
ARMS=()
if [[ "${#ARGS[@]}" -eq 0 ]]; then
  ARMS=("${STANDARD_ARMS[@]}")
else
  for a in "${ARGS[@]}"; do
    if [[ -d "$a" || "$a" == */dist ]]; then
      ARMS+=("web|$a|$a|(build this dist yourself)")
    elif [[ -f "$a" ]]; then
      ARMS+=("bin|$(basename "$a")|$a|(build this binary yourself)")
    else
      echo "FATAL: '$a' is neither a directory nor a file. Pass a web dist dir or a compiled binary." >&2
      exit 2
    fi
  done
fi

for arm in "${ARMS[@]}"; do
  IFS='|' read -r kind label path build invocations arm_note <<<"$arm"
  case "$kind" in
    web) measure_web "$label" "$path" "$build" ;;
    bin) measure_bin "$label" "$path" "$build" "${invocations:-}" "${arm_note:-}" ;;
  esac
done

# --- report -----------------------------------------------------------------
echo "# Shipped assets and startup, one basis"
echo
echo "## $(date -u +%F) — $HOST_DESC"
echo
echo "- Reproduce: \`just bench-assets\` (\`scripts/bench/assets.sh\`), $(date -u +%FT%TZ)."
echo "- Compressors: \`$GZIP_VERSION\` at \`-9\`, \`$BROTLI_VERSION\` at \`-q 11\`, every file compressed on its own (one file = one response)."
echo "- Web arm = FIRST-RENDER CLOSURE: \`index.html\` + the \`.js\`/\`.css\`/\`.wasm\` it references (+ the \`.wasm\` named inside that JS). Lazily imported route chunks are excluded; whole-directory totals are the separate table below."
echo "- Binary arm = artifact size + wall time of each named invocation over $RUNS runs (min/median) with that invocation's peak RSS. \`--version\` is measured, not assumed to be free: a spec-driven CLI builds its command tree before it can answer, and the note column says when that is what you are reading. A binary is installed once, not downloaded per render, so its bytes are NOT the browser arms' bytes; the comparable number is \`scripts/bench/ops.sh\` (bytes per operation)."
echo "- Wall times are load-sensitive (bytes are not): \`min\` is the least contaminated estimate, and a run competing with a cargo build can double the median. Re-run on a quiet machine before quoting a startup number."
echo "- Timer: $TIMER_DESC. Peak RSS: $(case "$TIME_MODE" in darwin) echo '/usr/bin/time -l, bytes (Darwin)' ;; gnu) echo '/usr/bin/time -v, kbytes x 1024 (GNU)' ;; *) echo 'unavailable on this host' ;; esac)."
echo
echo "### First render, per asset kind (bytes)"
echo
echo "| arm | asset kind | files | raw | gzip -9 | brotli -q 11 | note |"
echo "|---|---|---:|---:|---:|---:|---|"
if [[ "${#FIRST_RENDER_ROWS[@]}" -eq 0 ]]; then
  echo "| (no web arm measured) | — | — | — | — | — | — |"
else
  for row in "${FIRST_RENDER_ROWS[@]}"; do
    IFS='|' read -r a k n raw gz br note <<<"$row"
    echo "| $a | $k | $n | $raw | $gz | $br | ${note:-} |"
  done
fi
echo
echo "### Binaries — size, startup, peak RSS"
echo
echo "| arm | path | on disk | stripped | invocation | wall min | median | runs | peak RSS | note |"
echo "|---|---|---:|---:|---|---:|---:|---:|---:|---|"
if [[ "${#BIN_ROWS[@]}" -eq 0 ]]; then
  echo "| (no binary arm measured) | — | — | — | — | — | — | — | — | — |"
else
  for row in "${BIN_ROWS[@]}"; do
    IFS='|' read -r a p size stripped inv mn md runs rss note <<<"$row"
    [[ "$mn" == "—" ]] || mn="${mn} ms"
    [[ "$md" == "—" ]] || md="${md} ms"
    [[ "$p" == "↳ same artifact" ]] || p="\`$p\`"
    [[ "$inv" == "—" ]] || inv="\`$inv\`"
    echo "| $a | $p | $size | $stripped | $inv | $mn | $md | $runs | $rss | ${note:-} |"
  done
fi
echo
echo "### Whole dist directory (bytes) — context, NOT first render"
echo
echo "| arm | files | raw | gzip -9 | brotli -q 11 |"
echo "|---|---:|---:|---:|---:|"
if [[ "${#DIST_TOTAL_ROWS[@]}" -eq 0 ]]; then
  echo "| (no web arm measured) | — | — | — | — |"
else
  for row in "${DIST_TOTAL_ROWS[@]}"; do
    IFS='|' read -r a n raw gz br <<<"$row"
    echo "| $a | $n | $raw | $gz | $br |"
  done
fi
echo
echo "### Per-file first-render breakdown (bytes)"
echo
echo "| arm | file | kind | raw | gzip -9 | brotli -q 11 |"
echo "|---|---|---|---:|---:|---:|"
if [[ "${#PER_FILE_ROWS[@]}" -eq 0 ]]; then
  echo "| (no web arm measured) | — | — | — | — | — |"
else
  for row in "${PER_FILE_ROWS[@]}"; do
    IFS='|' read -r a f k raw gz br <<<"$row"
    echo "| $a | \`$f\` | $k | $raw | $gz | $br |"
  done
fi
echo
echo "### Cross-arm summary (kB, 1 kB = 1000 B)"
echo
echo "| arm | basis | raw | gzip -9 | brotli -q 11 |"
echo "|---|---|---:|---:|---:|"
for row in "${SUMMARY_ROWS[@]}"; do
  IFS='|' read -r a basis raw gz br <<<"$row"
  echo "| $a | $basis | $raw | $gz | $br |"
done

if [[ "${#FINDINGS[@]}" -gt 0 ]]; then
  echo
  echo "### Findings"
  echo
  for f in "${FINDINGS[@]}"; do echo "- $f"; done
fi
