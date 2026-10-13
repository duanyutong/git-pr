#!/bin/sh
# Backend headers supply identity; message files are data and never shell source.
set -eu
edited="$1.git-pr-edited"
trap 'rm -f "$edited"' EXIT HUP INT TERM
awk '
BEGIN {
    directory = ENVIRON["GIT_PR_EDIT_DIR"]
    while ((getline entry < (directory "/map")) > 0) {
        split(entry, fields, " ")
        messages[fields[1]] = fields[2]
        for (size = 1; size <= length(fields[1]); size++) {
            prefix = substr(fields[1], 1, size)
            identities[prefix] = fields[1]
            multiplicity[prefix]++
        }
        expected++
    }
}
/^\+\+ reword / || /^JJ: describe / {
    prefix = $3
    chosen = identities[prefix]
    if (multiplicity[prefix] != 1 || seen[chosen]++) { invalid = 1; exit 1 }
    count++
    print $0
    path = directory "/" messages[chosen]
    while ((read_status = (getline line < path)) > 0) print line
    if (read_status < 0) { invalid = 1; exit 1 }
    close(path)
    print ""
}
END { if (invalid || count != expected) exit 1 }
' "$1" > "$edited"
mv "$edited" "$1"
