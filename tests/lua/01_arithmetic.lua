-- 算術・演算子優先順位・比較・論理・連結
-- Lua 5.1 では全数値が double。print/tostring は "%.14g" 形式。
-- 出力が非循環小数になるよう、意図的に割り切れる値を使う。

-- 基本算術
print(1 + 2)            --> 3
print(10 - 4)           --> 6
print(6 * 7)            --> 42
print(20 / 5)           --> 4
print(20 / 8)           --> 2.5
print(2 ^ 10)           --> 1024
print(17 % 5)           --> 2
print(-17 % 5)          --> 3   (Lua の % は被除数と異符号でも床演算)
print(5.5 % 2)          --> 1.5
print(-(3 + 4))         --> -7

-- 演算子優先順位: ^ は右結合かつ単項マイナスより強い
print(2 ^ 3 ^ 2)        --> 512   (2^(3^2))
print(-2 ^ 2)           --> -4    (-(2^2))
print(2 + 3 * 4)        --> 14
print((2 + 3) * 4)      --> 20
print(2 * 3 + 4 * 5)    --> 26
print(10 - 2 - 3)       --> 5     (左結合)

-- 比較演算子（結果は boolean）
print(1 < 2, 2 < 1)             --> true   false
print(2 <= 2, 3 <= 2)          --> true   false
print(3 > 2, 2 > 3)            --> true   false
print(3 >= 3, 2 >= 3)         --> true   false
print(1 == 1, 1 == 2)          --> true   false
print(1 ~= 2, 1 ~= 1)          --> true   false
print("abc" == "abc")           --> true
print("abc" < "abd")            --> true   (辞書順)
print("Z" < "a")                --> true   (大文字は小さい)

-- 論理演算子（and/or は値を返す, 短絡評価）
print(true and "yes")           --> yes
print(false and "yes")          --> false
print(nil and 1)                --> nil
print(false or "default")       --> default
print(1 or 2)                   --> 1
print(nil or false)             --> false
print(not true, not false)      --> false  true
print(not nil, not 0)           --> true   false   (0 は真)

-- 文字列連結 .. （数値は自動で文字列化）
print("a" .. "b" .. "c")        --> abc
print("n=" .. 42)               --> n=42
print(1 .. 2 .. 3)              --> 123
print("pi~" .. 3.5)             --> pi~3.5
