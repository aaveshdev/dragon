#!/bin/sh
set -e
D="$(cd "$(dirname "$0")" && pwd)"; RES=$(clang -print-resource-dir)
CF="--target=riscv64-unknown-elf -march=rv64imafdc -mabi=lp64d -mcmodel=medany -ffreestanding -fno-builtin -fno-stack-protector -nostdinc -isystem $RES/include -I $D/include -I $D -O2 -w -DDGN_BAREMETAL"
O=""
for s in "$D/riscv64/start.S" "$D/riscv64/board.c" "$D/riscv64/kernel.c" "$D/riscv64/fb.c" "$D/riscv64/input.c" "$D/riscv64/disk.c" "$D/gfx.c" "$D/console.c" "$D/cursor.c" "$D/libc.c" "$1"; do
  o="/tmp/dgn_bm_$(basename "$s").o"; clang $CF -c "$s" -o "$o"; O="$O $o"
done
ld.lld -T "$D/riscv64/link.ld" -o "$2" $O
