#!/bin/sh
# Exit 0 when every case passes, 1 with the failures on stdout otherwise.
failed=0
check() {
  got=$(./add.sh "$1" "$2")
  if [ "$got" != "$3" ]; then
    echo "FAIL: ./add.sh $1 $2 printed $got, want $3"
    failed=1
  fi
}
check 2 3 5
check 10 -4 6
[ "$failed" = 0 ] && echo "all tests pass"
exit "$failed"
