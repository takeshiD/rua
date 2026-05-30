-- string ライブラリのパターンマッチ: find / match / gmatch / gsub

-- string.find: 開始・終了インデックスを返す（1始まり）
print(string.find("hello world", "world"))     --> 7  11
print(string.find("hello", "xyz"))             --> nil
print(string.find("a.b.c", ".", 1, true))      --> 1  1  (plain=true でリテラル検索)
print(string.find("a.b.c", "%."))              --> 2  2  (パターンでドット)

-- キャプチャ付き find
print(string.find("key=value", "(%w+)=(%w+)"))  --> 1  9  key  value

-- string.match: マッチ部分（キャプチャがあればそれ）を返す
print(string.match("hello123world", "%d+"))     --> 123
print(string.match("2026-05-30", "(%d+)-(%d+)-(%d+)"))  --> 2026  05  30
print(string.match("   trim me  ", "^%s*(.-)%s*$"))     --> trim me
print(string.match("nope", "%d+"))              --> nil

-- string.gmatch: 反復子
local words = {}
for w in string.gmatch("the quick brown fox", "%a+") do
    words[#words + 1] = w
end
print(table.concat(words, ","))                 --> the,quick,brown,fox

local sum = 0
for n in string.gmatch("1 22 333", "%d+") do
    sum = sum + tonumber(n)
end
print(sum)                                      --> 356

-- string.gsub: 置換（置換後文字列と置換回数を返す）
print(string.gsub("hello world", "o", "0"))     --> hell0 w0rld  2
print(string.gsub("hello world", "o", "0", 1))  --> hell0 world  1
print(string.gsub("abc", "%w", "%0%0"))         --> aabbcc  3
print(string.gsub("a1b2c3", "(%a)(%d)", "%2%1")) --> 1a2b3c  3

-- 関数による gsub 置換
print((string.gsub("abc", "%a", function(c) return c:upper() end)))  --> ABC

-- テーブルによる gsub 置換
print((string.gsub("$name lives in $city", "%$(%w+)", {name="Sam", city="NY"})))  --> Sam lives in NY

-- アンカーとクラス
print(string.match("Hello", "^%u"))             --> H
print(string.gsub("a,b;c", "[,;]", "/"))        --> a/b/c  2
