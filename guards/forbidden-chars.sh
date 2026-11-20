#!/bin/sh
# Fail if any project file contains a forbidden typographic character.
# Characters are written as UTF-8 byte escapes so this file passes its own check.
#   U+2013 en dash, U+2014 em dash
#   U+00B7, U+318D, U+30FB middle dots
#   U+3008..U+300F angle and corner brackets
pattern=$(printf '%b' \
  '\0342\0200\0223|\0342\0200\0224|\0302\0267|\0343\0206\0215|\0343\0203\0273' \
  '|\0343\0200\0210|\0343\0200\0211|\0343\0200\0212|\0343\0200\0213' \
  '|\0343\0200\0214|\0343\0200\0215|\0343\0200\0216|\0343\0200\0217')

if git grep -nI --untracked -E "$pattern" -- .; then
  echo "forbidden-chars: remove en/em dashes, middle dots, and angle/corner brackets" >&2
  exit 1
fi
