#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEFAULT_ROADMAP="$ROOT_DIR/docs/roadmap.md"
TASK_PATTERN='^[[:space:]]*-[[:space:]]\[[[:space:]]\][[:space:]]+'

ROADMAP_PATH="$DEFAULT_ROADMAP"
MAX_ITERATIONS=30
NO_PROGRESS_LIMIT=3
LOG_DIR="$ROOT_DIR/target/codex-roadmap-loop"
MODEL=""
PROFILE=""
SANDBOX_MODE="workspace-write"
USE_FULL_AUTO=1
ALLOW_DIRTY=0
DRY_RUN=0

CODEX_EXTRA_ARGS=()

usage() {
  cat <<'USAGE'
Run a Codex exec loop that iteratively implements roadmap tasks.

Usage:
  scripts/codex-roadmap-loop.sh [options]

Options:
  --roadmap <path>            Roadmap file path (default: docs/roadmap.md)
  --max-iterations <n>        Max codex exec runs before stopping (default: 30)
  --no-progress-limit <n>     Stop after n runs with no checkbox reduction (default: 3)
  --log-dir <path>            Directory for per-iteration logs
  --model <model>             Forwarded to codex exec --model
  --profile <profile>         Forwarded to codex exec --profile
  --sandbox <mode>            codex sandbox mode (read-only|workspace-write|danger-full-access)
  --no-full-auto              Do not add --full-auto to codex exec
  --allow-dirty               Allow starting with uncommitted local changes
  --codex-arg <arg>           Extra argument (repeatable) forwarded to codex exec
  --dry-run                   Print the first generated prompt and exit
  -h, --help                  Show this help

Environment overrides:
  CODEX_MODEL, CODEX_PROFILE, CODEX_SANDBOX_MODE
  CODEX_MAX_ITERATIONS, CODEX_NO_PROGRESS_LIMIT, CODEX_LOG_DIR
USAGE
}

die() {
  echo "error: $*" >&2
  exit 1
}

is_positive_int() {
  [[ "$1" =~ ^[1-9][0-9]*$ ]]
}

count_open_tasks() {
  awk -v pattern="$TASK_PATTERN" '$0 ~ pattern {count++} END {print count+0}' "$ROADMAP_PATH"
}

first_open_task() {
  awk -v pattern="$TASK_PATTERN" '$0 ~ pattern {print NR ":" $0; exit}' "$ROADMAP_PATH"
}

task_section_for_line() {
  local line_no="$1"
  awk -v n="$line_no" 'NR<=n && /^## / {section=$0} END {print section}' "$ROADMAP_PATH"
}

load_env_overrides() {
  if [[ -n "${CODEX_MODEL:-}" && -z "$MODEL" ]]; then
    MODEL="$CODEX_MODEL"
  fi
  if [[ -n "${CODEX_PROFILE:-}" && -z "$PROFILE" ]]; then
    PROFILE="$CODEX_PROFILE"
  fi
  if [[ -n "${CODEX_SANDBOX_MODE:-}" ]]; then
    SANDBOX_MODE="$CODEX_SANDBOX_MODE"
  fi
  if [[ -n "${CODEX_MAX_ITERATIONS:-}" ]]; then
    MAX_ITERATIONS="$CODEX_MAX_ITERATIONS"
  fi
  if [[ -n "${CODEX_NO_PROGRESS_LIMIT:-}" ]]; then
    NO_PROGRESS_LIMIT="$CODEX_NO_PROGRESS_LIMIT"
  fi
  if [[ -n "${CODEX_LOG_DIR:-}" ]]; then
    LOG_DIR="$CODEX_LOG_DIR"
  fi
}

parse_args() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --roadmap)
        shift
        [[ $# -gt 0 ]] || die "--roadmap requires a value"
        ROADMAP_PATH="$1"
        ;;
      --max-iterations)
        shift
        [[ $# -gt 0 ]] || die "--max-iterations requires a value"
        MAX_ITERATIONS="$1"
        ;;
      --no-progress-limit)
        shift
        [[ $# -gt 0 ]] || die "--no-progress-limit requires a value"
        NO_PROGRESS_LIMIT="$1"
        ;;
      --log-dir)
        shift
        [[ $# -gt 0 ]] || die "--log-dir requires a value"
        LOG_DIR="$1"
        ;;
      --model)
        shift
        [[ $# -gt 0 ]] || die "--model requires a value"
        MODEL="$1"
        ;;
      --profile)
        shift
        [[ $# -gt 0 ]] || die "--profile requires a value"
        PROFILE="$1"
        ;;
      --sandbox)
        shift
        [[ $# -gt 0 ]] || die "--sandbox requires a value"
        SANDBOX_MODE="$1"
        ;;
      --no-full-auto)
        USE_FULL_AUTO=0
        ;;
      --allow-dirty)
        ALLOW_DIRTY=1
        ;;
      --codex-arg)
        shift
        [[ $# -gt 0 ]] || die "--codex-arg requires a value"
        CODEX_EXTRA_ARGS+=("$1")
        ;;
      --dry-run)
        DRY_RUN=1
        ;;
      -h|--help)
        usage
        exit 0
        ;;
      *)
        die "unknown option: $1"
        ;;
    esac
    shift
  done
}

validate_inputs() {
  command -v codex >/dev/null 2>&1 || die "codex CLI not found in PATH"
  command -v git >/dev/null 2>&1 || die "git not found in PATH"

  [[ -f "$ROADMAP_PATH" ]] || die "roadmap file not found: $ROADMAP_PATH"

  is_positive_int "$MAX_ITERATIONS" || die "--max-iterations must be a positive integer"
  is_positive_int "$NO_PROGRESS_LIMIT" || die "--no-progress-limit must be a positive integer"

  case "$SANDBOX_MODE" in
    read-only|workspace-write|danger-full-access) ;;
    *) die "--sandbox must be one of: read-only, workspace-write, danger-full-access" ;;
  esac

  if [[ "$ALLOW_DIRTY" -eq 0 ]] && [[ -n "$(git -C "$ROOT_DIR" status --porcelain)" ]]; then
    die "working tree is dirty; commit/stash changes first or pass --allow-dirty"
  fi
}

build_prompt_file() {
  local line_no="$1"
  local section="$2"
  local task_text="$3"
  local open_count="$4"
  local prompt_file="$5"

  cat > "$prompt_file" <<EOF_PROMPT
You are in repository: $ROOT_DIR
Roadmap file: $ROADMAP_PATH

There are currently $open_count unchecked roadmap items.
Focus on the first unchecked roadmap item in this run only.

Section: ${section:-"(none)"}
Task line: $line_no
Task text: $task_text

Execution requirements for this run:
1. Implement the target task end-to-end (code, tests, docs as needed).
2. Update roadmap checkboxes in $ROADMAP_PATH for work completed in this run.
3. Run validation commands from AGENTS.md:
   - cargo fmt --check
   - cargo clippy --all-targets --all-features -- -D warnings
   - cargo test
   - cargo build
4. Commit all changes from this run with a clear commit message.
5. If blocked, document blocker in roadmap and commit the best partial progress.

Do not ask for a plan only; perform implementation now.
EOF_PROMPT
}

run_loop() {
  mkdir -p "$LOG_DIR"
  cd "$ROOT_DIR"

  local run_id
  run_id="$(date +%Y%m%d-%H%M%S)"
  local iteration=1
  local no_progress_count=0

  local codex_cmd=(codex exec --cd "$ROOT_DIR" --sandbox "$SANDBOX_MODE")
  if [[ "$USE_FULL_AUTO" -eq 1 ]]; then
    codex_cmd+=(--full-auto)
  fi
  if [[ -n "$MODEL" ]]; then
    codex_cmd+=(--model "$MODEL")
  fi
  if [[ -n "$PROFILE" ]]; then
    codex_cmd+=(--profile "$PROFILE")
  fi
  if [[ ${#CODEX_EXTRA_ARGS[@]} -gt 0 ]]; then
    codex_cmd+=("${CODEX_EXTRA_ARGS[@]}")
  fi

  local initial_open
  initial_open="$(count_open_tasks)"
  if [[ "$initial_open" -eq 0 ]]; then
    echo "No unchecked roadmap tasks found in $ROADMAP_PATH"
    return 0
  fi

  echo "Starting codex roadmap loop"
  echo "Roadmap: $ROADMAP_PATH"
  echo "Initial unchecked tasks: $initial_open"
  echo "Logs: $LOG_DIR"

  while [[ "$iteration" -le "$MAX_ITERATIONS" ]]; do
    local open_before
    open_before="$(count_open_tasks)"
    if [[ "$open_before" -eq 0 ]]; then
      echo "All roadmap tasks are complete."
      return 0
    fi

    local first_open
    first_open="$(first_open_task)"
    if [[ -z "$first_open" ]]; then
      echo "No unchecked tasks found; exiting."
      return 0
    fi

    local line_no="${first_open%%:*}"
    local line_text="${first_open#*:}"
    local task_text
    task_text="$(printf '%s\n' "$line_text" | sed -E 's/^[[:space:]]*-[[:space:]]\[[[:space:]]\][[:space:]]*//')"
    local section
    section="$(task_section_for_line "$line_no")"

    local prompt_file
    prompt_file="$(mktemp "${TMPDIR:-/tmp}/codex-roadmap-prompt.XXXXXX")"
    build_prompt_file "$line_no" "$section" "$task_text" "$open_before" "$prompt_file"

    if [[ "$DRY_RUN" -eq 1 ]]; then
      echo "--- dry run prompt ---"
      cat "$prompt_file"
      rm -f "$prompt_file"
      return 0
    fi

    local iteration_log="$LOG_DIR/${run_id}-iter-${iteration}.log"

    echo
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] Iteration $iteration/$MAX_ITERATIONS"
    echo "Target: line $line_no | $task_text"
    echo "Log: $iteration_log"

    set +e
    "${codex_cmd[@]}" - < "$prompt_file" 2>&1 | tee "$iteration_log"
    local exec_code=${PIPESTATUS[0]}
    set -e

    rm -f "$prompt_file"

    if [[ "$exec_code" -ne 0 ]]; then
      echo "codex exec failed with exit code $exec_code (iteration $iteration)"
      return "$exec_code"
    fi

    local open_after
    open_after="$(count_open_tasks)"
    echo "Unchecked tasks: $open_before -> $open_after"

    if [[ "$open_after" -lt "$open_before" ]]; then
      no_progress_count=0
    else
      no_progress_count=$((no_progress_count + 1))
    fi

    if [[ "$open_after" -eq 0 ]]; then
      echo "All roadmap tasks are complete."
      return 0
    fi

    if [[ "$no_progress_count" -ge "$NO_PROGRESS_LIMIT" ]]; then
      echo "No checkbox progress for $no_progress_count iteration(s); stopping to avoid infinite loop."
      return 2
    fi

    iteration=$((iteration + 1))
  done

  echo "Reached max iterations ($MAX_ITERATIONS) with remaining roadmap tasks."
  return 3
}

main() {
  load_env_overrides
  parse_args "$@"
  validate_inputs
  run_loop
}

main "$@"
