#!/usr/bin/env bash
# install-git-stash-wrapper.sh — Hardening (4) PATH wrapper for git stash
# blocking in dispatched agent worktrees.
#
# Authorized by supervisor 2026-05-28 after 3/3 same-shape violations of
# the stash absolute-binding (LSP-B, LSP-C, joint-fix-1a) — all the same
# push+pop cycle that slipped hardening (3)'s commit-time hook.
#
# Per supervisor binders:
#   (1) per-dispatched-worktree only, not host-wide. Host git stays
#       unaffected; the user uses stash legitimately for interactive work.
#   (2) dispatch-template extension — agent's first action installs the
#       wrapper at $PATH-prepend-dir AND confirms `which git` resolves to
#       the wrapper before any other action.
#   (3) wrapper bypass forbidden — no env-var escape hatch.
#   (4) Q3-on-self at first dispatch — team-lead runs the wrapper in an
#       actual dispatched worktree before the parallel-5 round.
#
# Usage:
#   scripts/install-git-stash-wrapper.sh <branch-name>
#
# Installs to /tmp/shape-<branch>-bin/git. Caller (team-lead OR agent) is
# responsible for `export PATH=/tmp/shape-<branch>-bin:$PATH`.

set -euo pipefail

if [ $# -ne 1 ]; then
    echo "usage: $0 <branch-name>" >&2
    exit 2
fi

BRANCH="$1"
BINDIR="/tmp/shape-${BRANCH}-bin"
REAL_GIT="/run/current-system/sw/bin/git"

if [ ! -x "$REAL_GIT" ]; then
    echo "ERROR: real git not found at $REAL_GIT" >&2
    exit 3
fi

mkdir -p "$BINDIR"

cat > "$BINDIR/git" <<EOF
#!/bin/sh
# Hardening (4) PATH wrapper — blocks 'git stash' in dispatched agent
# worktree '$BRANCH'. Per supervisor 2026-05-28 authorization after
# 3/3 same-shape stash-binding violations (LSP-B, LSP-C, joint-fix-1a).
#
# Bypass FORBIDDEN. Agents needing transient state use 'git commit -m WIP'
# to a private branch.
if [ "\$1" = "stash" ]; then
    cat >&2 <<BLOCK

═══════════════════════════════════════════════════════════════════════
GIT STASH BLOCKED — Hardening (4) PATH wrapper
═══════════════════════════════════════════════════════════════════════

\\\`git stash\\\` is forbidden in dispatched agent worktrees per CLAUDE.md
ABSOLUTE BINDING (supervisor 2026-05-23 / 2026-05-24 / 2026-05-28).

This wrapper at $BINDIR/git intercepts the stash subcommand and refuses
to proceed. There is no bypass mechanism. The host user's git is
unaffected (this wrapper only fires when PATH includes $BINDIR/git
BEFORE the host git).

To recover (transient state for investigation):
  1. git commit -m "WIP: <reason>"   — on this worktree's branch
  2. continue investigation
  3. git reset HEAD~1 --soft         — to undo the WIP commit if no
                                       longer needed (keeps changes)

State-recovery for individual files:
  - git checkout -- <file>           — discard unstaged changes
  - git reset HEAD <file>            — unstage

═══════════════════════════════════════════════════════════════════════
BLOCK
    exit 1
fi
exec $REAL_GIT "\$@"
EOF

chmod +x "$BINDIR/git"

echo "Installed: $BINDIR/git → wrapper around $REAL_GIT"
echo "Activate:  export PATH=$BINDIR:\$PATH"
echo "Verify:    which git  (must print '$BINDIR/git')"
