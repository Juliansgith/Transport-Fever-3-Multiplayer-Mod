#!/usr/bin/env bash
# Makes GitHub enforce the branch rules in AGENTS.md, for everyone,
# administrators included:
#
# - dev, acceptance and main can be neither force-pushed nor deleted;
# - acceptance takes only commits whose ci checks passed;
# - main takes only commits whose ci and acceptance checks passed.
#
# Promotions are fast-forwards of commits already tested on the branch
# before, so they pass; a commit made on the branch itself, such as a pull
# request's merge commit, has no checks yet and is refused.
#
# Run it once as a repository administrator, with the GitHub CLI signed in:
#
#   tools/github/protect-branches.sh [owner/repo]
#
# Running it again reapplies the same rules. Check names are the jobs of
# .github/workflows/ci.yml and acceptance.yml; update both lists when a job
# is renamed or added.
set -euo pipefail

repo="${1:-Juliansgith/TPF3-MP}"
# GitHub Actions, so only checks from this repository's workflows count.
actions_app=15368

ci=(
  "test (windows-latest)" "test (ubuntu-latest)" "test (macos-latest)"
  "release build (windows-latest)" "release build (ubuntu-latest)"
  "release build (macos-latest)" "server image"
)
acceptance=(
  "load (windows-latest)" "load (ubuntu-latest)" "load (macos-latest)"
  "soak (ubuntu-latest)"
)

# The protection body for a branch that requires the checks named after it.
rules() {
  local checks="" name
  for name in "$@"; do
    checks+="${checks:+,}{\"context\":\"$name\",\"app_id\":$actions_app}"
  done
  local required="null"
  if [ -n "$checks" ]; then
    required="{\"strict\":false,\"checks\":[$checks]}"
  fi
  printf '{"required_status_checks":%s,"enforce_admins":true,' "$required"
  printf '"required_pull_request_reviews":null,"restrictions":null,'
  printf '"allow_force_pushes":false,"allow_deletions":false,'
  printf '"required_linear_history":false}'
}

protect() {
  local branch="$1"
  shift
  rules "$@" | gh api -X PUT "repos/$repo/branches/$branch/protection" --input - \
    --jq '"'"$branch"': \(.required_status_checks.checks // [] | length) required checks, force-push \(.allow_force_pushes.enabled), deletion \(.allow_deletions.enabled), admins bound \(.enforce_admins.enabled)"'
}

protect dev
protect acceptance "${ci[@]}"
protect main "${ci[@]}" "${acceptance[@]}"
