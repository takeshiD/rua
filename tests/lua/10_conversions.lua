-- 型と変換: type / tostring / tonumber

-- type
print(type(nil))                --> nil
print(type(true))               --> boolean
print(type(42))                 --> number
print(type("s"))                --> string
print(type({}))                 --> table
print(type(print))              --> function
print(type(type))               --> function

-- tostring（数値の整形は "%.14g"）
print(tostring(nil))            --> nil
print(tostring(true))           --> true
print(tostring(false))          --> false
print(tostring(42))             --> 42
print(tostring(3.5))            --> 3.5
print(tostring(-0.25))          --> -0.25
print(tostring(100))            --> 100
print(tostring(1000000))        --> 1000000
print(tostring("already"))      --> already

-- tonumber（10進）
print(tonumber("42"))           --> 42
print(tonumber("3.14"))         --> 3.14
print(tonumber("  10  "))       --> 10   (前後空白は許容)
print(tonumber("-7"))           --> -7
print(tonumber("1e3"))          --> 1000
print(tonumber("0x1F"))         --> 31   (16進リテラル)
print(tonumber("nope"))         --> nil
print(tonumber("12abc"))        --> nil
print(tonumber(""))             --> nil
print(tonumber(true))           --> nil  (boolean は変換不可)
print(tonumber(42))             --> 42   (数値はそのまま)

-- tonumber（基数指定）
print(tonumber("FF", 16))       --> 255
print(tonumber("777", 8))       --> 511
print(tonumber("101", 2))       --> 5
print(tonumber("z", 36))        --> 35
print(tonumber("10", 2))        --> 2

-- 数値リテラルの形式
print(0xff)                     --> 255
print(1e2)                      --> 100
print(.5)                       --> 0.5
print(3.0)                      --> 3    ("%.14g" で整数値は小数点なし)

-- 数値の文字列強制（算術文脈では文字列→数値）
print("10" + 5)                 --> 15
print("3" * "4")                --> 12
print(10 .. "")                 --> 10   (連結文脈では数値→文字列)
