# rua
rua is a lua implementation by Rust.

- provide Lua API for Rust
- provide Lua API for C/C++
- provide Lua Interpreter `rua`
    - Pretty REPL like IPyhton
        - pretty display
        - magic commands  
        ```sh
        In[0]: %ls
        Out[0]: .gitignore README.md function.lua ...
        In[1]: %who
        Out[1]: 
        ```
    - Easy switch lua version(default latest lua : currently lua54)  
        > Specified Lua51  `rua --lua51 file.lua`  
        > Specified LuaJIT `rua --luajit file.lua`  
- provide Lua compiler `ruac`
    - Easy switch lua version(default latest lua : currently lua54)  
        > Specified Lua51  `ruac --lua51 file.lua -o file.out`  
        > Specified LuaJIT `ruac --luajit file.lua -o file1.out`  
    - pretty display undump  
        > `ruac -l(--list)` / `ruac -ll(--full-list)`  

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



# Similar project
- [mlua](https://github.com/mlua-rs/mlua)
- [rlua](https://github.com/mlua-rs/rlua.git)
