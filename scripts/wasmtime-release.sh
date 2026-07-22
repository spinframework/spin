#!/usr/bin/env bash

# Finalize an in-flight "Bump wasmtime (prerelease)" once wasmtime is published
# to crates.io. Complements bump-wasmtime.yml, which points Spin at a wasmtime
# release *branch* (a git dependency) so integration can begin before wasmtime
# ships.
#
# Subcommands:
#   update    Rewrite the open bump PR's wasmtime git dependency to the published
#             crates.io version and push the change to the PR branch. A human then
#             reviews (and possibly amends) that PR before it is merged.
#   backport  After the bump PR has merged to main, backport it onto the last Spin
#             release branch, bump the patch version, and open a release PR.
#
# The two phases are intentionally separate so the bump PR can be reviewed and
# merged before the backport/patch release is kicked off.
#
# Configuration via environment (all optional; auto-discovered when unset):
#   PR_NUMBER        Bump-wasmtime PR number
#   CRATES_VERSION   Published wasmtime crates.io version to pin
#   RELEASE_BRANCH   Spin release branch to backport into (e.g. v4.0)
#
# Requires: gh (authenticated), jq, curl, cargo, git, and GNU sed. Commits are
# signed when git is configured with a signing key (as it is in CI).

set -euo pipefail

WASMTIME_GIT="https://github.com/bytecodealliance/wasmtime"
WASMTIME_REPO="bytecodealliance/wasmtime"

log() { echo "[wasmtime-release] $*"; }
die() {
  echo "[wasmtime-release] error: $*" >&2
  exit 1
}

# Most recently opened bump-wasmtime PR in the given state (open|merged).
# Echoes "<number> <headRef>", or nothing if there is no match.
find_bump_pr() {
  local state="$1"
  gh pr list --state "$state" --json number,headRefName --limit 50 |
    jq -r '[.[] | select(.headRefName | startswith("bump-wasmtime/prerelease-"))]
           | sort_by(.number) | last
           | if . == null then "" else "\(.number) \(.headRefName)" end'
}

# Wasmtime major version encoded in a "bump-wasmtime/prerelease-<version>" branch.
major_from_branch() {
  local version="${1#bump-wasmtime/prerelease-}"
  echo "${version%%.*}"
}

# Highest published, non-yanked, stable major.* wasmtime version on crates.io.
latest_crates_version() {
  local major="$1" nums
  nums=$(curl -sSL -H 'User-Agent: spinframework-ci (spin@spinframework.dev)' \
    "https://crates.io/api/v1/crates/wasmtime" |
    jq -r '.versions[] | select(.yanked==false) | .num') || return 0
  printf '%s\n' "$nums" | grep -E "^${major}\.[0-9]+\.[0-9]+$" | sort -V | tail -n1 || true
}

# Highest vX.Y Spin release branch on origin.
latest_release_branch() {
  git ls-remote --heads origin 'v[0-9]*.[0-9]*' |
    awk '{print $2}' | sed 's#refs/heads/##' |
    grep -E '^v[0-9]+\.[0-9]+$' | sort -V | tail -n1 || true
}

# Wasmtime version currently pinned in a ref's workspace Cargo.toml.
wasmtime_version_on_ref() {
  local ref="$1"
  git show "${ref}:Cargo.toml" |
    grep -E '^wasmtime = ' | head -n1 |
    grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -n1 || true
}

# Next patch version for a release branch, read from its Cargo.toml.
next_patch_version() {
  local branch="$1" current
  current=$(git show "origin/${branch}:Cargo.toml" |
    grep -E '^version = "' | head -n1 | sed -E 's/version = "([^"]+)"/\1/')
  [ -n "$current" ] || die "could not read the version from origin/${branch}:Cargo.toml"
  local major="${current%%.*}" rest="${current#*.}"
  local minor="${rest%%.*}" patch="${rest#*.}"
  patch="${patch%%[-+]*}" # strip any pre-release/build suffix
  echo "${major}.${minor}.$((patch + 1))"
}

# Rewrite wasmtime git/branch dependencies back to a crates.io version. This is
# the inverse of the rewrite performed by bump-wasmtime.yml.
rewrite_to_crates_io() {
  local version="$1"
  shift
  sed -i -E \
    "s#git = \"${WASMTIME_GIT}\", branch = \"release-[0-9][0-9.]*\"#version = \"${version}\"#g" \
    "$@"
  # Collapse a bare wasmtime table back to the plain-version form.
  sed -i -E "s#^wasmtime = \{ version = \"${version}\" \}#wasmtime = \"${version}\"#" Cargo.toml
}

cmd_update() {
  local pr_number="${PR_NUMBER:-}" pr_branch="" crates_version="${CRATES_VERSION:-}"

  if [ -n "$pr_number" ]; then
    pr_branch=$(gh pr view "$pr_number" --json headRefName --jq '.headRefName')
  else
    read -r pr_number pr_branch <<<"$(find_bump_pr open)" || true
  fi
  if [ -z "$pr_number" ] || [ -z "$pr_branch" ]; then
    die "no open bump-wasmtime PR found"
  fi
  log "Finalizing bump PR #${pr_number} (${pr_branch})"

  if [ -z "$crates_version" ]; then
    crates_version=$(latest_crates_version "$(major_from_branch "$pr_branch")")
  fi
  [ -n "$crates_version" ] || die "wasmtime is not published on crates.io yet"
  log "Published crates.io version: ${crates_version}"

  git fetch origin "$pr_branch"
  git checkout -B "$pr_branch" "origin/${pr_branch}"

  if ! grep -q "$WASMTIME_REPO" Cargo.toml examples/spin-timer/Cargo.toml; then
    log "Bump PR already points at a crates.io release; nothing to do."
    return 0
  fi

  rewrite_to_crates_io "$crates_version" Cargo.toml examples/spin-timer/Cargo.toml

  if grep -q "$WASMTIME_REPO" Cargo.toml examples/spin-timer/Cargo.toml; then
    die "wasmtime git references remain after the rewrite; the Cargo.toml format may have changed"
  fi

  cargo update
  cargo update --manifest-path examples/spin-timer/Cargo.toml

  git commit -a -m "Update wasmtime to v${crates_version}"
  git push origin "HEAD:${pr_branch}"
  log "Pushed crates.io update to ${pr_branch}"
}

cmd_backport() {
  local pr_number="${PR_NUMBER:-}" pr_branch="" crates_version="${CRATES_VERSION:-}"
  local release_branch="${RELEASE_BRANCH:-}"

  if [ -n "$pr_number" ]; then
    pr_branch=$(gh pr view "$pr_number" --json headRefName --jq '.headRefName')
  else
    # Most recent bump PR regardless of state; the merge check below rejects it
    # if it hasn't landed yet, so we never backport a stale one.
    read -r pr_number pr_branch <<<"$(find_bump_pr all)" || true
  fi
  if [ -z "$pr_number" ] || [ -z "$pr_branch" ]; then
    die "no bump-wasmtime PR found"
  fi

  # Backport what actually landed on main, so only run once the PR has merged.
  local merge_commit
  merge_commit=$(gh pr view "$pr_number" --json mergeCommit --jq '.mergeCommit.oid // empty')
  [ -n "$merge_commit" ] || die "PR #${pr_number} has not been merged yet"
  log "Backporting merged bump PR #${pr_number} (merge commit ${merge_commit})"

  # Read the wasmtime version that actually landed on main (what we're backporting).
  git fetch origin main
  if [ -z "$crates_version" ]; then
    crates_version=$(wasmtime_version_on_ref origin/main)
  fi
  [ -n "$crates_version" ] || die "could not read the wasmtime version pinned on main"

  if [ -z "$release_branch" ]; then
    release_branch=$(latest_release_branch)
  fi
  [ -n "$release_branch" ] || die "could not determine a Spin release branch"
  git fetch origin "$release_branch"

  local next_version
  next_version=$(next_patch_version "$release_branch")
  log "Release branch ${release_branch}; patch release will be ${next_version}"

  local backport_branch="backport-wasmtime-${crates_version}-to-${release_branch}"
  if git ls-remote --exit-code --heads origin "$backport_branch" >/dev/null 2>&1; then
    log "Backport branch ${backport_branch} already exists; nothing to do."
    return 0
  fi

  git checkout -B "$backport_branch" "origin/${release_branch}"

  # Cherry-pick the PR as it landed on main. Spin uses both squash and merge
  # commits, so add -m 1 when the merge point has more than one parent.
  local cherry_pick=(git cherry-pick -x)
  if [ "$(git rev-list --parents -n1 "$merge_commit" | wc -w)" -ge 3 ]; then
    cherry_pick+=(-m 1)
  fi
  if ! "${cherry_pick[@]}" "$merge_commit"; then
    git cherry-pick --abort || true
    die "cherry-pick of #${pr_number} onto ${release_branch} hit conflicts; backport manually with .github/gh-backport.sh ${pr_number} ${release_branch}"
  fi

  # Bump the patch version and refresh the workspace lockfile.
  sed -i -E "s/^version = \"[0-9]+\.[0-9]+\.[0-9]+([^\"]*)\"/version = \"${next_version}\"/" Cargo.toml
  cargo update -w
  git commit -a -m "Bump Spin to ${next_version}"

  git push origin "$backport_branch"

  local title body
  title="[Backport ${release_branch}] wasmtime v${crates_version} + Spin ${next_version} patch release"
  body=$(printf '%s\n' \
    "Backports #${pr_number} to \`${release_branch}\` and prepares the \`${next_version}\` patch release." \
    "" \
    "- Pins wasmtime to the published crates.io release \`v${crates_version}\`." \
    "- Bumps the Spin version to \`${next_version}\`." \
    "" \
    "Once merged, tag \`${next_version}\` on \`${release_branch}\` to trigger the Release workflow." \
    "" \
    "> [!WARNING]" \
    "> This backports the entire wasmtime bump. Confirm that shipping these changes as a patch release is appropriate before merging.")
  gh pr create --base "$release_branch" --head "$backport_branch" --title "$title" --body "$body"
  log "Opened backport/patch PR into ${release_branch}"
}

case "${1:-}" in
  update)
    shift
    cmd_update "$@"
    ;;
  backport)
    shift
    cmd_backport "$@"
    ;;
  *)
    die "usage: $0 {update|backport}"
    ;;
esac
