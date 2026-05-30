-- 制御構文: if/elseif/else, while, repeat, 数値for, ジェネリックfor, break

-- if / elseif / else
local function classify(n)
    if n < 0 then
        return "negative"
    elseif n == 0 then
        return "zero"
    else
        return "positive"
    end
end
print(classify(-5), classify(0), classify(7))   --> negative  zero  positive

-- while
local i = 1
local acc = 0
while i <= 5 do
    acc = acc + i
    i = i + 1
end
print(acc)                      --> 15

-- repeat ... until（後判定, until のスコープはブロック内変数を見られる）
local j = 0
repeat
    j = j + 1
until j >= 3
print(j)                        --> 3

-- 数値 for（上り）
local total = 0
for k = 1, 10 do
    total = total + k
end
print(total)                    --> 55

-- 数値 for（ステップ指定・下り）
local out = {}
for k = 10, 1, -2 do
    out[#out + 1] = k
end
print(table.concat(out, " "))   --> 10 8 6 4 2

-- 数値 for（小数ステップ）
local fracs = {}
for x = 0, 1, 0.5 do
    fracs[#fracs + 1] = x
end
print(table.concat(fracs, " ")) --> 0 0.5 1

-- ジェネリック for + ipairs
local fruits = {"apple", "banana", "cherry"}
for idx, name in ipairs(fruits) do
    print(idx, name)
end

-- break
local found
for k = 1, 100 do
    if k * k > 50 then
        found = k
        break
    end
end
print(found)                    --> 8

-- ネストしたループと break（内側のみ抜ける）
local pairs_count = 0
for a = 1, 3 do
    for b = 1, 3 do
        if b == 2 then break end
        pairs_count = pairs_count + 1
    end
end
print(pairs_count)              --> 3
