# Whale Toolchain

Whale is a general-purpose low-level toolchain written in Rust. It is designed as a modern, lightweight backend for the **Wave** programming language.

## Capability status

| Area | Status | Notes |
|---|---|---|
| Assembler (`whale asm`) | **Implemented** | AMD64 only. Output must be an ELF object (`.o`). |
| Object CLI (`whale object`) | **Implemented** | Build ELF objects from binary / IR inputs. |
| Linker (`whale link`) | **Planned** | CLI stub only — prints "coming soon", no resolution or relocation yet. |
| IR tools (`whale ir`) | **Experimental** | Requires building with `--features socket-cli`. Subcommand: `ir lower`. |

Defined-behavior goals for a full native link path are tracked separately; this README only documents what the current tree actually runs.

## Key components

- **Whale Assembler (`asm`)** — AMD64 assembler with multi-section support, standard directives (`global`, `section`, `extern`), and ELF relocation generation into `.o` objects.
- **Whale Object (`object`)** — Object-file library and CLI for sections, symbols, and ELF64 generation.
- **Whale Linker (`linker`)** — Not implemented yet (placeholder CLI).
- **Whale IR (`ir`)** — Experimental lower/print/verify demos behind the `socket-cli` feature.

## Project philosophy

1. **Modular** — Every component is a reusable Rust crate.
2. **Transparent** — Built from scratch to avoid opaque legacy toolchain layers.
3. **Performant** — Uses Rust's memory safety and zero-cost abstractions.

## Getting started

### Installation

```bash
# Core CLI (asm / object / link stub)
cargo build --release

# Include experimental IR CLI (needed for `whale ir`)
cargo build --release --features socket-cli
```

Install the binary from `target/release/whale`, or run via `cargo run -- ...`.

### Basic usage

#### 1. Assemble source (ELF object)

```bash
whale asm --amd64 input.asm -o output.o
```

Raw `.bin` output is **not** supported by the current assembler CLI; it rejects non-`.o` outputs.

#### 2. Create / inspect object files

```bash
whale object input.bin -o output.o
```

#### 3. Link objects (not available yet)

```bash
whale link obj1.o obj2.o -o executable
# Currently prints a stub message only.
```

#### 4. IR lower (feature-gated)

```bash
cargo run --features socket-cli -- ir lower program.json -o out.wir
```

## Documentation

Detailed CLI notes live under `docs/cli`:

- [Assembler CLI](docs/cli/asm.md) — note: some prose there still describes `.bin` output; the live `asm` command requires `.o`.
- [Object CLI](docs/cli/object.md)

## License

This project is licensed under the MPL-2.0 License — see the [LICENSE](LICENSE) file for details.
