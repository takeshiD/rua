-- 反復: ipairs / pairs / next / select
-- 注意: pairs のハッシュ部の列挙順は未規定。決定的にするためソートしてから出力する。

-- ipairs は 1 から連続する整数キーを順に
local arr = {"a", "b", "c"}
for i, v in ipairs(arr) do
    print(i, v)
end

-- ipairs は最初の nil で停止
local sparse = {10, 20, nil, 40}
local seen = 0
for _ in ipairs(sparse) do seen = seen + 1 end
print("ipairs count", seen)     --> ipairs count  2

-- pairs で全キーを列挙（キーをソートして決定的に出力）
local t = {x = 1, y = 2, z = 3}
local keys = {}
for k in pairs(t) do keys[#keys + 1] = k end
table.sort(keys)
print(table.concat(keys, ","))  --> x,y,z

-- pairs は配列部とハッシュ部の両方を回す（個数で確認）
local mixed = {10, 20, 30, name = "foo", flag = true}
local n = 0
for _ in pairs(mixed) do n = n + 1 end
print("pairs count", n)         --> pairs count  5

-- next による手動反復（空テーブルは nil）
print(next({}))                 --> nil
local single = {only = 1}
local k, v = next(single)
print(k, v)                     --> only  1
print(next(single, "only"))     --> nil   (最後の次は nil)

-- select("#", ...) と select(n, ...)
print(select("#"))              --> 0
print(select("#", 1, 2, 3))     --> 3
print(select("#", nil, nil))    --> 2
print(select(2, "a", "b", "c")) --> b  c
print(select(3, "a", "b", "c")) --> c

-- ipairs/pairs を使った合計
local data = {5, 10, 15, 20}
local total = 0
for _, v in ipairs(data) do total = total + v end
print(total)                    --> 50

-- ジェネリック for は任意の反復子関数で動く（カスタム range）
local function range(n)
    local i = 0
    return function()
        i = i + 1
        if i <= n then return i end
    end
end
local collected = {}
for x in range(5) do collected[#collected + 1] = x end
print(table.concat(collected, " "))     --> 1 2 3 4 5
