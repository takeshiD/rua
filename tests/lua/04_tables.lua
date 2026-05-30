-- テーブル: コンストラクタ・配列・ハッシュ・# 演算子・table ライブラリ

-- コンストラクタ
local arr = {10, 20, 30}
print(arr[1], arr[2], arr[3])   --> 10  20  30
print(#arr)                     --> 3

local rec = {name = "Lua", year = 1993}
print(rec.name, rec.year)       --> Lua  1993
print(rec["name"])              --> Lua

-- 混在コンストラクタ
local mix = {1, 2, x = "a", 3, [10] = "ten"}
print(mix[1], mix[2], mix[3])   --> 1  2  3
print(mix.x, mix[10])           --> a  ten

-- 代入・削除（nil 代入）
local t = {}
t[1] = "one"
t.key = "val"
print(t[1], t.key)              --> one  val
t.key = nil
print(t.key)                    --> nil

-- ネストしたテーブル
local nested = {a = {b = {c = 42}}}
print(nested.a.b.c)             --> 42

-- table.insert / remove
local s = {}
table.insert(s, "a")
table.insert(s, "b")
table.insert(s, 1, "first")     -- 位置指定挿入
print(s[1], s[2], s[3], #s)     --> first  a  b  3
local popped = table.remove(s)  -- 末尾を削除して返す
print(popped, #s)               --> b  2
local removed = table.remove(s, 1)
print(removed, s[1], #s)        --> first  a  1

-- table.concat
print(table.concat({1, 2, 3, 4}))       --> 1234
print(table.concat({"a", "b", "c"}, "-"))   --> a-b-c
print(table.concat({"x", "y", "z"}, ",", 2, 3))  --> y,z

-- table.sort
local nums = {5, 3, 8, 1, 9, 2}
table.sort(nums)
print(table.concat(nums, " "))  --> 1 2 3 5 8 9
table.sort(nums, function(a, b) return a > b end)
print(table.concat(nums, " "))  --> 9 8 5 3 2 1

local strs = {"banana", "apple", "cherry"}
table.sort(strs)
print(table.concat(strs, " "))  --> apple banana cherry

-- # の境界（配列部）
local seq = {1, 2, 3, nil}
print(#seq)                     --> 3
