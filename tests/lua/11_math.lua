-- math ライブラリの主要関数（決定的な値のみ）

-- 定数
print(math.pi)                  --> 3.1415926535898  ("%.14g")
print(math.huge)                --> inf
print(-math.huge)               --> -inf

-- 丸め
print(math.floor(3.7))          --> 3
print(math.floor(-3.2))         --> -4
print(math.ceil(3.2))           --> 4
print(math.ceil(-3.7))          --> -3
print(math.floor(5))            --> 5

-- 絶対値・符号系
print(math.abs(-7))             --> 7
print(math.abs(7))              --> 7
print(math.abs(-3.5))           --> 3.5

-- 最大・最小
print(math.max(1, 5, 3, 9, 2))  --> 9
print(math.min(1, 5, 3, 9, 2))  --> 1
print(math.max(-1, -5))         --> -1

-- べき乗・平方根
print(math.sqrt(16))            --> 4
print(math.sqrt(2))             --> 1.4142135623731
print(math.pow(2, 10))          --> 1024
print(math.sqrt(144))           --> 12

-- 三角関数（割り切れる代表値）
print(math.sin(0))              --> 0
print(math.cos(0))              --> 1
print(math.tan(0))              --> 0

-- 指数・対数
print(math.exp(0))              --> 1
print(math.log(1))              --> 0
print(math.log10(1000))         --> 3
print(math.log10(100))          --> 2

-- modf / fmod
print(math.modf(3.75))          --> 3  0.75
print(math.fmod(7, 3))          --> 1
print(math.fmod(-7, 3))         --> -1   (fmod は被除数の符号、% とは異なる)

-- 整数⇄浮動の境界
print(math.floor(2.999999))     --> 2
print(math.ceil(2.000001))      --> 3

-- math.huge と比較
print(math.huge > 1e308)        --> true
print(1 / 0 == math.huge)       --> true

-- random は非決定的なので範囲のみ検証
math.randomseed(0)
local r = math.random()
print(r >= 0 and r < 1)         --> true
local ri = math.random(1, 6)
print(ri >= 1 and ri <= 6)      --> true
print(math.type and "has type" or "no type")    --> no type  (5.1 に math.type は無い)
