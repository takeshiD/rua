-- coroutine 基本テスト

-- 1. create + resume + status
local co = coroutine.create(function(a, b)
    return a + b
end)
print(coroutine.status(co))         -- suspended
local ok, v = coroutine.resume(co, 10, 20)
print(ok, v)                        -- true  30
print(coroutine.status(co))         -- dead

-- 2. yield / resume で値を双方向に受け渡し
local co2 = coroutine.create(function(x)
    local y = coroutine.yield(x * 2)
    return y + 1
end)
local ok2, v2 = coroutine.resume(co2, 5)
print(ok2, v2)                      -- true  10
local ok3, v3 = coroutine.resume(co2, 100)
print(ok3, v3)                      -- true  101
print(coroutine.status(co2))        -- dead

-- 3. dead コルーチンを再 resume するとエラー
local ok4, err4 = coroutine.resume(co2)
print(ok4, err4)                    -- false  cannot resume dead coroutine

-- 4. 複数値 yield
local co3 = coroutine.create(function()
    coroutine.yield(1, 2, 3)
    coroutine.yield(4, 5)
    return 6
end)
local _, a, b, c = coroutine.resume(co3)
print(a, b, c)                      -- 1  2  3
local _, d, e = coroutine.resume(co3)
print(d, e)                         -- 4  5
local _, f = coroutine.resume(co3)
print(f)                            -- 6

-- 5. coroutine.wrap
local gen = coroutine.wrap(function()
    for i = 1, 3 do
        coroutine.yield(i)
    end
end)
print(gen())                        -- 1
print(gen())                        -- 2
print(gen())                        -- 3

-- 6. wrap で dead になった後は error
gen()                               -- 4回目: コルーチン本体が正常終了して dead になる
local ok5, err5 = pcall(gen)
print(ok5)                          -- false

-- 7. ネストした関数呼び出しから yield
local function inner(n)
    return coroutine.yield(n)
end
local co4 = coroutine.create(function()
    local r = inner(42)
    return r * 2
end)
local _, yv = coroutine.resume(co4)
print(yv)                           -- 42
local _, rv = coroutine.resume(co4, 7)
print(rv)                           -- 14

-- 8. type check
print(type(co))                     -- thread
print(type(co) == "thread")         -- true
