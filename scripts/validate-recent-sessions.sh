#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Validate ccbox timeline parsing (CLI history + details) across recent sessions.

USAGE:
  scripts/validate-recent-sessions.sh [--count N] [--per-project N] [--max-bytes N] [--bin PATH] [--no-build]

DEFAULTS:
  --count 10         Validate 10 sessions (distinct projects when possible)
  --per-project 5    Consider up to 5 newest sessions per project
  --max-bytes 50000000  Skip sessions larger than this (50MB) to avoid very slow parses
  --bin target/debug/ccbox (built automatically unless --no-build is set)

EXAMPLES:
  scripts/validate-recent-sessions.sh
  scripts/validate-recent-sessions.sh --count 10 --max-bytes 200000000
  scripts/validate-recent-sessions.sh --bin ./target/release/ccbox --no-build
EOF
}

count=10
per_project=5
max_bytes=50000000
ccbox_bin=""
no_build=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --count)
      count="${2:-}"
      shift 2
      ;;
    --per-project)
      per_project="${2:-}"
      shift 2
      ;;
    --max-bytes)
      max_bytes="${2:-}"
      shift 2
      ;;
    --bin)
      ccbox_bin="${2:-}"
      shift 2
      ;;
    --no-build)
      no_build=1
      shift 1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"
cd "$repo_root"

if [[ -z "$ccbox_bin" ]]; then
  ccbox_bin="$repo_root/target/debug/ccbox"
fi

if [[ "$no_build" -eq 0 ]]; then
  echo "Building ccbox (debug)..."
  cargo build -q
fi

if [[ ! -x "$ccbox_bin" ]]; then
  echo "ccbox binary not found or not executable: $ccbox_bin" >&2
  exit 1
fi

detect_engine() {
  local log_path="$1"
  if [[ "$log_path" == *"/.claude/"* ]]; then
    echo "claude"
    return 0
  fi
  if [[ "$log_path" == *"/.gemini/"* ]]; then
    echo "gemini"
    return 0
  fi
  if [[ "$log_path" == *"/opencode/sessions/"* ]]; then
    echo "opencode"
    return 0
  fi
  if [[ "$log_path" == *"/.codex/"* ]]; then
    echo "codex"
    return 0
  fi
  echo "unknown"
}

has_line() {
  local file="$1"
  local value="$2"
  if [[ ! -f "$file" ]]; then
    return 1
  fi
  grep -Fxq -- "$value" "$file"
}

tmp_dir="$(mktemp -d)"
cleanup() {
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

candidates="$tmp_dir/candidates.tsv"
selected="$tmp_dir/selected.tsv"
selected_projects="$tmp_dir/selected_projects.txt"

: > "$candidates"
: > "$selected"
: > "$selected_projects"

echo "Collecting candidates (up to ${per_project} sessions per project, max-bytes=${max_bytes})..."
projects_file="$tmp_dir/projects.txt"
: > "$projects_file"

"$ccbox_bin" projects 2>/dev/null | awk -F'\t' 'NF>=3 {print $2}' > "$projects_file"

while IFS= read -r project_path; do
  [[ -z "$project_path" ]] && continue
  # sessions output columns with --size:
  # started_at<TAB>session_id<TAB>title...<TAB>file_size_bytes<TAB>log_path
  "$ccbox_bin" sessions "$project_path" --limit "$per_project" --size 2>/dev/null \
    | awk -F'\t' -v proj="$project_path" -v max="$max_bytes" 'NF>=4 {size=$(NF-1)+0; if (size<=max) print proj "\t" $2 "\t" $NF "\t" size "\t" $1}'
done < "$projects_file" >> "$candidates"

if [[ ! -s "$candidates" ]]; then
  echo "No candidate sessions found (or all were larger than --max-bytes)." >&2
  exit 1
fi

sorted_candidates="$tmp_dir/candidates.sorted.tsv"
sort -t $'\t' -k5,5r "$candidates" > "$sorted_candidates"

pick_one_for_engine() {
  local engine="$1"
  while IFS=$'\t' read -r project_path session_id log_path size started_at; do
    [[ -z "$project_path" || -z "$session_id" || -z "$log_path" ]] && continue
    local detected
    detected="$(detect_engine "$log_path")"
    [[ "$detected" != "$engine" ]] && continue
    if has_line "$selected_projects" "$project_path"; then
      continue
    fi
    echo -e "${project_path}\t${session_id}\t${log_path}\t${size}\t${started_at}\t${detected}" >> "$selected"
    echo "$project_path" >> "$selected_projects"
    return 0
  done < "$sorted_candidates"
  return 1
}

fill_remaining() {
  while IFS=$'\t' read -r project_path session_id log_path size started_at; do
    [[ -z "$project_path" || -z "$session_id" || -z "$log_path" ]] && continue
    if has_line "$selected_projects" "$project_path"; then
      continue
    fi
    local detected
    detected="$(detect_engine "$log_path")"
    echo -e "${project_path}\t${session_id}\t${log_path}\t${size}\t${started_at}\t${detected}" >> "$selected"
    echo "$project_path" >> "$selected_projects"
    local picked
    picked="$(wc -l < "$selected" | tr -d ' ')"
    if [[ "$picked" -ge "$count" ]]; then
      return 0
    fi
  done < "$sorted_candidates"
  return 0
}

echo "Selecting up to ${count} sessions (distinct projects), preferring engine diversity..."
for engine in codex claude gemini opencode; do
  if [[ "$(wc -l < "$selected" | tr -d ' ')" -ge "$count" ]]; then
    break
  fi
  pick_one_for_engine "$engine" || true
done
fill_remaining

picked_count="$(wc -l < "$selected" | tr -d ' ')"
if [[ "$picked_count" -eq 0 ]]; then
  echo "No sessions selected." >&2
  exit 1
fi
if [[ "$picked_count" -lt "$count" ]]; then
  echo "Selected ${picked_count}/${count} sessions (not enough distinct projects under --max-bytes)." >&2
fi

echo
echo "Validating ${picked_count} session(s)..."
echo

failures=0
validated=0

while IFS=$'\t' read -r project_path session_id log_path size started_at engine; do
  validated=$((validated + 1))
  echo "[$validated/$picked_count] ${engine}  id=${session_id}  size=${size}  project=${project_path}"

  out_file="$tmp_dir/${session_id}.out"
  err_file="$tmp_dir/${session_id}.err"
  full_file="$tmp_dir/${session_id}.full"

  if ! "$ccbox_bin" history "$project_path" --id "$session_id" --limit 10 --size >"$out_file" 2>"$err_file"; then
    echo "  FAIL: history command failed" >&2
    failures=$((failures + 1))
    continue
  fi

  items_total="$(grep -Eo 'items_total=[0-9]+' "$err_file" | head -n 1 | cut -d= -f2 || true)"
  if [[ -z "$items_total" || "$items_total" -le 0 ]]; then
    echo "  FAIL: timeline items_total not found or zero" >&2
    failures=$((failures + 1))
    continue
  fi

  first_two_users="$(grep -E 'USER: ' "$out_file" | head -n 2 | sed 's/.*USER: //')"
  user_1="$(echo "$first_two_users" | sed -n '1p')"
  user_2="$(echo "$first_two_users" | sed -n '2p')"
  if [[ -n "$user_1" && -n "$user_2" && "$user_1" == "$user_2" ]]; then
    echo "  FAIL: first user message appears duplicated" >&2
    failures=$((failures + 1))
    continue
  fi

  if ! "$ccbox_bin" history "$project_path" --id "$session_id" --limit 5 --full >"$full_file" 2>>"$err_file"; then
    echo "  FAIL: history --full command failed" >&2
    failures=$((failures + 1))
    continue
  fi

  if ! grep -q '^  ' "$full_file"; then
    echo "  FAIL: no detail lines printed (details view would be empty)" >&2
    failures=$((failures + 1))
    continue
  fi

  echo "  OK (items_total=${items_total})"
done < "$selected"

echo
if [[ "$failures" -eq 0 ]]; then
  echo "All validations passed."
  exit 0
fi

echo "Validations failed: ${failures}" >&2
exit 1

