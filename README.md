# rua

A Lua 5.1 interpreter written in Rust, aiming for complete compatibility with PUC-Rio Lua 5.1.

> [日本語版はこちら](README.ja.md)

## Features

- **Complete Lua 5.1 language support** — all syntax, operators, metatables, closures, varargs, multiple returns, tail calls
- **Register-based bytecode VM** — the same architecture as the reference PUC-Rio implementation
- **Standard library** — `base`, `string`, `table`, `math`, `io`, `os` (see [status](#standard-library-status))
- **Rich interactive REPL** — syntax highlighting, tab completion, persistent history, multi-line continuation
- **`luac`-equivalent compiler** — bytecode listing, compile-only syntax check, chunk output
- **Shell completions** — bash, zsh, fish, elvish, powershell
- **C API layer** — `lua.h` ABI-compatible `extern "C"` functions (cdylib / staticlib)
- **Safe Rust embedding API** — ergonomic high-level API in the style of `mlua` / `rlua`
- **Garbage collector** — arena-based mark-and-sweep (no `unsafe` required)

## Installation

### Build from source

Requires Rust **1.96** or later (stable).

```bash
git clone https://github.com/takeshiD/rua
cd rua
cargo build --release
```

The binary is placed at `target/release/rua`.

To install into `~/.cargo/bin`:

```bash
cargo install --path crates/rua-cli
```

## Quick Start

```bash
# Run a script
rua script.lua

# Run with arguments (accessible as arg[1], arg[2], ... and ...)
rua script.lua foo bar

# Read from stdin
echo 'print("hello, world")' | rua -

# Start the interactive REPL
rua
```

## CLI Reference

### `rua run` — Execute a script

```bash
rua run script.lua [args...]
rua run -               # read from stdin
```

Script arguments are available as `arg[0]`, `arg[1]`, ... and through `...` in the main chunk — the same convention as the official `lua5.1` binary.

### `rua` (no subcommand) — Shorthand

```bash
rua script.lua [args...]   # same as rua run
rua                        # launch REPL
```

### `rua repl` — Interactive REPL

```bash
rua repl
```

| Key | Action |
|-----|--------|
| `Tab` | Show completions |
| `Enter` | Execute (or continue if block is incomplete) |
| `Ctrl-C` | Cancel current input |
| `Ctrl-D` | Exit REPL |

Expressions are evaluated and their values printed automatically (`1+2` → prints `3`).
History is saved to `~/.local/share/rua/history.txt`.

### `rua luac` — Compiler

```bash
rua luac -p script.lua              # syntax check only (no output on success)
rua luac -l script.lua              # list bytecode instructions
rua luac -ll script.lua             # list bytecode + constants, locals, upvalues
rua luac -o out.rbc script.lua      # compile to file (rua bytecode format)
rua luac -s -o out.rbc script.lua   # strip debug info
rua run out.rbc                     # execute compiled chunk
```

### `rua completions` — Shell completions

```bash
# bash
rua completions bash >> ~/.bashrc

# zsh
rua completions zsh > ~/.zfunc/_rua
# Ensure fpath=(~/.zfunc $fpath) and autoload -U compinit are in ~/.zshrc

# fish
rua completions fish > ~/.config/fish/completions/rua.fish
```

## Standard Library Status

| Library | Status | Notes |
|---------|--------|-------|
| `base` | ✅ Complete | `print`, `type`, `tostring`, `tonumber`, `pairs`, `ipairs`, `next`, `select`, `error`, `assert`, `pcall`, `xpcall`, `rawget`, `rawset`, `rawequal`, `setmetatable`, `getmetatable`, `unpack`, `_G`, `_VERSION` |
| `string` | ✅ Complete | All functions including full pattern engine (`find`, `match`, `gmatch`, `gsub`) |
| `table` | ✅ Complete | `insert`, `remove`, `concat`, `sort`, `maxn` |
| `math` | ✅ Complete | All trigonometric, exponential, rounding, and random functions |
| `io` | ✅ Complete | `io.open`, `io.close`, `io.read`, `io.write`, `io.lines`, `io.flush`, `io.input`, `io.output`, `io.type`, `io.stdin/stdout/stderr`, all `file:*` methods |
| `os` | 🔶 Partial | `os.time`, `os.date`, `os.clock`, `os.exit` implemented; `os.execute`, `os.getenv`, `os.remove`, `os.rename` pending |
| `debug` | ❌ Not yet | Planned |
| `package` / `require` | ❌ Not yet | Planned |
| `coroutine` | ❌ Not yet | Planned |

### Known limitations

- `s:upper()` string method syntax requires the shared string metatable, which is not yet wired; use `string.upper(s)` instead.
- `error(msg, level)` error position prefix is approximate (CallInfo lacks pc/line mapping).

## Architecture

```
rua/
├── crates/
│   ├── rua-core/        # Lexer → Parser → Codegen → VM → GC · stdlib · Rust API
│   ├── rua-cli/         # Standalone interpreter binary (rua run, repl, luac)
│   └── rua-capi/        # C API layer — lua.h ABI-compatible cdylib + staticlib
├── tests/
│   ├── lua/             # 15 golden test scripts (compared against lua5.1 output)
│   └── lua-suite/       # PUC-Rio official test suite integration
├── fuzz/                # cargo-fuzz targets (compile_only, compile_run)
└── docs/
    ├── ARCHITECTURE.md  # Design decisions, GC strategy, development phases
    └── CONFORMANCE.md   # Testing strategy, golden harness, reference management
```

### Crate responsibilities

| Crate | Role | Lua 5.1 equivalent |
|-------|------|--------------------|
| `rua-core` | Everything except the CLI front-end | `llex.c`, `lparser.c`, `lcode.c`, `lvm.c`, `lgc.c`, `lstate.c`, `ldo.c`, `lib*.c` |
| `rua-cli` | `rua` binary, REPL, luac | `lua.c`, `luac.c` |
| `rua-capi` | `extern "C"` ABI layer | `lapi.c`, `lauxlib.c` |

### Value model

```
Lua type          Rust representation
─────────────────────────────────────
nil               Value::Nil
boolean           Value::Boolean(bool)
number            Value::Number(f64)      ← Lua 5.1 has only double
string            Value::GcRef(GcHandle::Str(_))   ← interned
table             Value::GcRef(GcHandle::Table(_))
function          Value::GcRef(GcHandle::Closure(_))
userdata          Value::GcRef(GcHandle::Userdata(_))
lightuserdata     Value::LightUserData(*mut c_void)
```

GC objects live in typed arenas ([slotmap](https://docs.rs/slotmap)); `Value` holds a generational index — no `unsafe` required for GC traversal.

## Running the Tests

```bash
# Unit + integration tests
cargo test --workspace

# Validate golden .expected files against reference lua5.1
# (requires lua5.1 installed: apt install lua5.1)
cargo test -p rua-cli -- --ignored validate_expected_against_reference

# PUC-Rio official test suite (fetched from lua.org)
tests/lua-suite/fetch.sh
cargo test -p rua-cli --test official_suite -- --ignored --nocapture

# Fuzz (requires nightly + cargo-fuzz)
cargo +nightly fuzz run compile_only -- -max_total_time=60
cargo +nightly fuzz run compile_run  -- -max_total_time=60
```

## Embedding in Rust

```rust
use rua_core::{LuaState, stdlib};

let mut state = LuaState::new();
stdlib::open_libs(&mut state);

// Register a native function
state.register("add", |state| {
    let a = state.check_number(1)?;
    let b = state.check_number(2)?;
    state.push_number(a + b);
    Ok(1)
});

// Execute Lua code
state.do_string("print(add(1, 2))")?;
```

## Embedding in C / C++

Include the bundled headers and link against `librua_capi`:

```c
#include "lua.h"
#include "lauxlib.h"
#include "lualib.h"

int main(void) {
    lua_State *L = luaL_newstate();
    luaL_openlibs(L);
    luaL_dostring(L, "print('hello from C')");
    lua_close(L);
    return 0;
}
```

```bash
# static link
gcc main.c -Icrates/rua-capi/include \
    target/release/librua_capi.a -lpthread -ldl -lm -o demo
```

## Compatibility

`rua` targets **Lua 5.1** (the same version embedded in Neovim, Redis, World of Warcraft add-ons, and many other applications).

Intentional non-goals:
- LuaJIT extensions (`bit`, `ffi`, `jit`)
- Lua 5.2+ features (`goto`, integer subtype, bitwise operators, etc.)
- JIT compilation

## Contributing

```bash
# Check formatting and lints (same as CI)
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI runs on **Rust stable** (`dtolnay/rust-toolchain@stable`). Make sure your local toolchain is up to date (`rustup update stable`) to avoid version-drift failures.

## License

MIT — see [LICENSE](LICENSE).
