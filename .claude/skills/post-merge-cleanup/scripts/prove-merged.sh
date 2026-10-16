#!/usr/bin/env bash
# Proves a local branch's work reached master through a merged pull request, so deleting the branch
# loses nothing. PRs are squash-merged, so master never holds the branch's own commits and
# `git branch -d` cannot tell. This compares the branch with the head of the PR that merged it.
#
# Usage: prove-merged.sh <branch>, from any checkout of the repository.
# Exits 0 and names the PR when the branch is proven merged. Exits 1 with the reason otherwise.
set -euo pipefail

branch=${1:?usage: prove-merged.sh <branch>}

tip=$(git rev-parse --verify --quiet "refs/heads/$branch^{commit}") || {
  echo "$branch: no such local branch"
  exit 1
}

# gh lists the newest PR first, so a branch name reused for a later PR is judged against that one.
prs=$(gh pr list --head "$branch" --state all --json number,state,headRefOid \
  --jq '.[] | "\(.number) \(.state) \(.headRefOid)"')

open=$(awk '$2 == "OPEN" { printf "#%s ", $1 }' <<<"$prs")
if [[ -n $open ]]; then
  echo "$branch: PR ${open% } is still open"
  exit 1
fi

read -r number _ head < <(awk '$2 == "MERGED"' <<<"$prs") || true
if [[ -z ${number:-} ]]; then
  echo "$branch: no merged PR${prs:+ (found: $(awk '{ printf "#%s %s ", $1, $2 }' <<<"$prs"))}"
  exit 1
fi

if [[ $tip == "$head" ]]; then
  echo "$branch: merged in #$number, and the local tip is the PR head"
  exit 0
fi

# The tip differs when the PR was rebased or amended after this copy of the branch was made. That
# is safe as long as every local commit has a counterpart in the PR that changes no line the PR's
# version leaves alone. Counterparts still differ in context lines, hunk positions and conflict
# resolutions once master has moved underneath them, and range-diff shows those as expected noise.
git fetch --quiet origin "pull/$number/head"
pairing=$(git range-diff --no-color "$tip...$head")

# Commit lines pad their counters with leading spaces once either side reaches double digits.
# `<` marks a local commit the PR has no counterpart for.
unmatched=$(grep -E '^ *[0-9]+: +[0-9a-f]+ <' <<<"$pairing" || true)
# range-diff indents each patch line and prefixes it with `-` when only the local commit carries it.
# A local-only `+` or `-` is a line the local side adds or removes and the PR does not.
local_only=$(grep -E '^    -[-+]' <<<"$pairing" || true)

if [[ -z $unmatched && -z $local_only ]]; then
  echo "$branch: merged in #$number, which carries every change the local commits make"
  exit 0
fi

echo "$branch: merged in #$number, but holds work the PR does not"
if [[ -n $unmatched ]]; then
  echo "local commits with no counterpart in the PR:"
  echo "$unmatched"
fi
if [[ -n $local_only ]]; then
  echo "lines only the local commits change:"
  head -n 20 <<<"$local_only"
fi
exit 1
