# Building the Dragon language

This project is the Dragon compiler, virtual machine, standard library and the bare-metal SDK. 

## Tools needed

| Tool | Used for |
|---|---|
| Rust (stable, with `cargo`) | building the compiler |
| `gcc` or `clang` | `--native` builds (Dragon is translated to C and compiled) |
| `clang` and `ld.lld` | `--baremetal` builds |
| `qemu-system-x86_64`, `qemu-system-riscv64` | running and testing bare-metal images |

## Build

```sh
cargo build --release
./target/release/dragon --help
```

The standard packages and the bare-metal kernel sources (`baremetal/`) are embedded in the compiler, so rebuild it after
changing them. To keep the build output somewhere else, set `CARGO_TARGET_DIR`.

## Use

```sh
dragon file.dgn                                  # run on the virtual machine
dragon -c file.dgn -o file --native              # compile to a native executable
dragon -c file.dgn -o image.elf --native --baremetal x86_64      # a bootable image (or riscv64)
dragon -c file.dgn -o prog.dxe --native --baremetal dxe          # a ring-3 program for an OS that loads .dxe files
```

Bare-metal options: `--sdk DIR` overlays your own copies of the embedded kernel sources, `--embed FILE` adds a file to the
image (it is read back with the `blob` module), `--qemu` runs the image in QEMU.


The suite covers the language (VM and native must agree), the example applications with a platform mock, the project
scaffolding for Android, iOS and Windows, and, when `qemu-*`, `clang` and `ld.lld` are present, the bare-metal targets on
x86-64 and RISC-V (output equals the host's, interactive shell, preemptive tasks, garbage collection). Steps whose tools are
missing are skipped and said so. The operating-system tests live in the Vaelis project.
