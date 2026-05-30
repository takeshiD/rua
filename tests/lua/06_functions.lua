-- 関数・クロージャ・可変長引数・多値返却・再帰・末尾呼び出し

-- 基本の関数定義と呼び出し
local function add(a, b) return a + b end
print(add(3, 4))                --> 7

-- 引数不足は nil, 余剰は捨てられる
local function f(a, b) return a, b end
print(f(1))                     --> 1  nil
print(f(1, 2, 3))               --> 1  2

-- 多値返却と代入
local function minmax(t)
    local lo, hi = t[1], t[1]
    for i = 2, #t do
        if t[i] < lo then lo = t[i] end
        if t[i] > hi then hi = t[i] end
    end
    return lo, hi
end
local lo, hi = minmax({3, 1, 4, 1, 5, 9, 2, 6})
print(lo, hi)                   --> 1  9

-- 多値が途中だと 1 値に切り詰め, 末尾だと展開
local function two() return 10, 20 end
print(two(), 99)                --> 10  99   (途中の two() は 1 値)
print(99, two())                --> 99  10  20  (末尾は展開)
local list = {two(), two()}     -- 最初は1値, 最後は展開
print(#list)                    --> 3
print(list[1], list[2], list[3])    --> 10  10  20

-- クロージャ（上位値のキャプチャ）
local function counter()
    local n = 0
    return function()
        n = n + 1
        return n
    end
end
local c = counter()
print(c(), c(), c())            --> 1  2  3
local c2 = counter()
print(c2(), c())                --> 1  4   (各クロージャは独立した n)

-- 可変長引数 ...
local function sum(...)
    local s = 0
    for _, v in ipairs({...}) do
        s = s + v
    end
    return s
end
print(sum(1, 2, 3, 4, 5))       --> 15
print(sum())                    --> 0

local function count(...)
    return select("#", ...)
end
print(count(1, nil, 3))         --> 3   (select '#' は nil 込みの個数)

local function first(...)
    return (select(1, ...))
end
print(first("a", "b", "c"))     --> a

-- 再帰
local function fact(n)
    if n <= 1 then return 1 end
    return n * fact(n - 1)
end
print(fact(5))                  --> 120

local function fib(n)
    if n < 2 then return n end
    return fib(n - 1) + fib(n - 2)
end
print(fib(10))                  --> 55

-- 末尾呼び出し（深い再帰でもスタックを伸ばさない）
local function loop(n, acc)
    if n == 0 then return acc end
    return loop(n - 1, acc + n)     -- 末尾呼び出し
end
print(loop(100000, 0))          --> 5000050000

-- 相互再帰
local isEven, isOdd
function isEven(n) if n == 0 then return true else return isOdd(n - 1) end end
function isOdd(n) if n == 0 then return false else return isEven(n - 1) end end
print(isEven(10), isOdd(10))    --> true  false
