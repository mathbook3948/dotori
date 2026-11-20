#!/bin/sh
en_dash='\0342\0200\0223'
em_dash='\0342\0200\0224'
middle_dot='\0302\0267'
hangul_middle_dot='\0343\0206\0215'
katakana_middle_dot='\0343\0203\0273'
angle_brackets='\0343\0200\0210|\0343\0200\0211|\0343\0200\0212|\0343\0200\0213'
corner_brackets='\0343\0200\0214|\0343\0200\0215|\0343\0200\0216|\0343\0200\0217'

forbidden_as_utf8_bytes=$(printf '%b' "$en_dash|$em_dash|$middle_dot|$hangul_middle_dot|$katakana_middle_dot|$angle_brackets|$corner_brackets")

if git grep -nI --untracked -E "$forbidden_as_utf8_bytes" -- . ':!.claude/skills'; then
  echo "forbidden-chars: remove en/em dashes, middle dots, and angle/corner brackets" >&2
  exit 1
fi
