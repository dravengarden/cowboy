#!/usr/bin/env bash
# docs/INDEX.md must link every Markdown page below docs/, and every local link
# in it must resolve. Same contract as the Columbus documentation-index gate,
# kept here so a Cowboy change fails in its own repository first.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
docs="$root/docs"
index="$docs/INDEX.md"

sed -n '2,12p' "$index" | grep -Eq '^type:[[:space:]]*docs_index[[:space:]]*$' \
    || { echo "docs/INDEX.md: front matter must declare type: docs_index" >&2; exit 1; }

declare -A linked=()
status=0
while IFS= read -r link; do
    target="${link#](}"
    target="${target%)}"
    target="${target%%#*}"
    [[ -z "$target" ]] && continue
    case "$target" in
        http://* | https://* | mailto:*) continue ;;
    esac
    if [[ ! -e "$docs/$target" ]]; then
        echo "docs/INDEX.md: broken local link: $target" >&2
        status=1
        continue
    fi
    [[ "$target" == *.md ]] && linked["$(realpath -m -- "$docs/$target")"]=1
done < <(grep -o '\]([^)[:space:]]*)' "$index" || true)

pages=0
while IFS= read -r page; do
    pages=$((pages + 1))
    if [[ -z "${linked[$(realpath -m -- "$page")]+present}" ]]; then
        echo "docs/INDEX.md: Markdown page is not linked: ${page#"$docs"/}" >&2
        status=1
    fi
done < <(cd "$root" && git ls-files -z -- 'docs/*.md' 'docs/**/*.md' | tr '\0' '\n' | grep -v '^docs/INDEX.md$' | sed "s#^#$root/#")

[[ $status -eq 0 ]] && echo "docs index ok: $pages Markdown pages"
exit "$status"
