-- メタテーブル: __index/__newindex/算術/__eq/__lt/__le/__call/__tostring/__concat/__len

-- __index（テーブル）でデフォルト値・継承
local base = {greet = "hello", kind = "base"}
local obj = setmetatable({kind = "obj"}, {__index = base})
print(obj.kind)                 --> obj   (自分の値が優先)
print(obj.greet)                --> hello (__index 経由)
print(rawget(obj, "greet"))     --> nil   (生アクセスは無視)

-- __index（関数）
local computed = setmetatable({}, {__index = function(_, k) return "key:" .. k end})
print(computed.foo)             --> key:foo
print(computed[42])             --> key:42

-- __newindex（関数）でアクセス記録
local log = {}
local guarded = setmetatable({}, {
    __newindex = function(t, k, v)
        log[#log + 1] = k .. "=" .. tostring(v)
        rawset(t, k, v)
    end
})
guarded.a = 1
guarded.b = 2
guarded.a = 3                   -- 既存キーは __newindex を呼ばない
print(table.concat(log, ","))   --> a=1,b=2
print(guarded.a)                --> 3

-- 算術メタメソッド（ベクトル）
local Vec = {}
Vec.__index = Vec
Vec.__add = function(u, v) return setmetatable({u[1] + v[1], u[2] + v[2]}, Vec) end
Vec.__sub = function(u, v) return setmetatable({u[1] - v[1], u[2] - v[2]}, Vec) end
Vec.__mul = function(u, k) return setmetatable({u[1] * k, u[2] * k}, Vec) end
Vec.__unm = function(u) return setmetatable({-u[1], -u[2]}, Vec) end
Vec.__tostring = function(u) return "(" .. u[1] .. "," .. u[2] .. ")" end
Vec.__eq = function(u, v) return u[1] == v[1] and u[2] == v[2] end
local function vec(x, y) return setmetatable({x, y}, Vec) end

print(tostring(vec(1, 2) + vec(3, 4)))      --> (4,6)
print(tostring(vec(5, 5) - vec(1, 2)))      --> (4,3)
print(tostring(vec(2, 3) * 10))             --> (20,30)
print(tostring(-vec(1, 2)))                 --> (-1,-2)
print(vec(1, 2) == vec(1, 2))               --> true
print(vec(1, 2) == vec(1, 3))               --> false

-- __lt / __le（順序）
local mt_ord = {
    __lt = function(a, b) return a.v < b.v end,
    __le = function(a, b) return a.v <= b.v end,
}
local function box(v) return setmetatable({v = v}, mt_ord) end
print(box(1) < box(2))          --> true
print(box(2) < box(1))          --> false
print(box(2) <= box(2))         --> true

-- __call（呼び出し可能テーブル）
local adder = setmetatable({base = 100}, {
    __call = function(self, x) return self.base + x end
})
print(adder(5))                 --> 105

-- __concat（左右どちらがメタ持ちでも呼ばれる）
local Str = {}
Str.__tostring = function(s) return s.val end
Str.__concat = function(a, b)
    local function s(x) return type(x) == "table" and x.val or tostring(x) end
    return s(a) .. s(b)
end
local function wrap(v) return setmetatable({val = v}, Str) end
print(wrap("foo") .. "bar")     --> foobar
print("baz" .. wrap("qux"))     --> bazqux
print(tostring(wrap("hi")))     --> hi   (__tostring)

-- __len（5.1 では文字列/テーブルにのみ作用。テーブルの __len は 5.1 だと # に効かないが
-- rawlen 相当の挙動確認のため getmetatable で存在のみ検証）
print(getmetatable("a string").__index == string)  --> true  (文字列のメタテーブル)
