#!/bin/bash
# wingbar catalog widget: git-branch
#
# Shows the checked-out branch of one git repository (or the short commit
# when HEAD is detached), and whether it has uncommitted changes.
#
# Which repository: the first argument, else WINGBAR_GIT_DIR, e.g.
#   "command": "bash ~/.config/wingbar/widgets/git-branch.sh ~/code/myproject"
# With neither, the widget hides.
#
# Levels (line 1 is "level|label"):
#   ok     clean: the branch name
#   warn   uncommitted changes (staged, unstaged or untracked files)
#   dim    the folder is not a git repository: "not a repo"
#   (empty line: no repository set, or git is missing; the widget hides)
#
# Menu: the repository's name, the number of changed files, commits ahead of
# and behind the upstream, then "Open in Finder" and "Open in Terminal".
#
# Needs: git (Xcode Command Line Tools or Homebrew). It reads the repository
# only (with --no-optional-locks, so it never blocks your own git commands);
# no network (it does not fetch), no secrets.
# Settings (environment, optional):
#   WINGBAR_GIT_DIR   the repository, when no argument is given
#   WINGBAR_GIT       the git binary to use
# Suggested interval: 5s.

GIT=${WINGBAR_GIT:-git}
DIR=${1:-${WINGBAR_GIT_DIR:-}}
case $DIR in "~") DIR=$HOME ;; "~/"*) DIR="$HOME/${DIR#"~/"}" ;; esac

if [ -z "$DIR" ] || ! command -v "$GIT" >/dev/null 2>&1; then
	echo ""
	exit 0
fi
# /usr/bin/git without the Command Line Tools would ask to install them
if [ "$(command -v "$GIT")" = /usr/bin/git ] && ! /usr/bin/xcode-select -p >/dev/null 2>&1; then
	echo ""
	exit 0
fi

g() {
	"$GIT" --no-optional-locks -C "$DIR" "$@" 2>/dev/null
}

# Text for the label or a menu title: without "|" (in a menu line it would end
# the title and start a command) and control characters (a line break would
# start a line of its own).
plain() {
	printf '%s' "$1" | tr -d '|[:cntrl:]'
}

TOP=$(g rev-parse --show-toplevel)
if [ -z "$TOP" ]; then
	echo "dim|not a repo"
	echo "menu: $(plain "$DIR") is not a git repository"
	exit 0
fi

BRANCH=$(g symbolic-ref --short -q HEAD)
[ -z "$BRANCH" ] && BRANCH=$(g rev-parse --short HEAD)
[ -z "$BRANCH" ] && BRANCH="(no commits)"
BRANCH=$(plain "$BRANCH")

CHANGED=$(g status --porcelain | awk 'END { print NR }')
LEVEL=ok
[ "${CHANGED:-0}" -gt 0 ] && LEVEL=warn

echo "$LEVEL|$BRANCH"
echo "menu: $(plain "$(basename "$TOP")")"
if [ "${CHANGED:-0}" -eq 1 ]; then
	echo "menu: 1 changed file"
elif [ "${CHANGED:-0}" -gt 1 ]; then
	echo "menu: $CHANGED changed files"
else
	echo "menu: No changes"
fi
# "behind<TAB>ahead" against the upstream, when there is one
COUNTS=$(g rev-list --left-right --count '@{upstream}...HEAD')
if [ -n "$COUNTS" ]; then
	set -- $COUNTS
	echo "menu: $2 ahead, $1 behind upstream"
fi
echo "menu: ---"
printf 'menu: Open in Finder | open %q\n' "$TOP"
printf 'menu: Open in Terminal | open -a Terminal %q\n' "$TOP"
