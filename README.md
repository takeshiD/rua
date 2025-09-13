# rua
rua is a lua implementation by Rust.

# Comparison mlua

## Embedding in C/C++
- Using original API implemented C
- `rua` provide C/C++ API

## Embedding in Rust
- `mlua` provides Lua API, but need several dependencies.
- `rua` provides Lua API


## Version and Features
| Lua version and dialect | `mlua`    | `rua`   |
| ----------------------- | --------  | ------- |
| `lua51`                 | Supported |         |
| `lua52`                 | Supported |         |
| `lua53`                 | Supported |         |
| `lua54`                 | Supported |         |
| `luajit`                | Supported |         |
| `luajit52`              | Supported |         |
| `luau`                  | Supported |         |


| Feature for emebedding in Rust | `mlua`    | `rua`   |
| ------------------------------ | --------  | ------- |
| `async/await`                  | Supported |         |
| `send`                         | Supported |         |
| `error-send`                   | Supported |         |
| `serde`                        | Supported |         |
| `macros`                       | Supported |         |
| `anyhow`                       | Supported |         |
| `userdata-wrappers`            | Supported |         |

| Embedding in C/C++ | 


# Similar project
- [mlua](https://github.com/mlua-rs/mlua)
- [rlua](https://github.com/mlua-rs/rlua.git)
