---
name: post-merge-cleanup
description: Tidy up after a branch's PR has merged — remove the worktree it was checked out in, delete the branch, bring master up to date, and reinstall dependencies if the main checkout moved. Sweeps every merged worktree when run from the main checkout, and leaves unrelated work in the main checkout on its own branch. Use when the user says a branch, PR or worktree is merged, done, or landed and wants to reset or delete it, or asks to find out which worktrees or branches have already merged and clear them out.
allowed-tools: Bash
---

# Post-merge cleanup — reset for the next piece of work

The main checkout and every worktree share one repository, so a merged branch can be checked out in
either. Clean up each merged branch and the worktree it lives in, but leave the main checkout on
whatever it is doing unless that is the merged branch itself.

Run git with absolute paths (`git -C <dir>`). A worktree's directory is deleted partway through, so
a shell sitting inside it would lose its working directory. The exception is a session isolated in
a worktree (one entered with `EnterWorktree`): it refuses git commands aimed at the main checkout.
There, run steps 1–4 from inside the worktree without `-C`, and the rest once step 5 has returned
the session to the main checkout.

The shell may be zsh, which does not split an unquoted variable into words. `set -- $pair` and
`git="git -C <main>"; $git branch` each hand over one mangled argument, so write every command out
in full.

## 1. Work out where you are

```bash
git rev-parse --show-toplevel --git-dir --git-common-dir
git branch --show-current
```

- **In a worktree** when `--git-dir` and `--git-common-dir` differ. The main checkout is the parent
  of `--git-common-dir`. The worktree's path is `--show-toplevel`.
- **In the main checkout** otherwise.

## 2. Find the merged branches

```bash
git fetch --prune origin
git for-each-ref --format='%(refname:short) %(upstream:track)' refs/heads
git worktree list --porcelain
```

- **The user named a branch, PR or worktree:** clean up that one.
- **In a worktree:** its branch.
- **In the main checkout:** every local branch whose upstream reads `[gone]`, whether it is checked
  out in the main checkout, in a worktree, or nowhere, **plus** every branch a linked worktree has
  checked out. Work done in a worktree is often pushed without an upstream, so it never reads
  `[gone]`, and those worktrees are what this cleanup exists to clear. The porcelain listing pairs
  each worktree path with its `branch`, which tells you which worktrees go with it. A worktree
  marked `prunable` has already lost its directory, and the prune in step 5 clears it.

Both only nominate a branch. A branch whose PR closed unmerged reads `[gone]` too, and a worktree
may hold work still in review, so step 3 decides. Check a branch the user names even if it isn't
listed. If nothing turns up, ask.

### Start branches

A worktree that Claude Code creates starts on a branch named `worktree-<directory name>`. A session
that then switches to a branch of its own leaves that start branch behind, pointing where the
worktree began. Collect each one that no worktree has checked out. From the main checkout that means
every such branch. Otherwise it means only the start branches of the worktrees you are removing.

```bash
git merge-base --is-ancestor <start-branch> origin/master
```

Success means it holds no commit master lacks, so it needs no PR and step 7 deletes it. Any other
result means it carries work of its own, so put it through step 3 like any other branch.

## 3. Prove each branch is merged before deleting anything

PRs here are squash-merged, so master never holds the branch's own commits and `git branch -d`
always refuses. Run the bundled proof instead, one branch per call, from the base directory printed
when this skill loaded:

```bash
bash <skill base directory>/scripts/prove-merged.sh <branch>
```

It exits 0 when the branch's newest PR is `MERGED` and nothing local is missing from it. That
covers two cases:

- **The local tip is the PR head.**
- **The PR was rebased or amended after this copy of the branch was made.** This is common: a
  branch stacked on an earlier PR gets rebased onto master once that PR squash-merges. The script
  pairs each local commit with its counterpart in the PR and passes only when none is unmatched
  and none adds or removes a line the PR's version doesn't. Context lines, hunk positions and
  conflict resolutions still differ once master has moved underneath, and those don't count.

Otherwise it exits 1 and says why: no merged PR, a PR still open, a local commit with no
counterpart, or the lines only the local side changes. That branch drops out of the cleanup, along
with its worktree. Tell the user what the script reported and carry on with the rest. Do not force
anything.

## 4. Look for uncommitted work in each worktree

```bash
git -C <worktree> status --short
```

Any output means removing that worktree would destroy work, so leave it and its branch alone and
ask. Ignored files such as `node_modules` and build output never show here. They are rebuilt from
the repository, so losing them is fine.

## 5. Remove the worktrees

- **The worktree this session entered with `EnterWorktree`:** call `ExitWorktree` with
  `action: "remove"`. It deletes the directory and the branch, and returns the session to the main
  checkout. A squash merge never puts the branch's own commits on master, so the first call refuses
  and lists them as unmerged. Step 3 proved they are the merged PR's, and the user asking for this
  cleanup is the confirmation it wants, so call it again with `discard_changes: true`. If the
  refusal lists uncommitted files instead, stop and ask, because step 4 missed them.
- **Any other worktree,** including one an earlier session created:

  ```bash
  git -C <main> worktree remove <worktree-path>
  git -C <main> worktree prune
  ```

  Use no `--force`. If git refuses, it found changes that step 4 missed, so stop and ask.

## 6. Update master

```bash
git -C <main> branch --show-current
git -C <main> status --short
```

- **The main checkout is on master, or on a branch being cleaned up:** move it to master. Its
  uncommitted changes come along, and are usually unrelated work in progress. Compare
  `git -C <main> diff --name-only master origin/master` with the changed files. If they overlap,
  stop and ask. If they don't, the pull fast-forwards around them, so leave them alone and mention
  them in the report.

  ```bash
  git -C <main> checkout master
  git -C <main> pull --ff-only
  ```

- **The main checkout is on any other branch:** that is the user's work in progress, so leave it
  checked out and untouched. Fast-forward master without checking it out:

  ```bash
  git -C <main> fetch origin master:master
  ```

  Git refuses this while master is checked out in some worktree. In that case run
  `git -C <that-worktree> pull --ff-only` instead.

If a fast-forward fails, master has diverged. Stop and report it. Do not rebase or reset.

## 7. Delete the branches

```bash
git -C <main> branch -D <branch>
```

Delete the start branches step 2 collected in the same command. Skip any branch `ExitWorktree`
already deleted. `-D` is safe only because step 3 proved the branch's work is in the merged PR, or
step 2 proved a start branch is already on master. Git refuses to delete a branch that a worktree
still has checked out, which means step 5 left that worktree behind.

GitHub deletes the remote branch on merge, and the `--prune` fetch has already dropped its tracking
ref. If `git ls-remote --heads origin <branch>` still lists it, mention that and leave it alone.

## 8. Reinstall dependencies if the main checkout moved

Run this only when step 6 checked out or pulled master in a checkout:

```bash
cd <that-checkout> && npm install
```

The merge may have changed the lockfile, so this brings `node_modules` back in line with master.
Warnings that list packages whose install scripts were skipped are expected. Only a non-zero exit
is a failure. When the main checkout stayed on another branch, skip this step: its `node_modules`
still matches that branch's own lockfile.

## 9. Report

Give a short summary:

- each PR number, with its deleted branch and removed worktree, and whether it passed as a rebase
- the start branches deleted
- any branch or worktree left alone, and why
- whether the main checkout moved to master or stayed on its own branch
- the commits master moved to
- whether `npm install` ran and succeeded
- any uncommitted changes carried over
