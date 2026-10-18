#!/bin/sh
# Rename the project from its working name rsim to yavs in this working tree.
#
# Idempotent: a second run changes nothing. It also rewrites uncommitted and
# untracked (not ignored) files, so a worktree with work in progress can run it
# before rebasing onto the rename commit on main.
#
# Kept as they are: ~/.cache/rsim-* directory names (they exist on disk),
# ~/git/rsim and ~/git/vunit-rsim checkouts, the VUnit branch names
# feature/rsim and rsim-integration, the strsim crate, lines that contain
# "rename: keep" or "the working name rsim", and the vendored library sources
# (lib/std, lib/ieee, vhpi_user.h).
#
# Usage: tools/rename-yavs.sh   (from anywhere inside the working tree; runs
# cargo fmt afterwards when cargo is on PATH)
set -eu

cd "$(git rev-parse --show-toplevel)"

# Paths: the crate directories first, then every other path that names rsim.
for d in crates/rsim crates/rsim-*; do
    [ -d "$d" ] || continue
    git mv -k "$d" "crates/yavs${d#crates/rsim}"
    if [ -d "$d" ]; then mv "$d" "crates/yavs${d#crates/rsim}"; fi
done
git ls-files -co --exclude-standard | grep 'rsim' | grep -v -e '^lib/std/' -e '^lib/ieee/' |
while IFS= read -r p; do
    [ -e "$p" ] || continue
    n=$(printf '%s\n' "$p" | sed 's/rsim/yavs/g')
    mkdir -p "$(dirname "$n")"
    git mv -k "$p" "$n"
    if [ -e "$p" ]; then mv "$p" "$n"; fi
done

# Contents.
script='
/rename: keep/b
/the working name rsim/b
s/strsim/@@YAVS_KEEP_SIMILARITY@@/g
s#\.cache/rsim#@@YAVS_KEEP_CACHE@@#g
s#\.cache" / "rsim#@@YAVS_KEEP_CACHEPY@@#g
s#git/vunit-rsim#@@YAVS_KEEP_GITVUNIT@@#g
s#git/rsim#@@YAVS_KEEP_GIT@@#g
s#feature/rsim#@@YAVS_KEEP_FEATURE@@#g
s/rsim-integration/@@YAVS_KEEP_INTEGRATION@@/g
s/rsim/yavs/g
s/Rsim/Yavs/g
s/RSIM/YAVS/g
s/@@YAVS_KEEP_SIMILARITY@@/strsim/g
s#@@YAVS_KEEP_CACHE@@#.cache/rsim#g
s#@@YAVS_KEEP_CACHEPY@@#.cache" / "rsim#g
s#@@YAVS_KEEP_GITVUNIT@@#git/vunit-rsim#g
s#@@YAVS_KEEP_GIT@@#git/rsim#g
s#@@YAVS_KEEP_FEATURE@@#feature/rsim#g
s/@@YAVS_KEEP_INTEGRATION@@/rsim-integration/g
'
git ls-files -co --exclude-standard |
    grep -v -e '^lib/std/' -e '^lib/ieee/' -e 'vhpi_user\.h$' -e '^tools/rename-yavs\.sh$' |
while IFS= read -r f; do
    [ -f "$f" ] || continue
    grep -q -I -e rsim -e Rsim -e RSIM "$f" || continue
    sed "$script" "$f" > "$f.yavs-tmp"
    if cmp -s "$f" "$f.yavs-tmp"; then
        rm "$f.yavs-tmp"
    else
        cat "$f.yavs-tmp" > "$f"
        rm "$f.yavs-tmp"
        echo "rewrote $f"
    fi
done

# The new crate names sort differently (yavs_diag after std), so reformat.
if command -v cargo > /dev/null; then
    cargo fmt --all
    cargo fmt --all --manifest-path fuzz/Cargo.toml
fi
