-- 文字列リテラル・エスケープ・long string・基本操作

-- クォート種別
print('single')                 --> single
print("double")                 --> double
print('he said "hi"')           --> he said "hi"
print("it's ok")                --> it's ok

-- エスケープシーケンス
print("tab\tend")               --> tab<TAB>end
print("line1\nline2")           --> 2 行に分かれる
print("back\\slash")            --> back\slash
print("quote\"q")               --> quote"q
print("bell-len", #"\a")       --> bell-len  1
print("\65\66\67")              --> ABC   (10進エスケープ)
-- 注: Lua 5.1 には \x（16進）エスケープは無いので使わない（5.2 以降の機能）。

-- long string [[ ]]（エスケープ無効・先頭改行は除去）
print([[raw \n not escaped]])   --> raw \n not escaped
print([==[has ]] inside]==])    --> has ]] inside
local multi = [[
line A
line B]]
print(multi)                    --> line A / line B（先頭の改行は1つ除去）

-- 長さ演算子と連結
print(#"hello")                 --> 5
print(#"")                      --> 0
print("foo" .. "bar")           --> foobar

-- string ライブラリ: 基本
print(string.len("hello"))      --> 5
print(string.upper("abcXYZ"))   --> ABCXYZ
print(string.lower("abcXYZ"))   --> abcxyz
print(string.rep("ab", 3))      --> ababab
print(string.reverse("abc"))    --> cba
print(string.sub("hello", 2, 4))    --> ell
print(string.sub("hello", -3))      --> llo
print(string.sub("hello", 2))       --> ello
print(("method"):upper())           --> METHOD   (メソッド構文)
print(string.byte("A"))             --> 65
print(string.byte("ABC", 2))        --> 66
print(string.char(72, 105))         --> Hi
print(string.format("%d/%s/%05.2f", 7, "x", 3.5))  --> 7/x/03.50
print(string.format("%q", 'a"b'))   --> "a\"b"
