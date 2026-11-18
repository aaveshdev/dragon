# Dragon on bare metal

`dragon -c prog.dgn -o prog.elf --native --baremetal riscv64|x86_64 [--qemu]`

The same C that `--native` produces is compiled with `-ffreestanding -nostdinc` against a tiny
libc (`libc.c` and the headers in `include/`) and a board directory (`riscv64/` for QEMU `virt`, `x86_64/` for QEMU `pc`). No operating system is involved: `_start` sets up the stack and FPU, clears
BSS and calls `main`. Output goes to the NS16550 UART, input comes from it (`readln`), `exit()`
powers the machine off.

Needs `clang` (with the RISC-V / x86-64 target), `ld.lld`, and `qemu-system-riscv64` / `qemu-system-x86_64` to run it.

What works (checked by `tests/baremetal.sh`, `tests/baremetal_shell.py`): the language itself,
lists/maps/strings/classes, try/catch (own `setjmp`), the garbage collector (conservative stack
scan), JSON, floats and `printf`-style formatting, and `readln` from the serial port.

What does not: files, sockets, threads, regex, FFI and `time` beyond a simple clock all fail
cleanly (stubs in `libc.c`). `math` functions are compact series implementations, so the last
digits can differ from a full libm.

Adding a target = a `<arch>/` directory and a case in `src/codegen_c/baremetal.rs`. Portable parts are shared:
`sched.c` (tasks, mutexes, GC support), `input_hl.c` (key names / pointer from raw events), `gfx.c`, `console.c`,
`cursor.c`, `libc.c`. A target supplies `start.S` + `link.ld` (boot, setjmp/longjmp), `kernel.c` (`dgn_arch_*`: task
frames, tick timer, yield, interrupt entry), `board.c` (UART, clock, power-off, RAM end), `fb.c` (ramfb), `input.c`
(raw input events), `disk.c` (block device).

| | RISC-V 64 (`riscv64`) | x86-64 (`x86_64`) |
|---|---|---|
| QEMU machine | `virt`, `-bios none` | `pc`, boots via the PVH ELF note (32-bit entry, switches to long mode itself) |
| console | NS16550 | COM1 |
| timer / clock | CLINT (10 ms tick) | PIT 100 Hz tick, HPET clock |
| framebuffer | `ramfb` via fw_cfg MMIO | `ramfb` via fw_cfg I/O ports |
| keyboard / mouse | virtio-input (MMIO) | PS/2 (i8042) |
| disk | virtio-blk (MMIO) | ATA PIO (IDE) |
| exit | sifive_test | `isa-debug-exit` / ACPI power-off |

QEMU command lines used by the tests are in `tests/bm_arch.py` and `tests/dos/qemu_dos.py`. aarch64 is not
included: no `qemu-system-aarch64` was available to test it, and an untested port would be a guess.

## Hardware access and multitasking (`import hw`, `import task`)

Bare-metal builds have two extra built-in modules (not available in the interpreter or other builds).

`hw`: `stack_free()`, `peek8/16/32/64(addr)`, `poke8/16/32/64(addr, value)` (volatile device access; a bad address
traps and prints a diagnostic), `micros()`, `millis()`, `ticks()` (10 ms timer ticks), `putc(c)`,
`heap_used()`, `heap_peak()`, `halt(code)`.

`task`: `spawn(fn[, arg])` returns an id (1, 2, ...; the main flow is 0), `sleep(ms)`, `pause()`
(yield), `id()`, `alive(id)`, `count()`, `wait(id)`, `atomic(fn)`.

How it works: a machine-timer interrupt fires every 10 ms. Every task has its own stack. A trap saves
all registers in a frame on the task's stack, and the C handler returns the frame to resume, which is
the context switch. Tasks are switched away only at safe points (loop iterations and conditions in
generated code, `sleep`, `pause`), never in the middle of a runtime operation, so list/map updates,
allocation and the collector never see half-finished work from another task. The collector scans every
task's stack. Each task has its own try/catch stack. Memory allocation and console output are
critical sections. A compute loop with no loop or condition at all is not interrupted.

Known limits: user data shared between tasks is not protected beyond `task.atomic` (statements
like `n = n + 1` can interleave at loop boundaries); an exception thrown out of `task.atomic` leaves
preemption blocked; 64 tasks, 64 KB stack each by default (`task.spawn(fn, arg, stack_kb)`); single core. The boot stack is 4 MB.

## Writing an operating system on this target

The bare-metal target is a generic SDK: the kernel in C (`baremetal/`) is embedded in the compiler, and programs written in
Dragon use these modules: `hw` (memory peek/poke, port I/O including `insw`/`outsw`, `alloc`, clock), `con` (text console),
`gfx` (drawing, back buffer, blit, glass/blur/rounded-rectangle primitives), `input` (`init`, `poll`, `cursor`, `inject`: raw
events from drivers written in Dragon), `disk` (the built-in ATA/virtio block device), `task` (preemptive multitasking), `proc`
(ring-3 processes, x86-64 only) and `blob` (files embedded in the image).

`--sdk DIR` overlays your own copy of any of the embedded C sources, and `--embed FILE` adds a file to the image. An operating
system built this way is a separate project: Vaelis (see its own repository and `BUILD.md`) is the one this language was used for.
`--baremetal dxe` builds a ring-3 program in the `.dxe` format that such a system can load.

Attach devices on QEMU: a disk (`-drive file=disk.img,if=ide,format=raw` on x86-64; riscv64 `-drive file=disk.img,if=none,format=raw,id=d0
-device virtio-blk-device,drive=d0`), a network card (`-netdev user,id=n0 -device virtio-net-pci,netdev=n0`) and USB
(`-device piix3-usb-uhci,id=uhci -device usb-kbd,bus=uhci.0`).

Verified on QEMU only; never run on real hardware.
