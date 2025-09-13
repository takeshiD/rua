# rua

**rua** is a Lua implementation written in Rust.  
It aims to provide a safe, modern, and flexible Lua runtime and tooling, without depending on the original C libraries.

## Features

- **Lua API for Rust**
- **Lua API for C/C++**
- **Lua Interpreter (`rua`)** with REPL and version switching
- **Lua Compiler (`ruac`)** with bytecode disassembly and version switching

# Usage

## Lua Interpreter (`rua`)

The `rua` command provides a Lua interpreter with a rich REPL experience and version switching.

### REPL
```sh
$ rua
In[0]: print("hello rua")
Out[0]: hello rua
```

#### Magic command
##### `ls` : list files in current directory
```
In[0]: %ls
Out[0]: .gitignore README.md function.lua ...
```

##### `who`: show current variables
```
In[1]: %who
Out[1]: a b c my_function
```


### Run a script
```sh
$ rua script.lua
```

### Switch Lua version
#### Run with Lua 5.1
```
rua --lua51 script.lua
```

#### Run with LuaJIT
```
rua --luajit script.lua
```




# Comparison mlua

## Embedding in C/C++
- Using original API implemented C
- `rua` provide C/C++ API

## Embedding in Rust
- `mlua` provides Lua API, but depend on several C Library.
- `rua` provides Lua API, **no depend on C Library*

## Performance
not yet measured

## Version and Features
| Lua version and dialect | `mlua`    | `rua`   |
| ----------------------- | --------  | ------- |
| `lua51`                 | Supported | Planned |
| `lua52`                 | Supported | Planned |
| `lua53`                 | Supported | Planned |
| `lua54`                 | Supported | Planned |
| `luajit`                | Supported | Planned |
| `luajit52`              | Supported | Planned |
| `luau`                  | Supported | Planned |



| Feature for emebedding in Rust | `mlua`    | `rua`   |
| ------------------------------ | --------  | ------- |
| `async/await`                  | Supported | Planned |
| `send`                         | Supported | Planned |
| `error-send`                   | Supported | Planned |
| `serde`                        | Supported | Planned |
| `macros`                       | Supported | Planned |
| `anyhow`                       | Supported | Planned |
| `userdata-wrappers`            | Supported | Planned |



# Inspired
- [mlua](https://github.com/mlua-rs/mlua)
- [rlua](https://github.com/mlua-rs/rlua.git)

