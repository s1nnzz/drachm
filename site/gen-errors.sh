#!/bin/sh
# Regenerates site/content/errors.md: one section per code in src/diag/codes.rs.
# Each section is the output of `drachm explain CODE`, so the page and the
# command always say the same thing. Run it after changing the code table:
#
#   site/gen-errors.sh
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
cargo build --quiet
drachm="$root/target/debug/drachm"
out="$root/site/content/errors.md"

codes=$(grep -oE '^    [EW][0-9]{4}:' src/diag/codes.rs | tr -d ' :')

{
    echo "---"
    echo "title: Error index"
    echo "description: Every error and warning code the compiler knows, with its explanation."
    echo "---"
    echo
    echo "# Error index"
    echo
    echo "Every error and warning code, generated from \`src/diag/codes.rs\` by"
    echo "\`site/gen-errors.sh\`. The same text is printed by \`drachm explain CODE\`."
    echo "Do not edit this page by hand."
    echo
    for code in $codes; do
        # The first line of `explain` is "CODE: title"; the rest is the text.
        "$drachm" explain "$code" | awk '
            NR == 1 { title = $0; sub(/^[^:]*: /, "", title); print "## " SUBCODE ": " title; print ""; next }
            NR == 2 && $0 == "" { next }
            { print }
        ' SUBCODE="$code"
        echo
    done
} > "$out"

echo "wrote $out ($(echo "$codes" | wc -l | tr -d ' ') codes)"
