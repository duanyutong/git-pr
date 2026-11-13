#!/bin/sh
set -eu

if [ "$#" -ne 1 ] || [ -z "$1" ]; then
    printf 'Usage: ./scripts/man.sh BINARY\nGenerates git-pr.1 from the supplied git-pr executable.\n' >&2
    exit 2
fi

# git-pr writes usage to stdout and options to stderr; retain both before formatting.
help_text=$(NO_COLOR=1 CLICOLOR_FORCE=0 "$1" --help 2>&1) || {
    help_status=$?
    printf '%s\n' "$help_text" >&2
    exit "$help_status"
}
printf '%s\n' "$help_text" | awk '
    function literal(line, output, i, character) {
        output = "\\&"
        for (i = 1; i <= length(line); i++) {
            character = substr(line, i, 1)
            if (character == "\\") output = output "\\e"
            else if (character == "-") output = output "\\-"
            else output = output character
        }
        return output
    }
    BEGIN {
        print ".TH GIT-PR 1\n.SH NAME"
        print "git\\-pr \\- publish Git branches and stacked GitHub pull requests"
    }
    /^Usage: / {
        print ".SH SYNOPSIS\n.nf"
        print literal(substr($0, 8))
        print ".fi"
        next
    }
    /^(Arguments|Options):$/ {
        section = $0
        sub(/:$/, "", section)
        print ".SH " toupper(section)
        next
    }
    /^COMMITS may be:$/ {
        print ".SH ARGUMENTS"
        next
    }
    /^  [^ ]|^      -/ {
        line = $0
        sub(/^[ \t]+/, "", line)
        if (line ~ /^-/ && !options_started) {
            print ".SH OPTIONS"
            options_started = 1
        }
        print ".TP\n.B"
        if (match(line, /  +|\t+/)) {
            heading = substr(line, 1, RSTART - 1)
            description = substr(line, RSTART + RLENGTH)
            print literal(heading)
            if (description != "") print literal(description)
        } else {
            print literal(line)
        }
        next
    }
    NF {
        sub(/^[ \t]+/, "")
        print literal($0)
    }
    END { print ".SH SEE ALSO\n.BR git (1)" }
'
