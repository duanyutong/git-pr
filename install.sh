#!/bin/sh
set -eu

main() {
    if [ "$#" -eq 1 ] && { [ "$1" = --help ] || [ "$1" = -h ]; }; then
        printf 'Usage: ./install.sh [PREFIX]\nInstalls git-pr and its manual under PREFIX (default: ~/.cargo), passed to Cargo as --root.\n'
        return
    fi
    if [ "$#" -gt 1 ] || { [ "$#" -eq 1 ] && [ -z "$1" ]; }; then
        printf 'ERROR: supply at most one non-empty installation prefix.\nUsage: ./install.sh [PREFIX]\n' >&2
        return 2
    fi

    prefix=${1:-"$HOME/.cargo"}
    case "$prefix" in
        /*) ;;
        *) prefix="$PWD/$prefix" ;;
    esac
    source_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
    manual_tmp=$(mktemp)
    trap 'rm -f "$manual_tmp"' EXIT
    trap 'exit 1' HUP INT TERM

    cargo install --path "$source_dir" --locked --root "$prefix"
    if ! sh "$source_dir/scripts/man.sh" "$prefix/bin/git-pr" > "$manual_tmp" ||
        ! install -d "$prefix/share/man/man1" ||
        ! install -m 644 "$manual_tmp" "$prefix/share/man/man1/git-pr.1"; then
        printf 'ERROR: the binary was installed at %s/bin/git-pr, but its manual could not be installed at %s/share/man/man1/git-pr.1.\n' "$prefix" "$prefix" >&2
        return 1
    fi
    printf 'Installed git-pr and its manual under %s.\n' "$prefix"
    printf 'Ensure %s/bin is on PATH. If MANPATH is explicitly set, include %s/share/man.\n' "$prefix" "$prefix"
}

main "$@"
