-- io.write と os の決定的な部分のみ
-- （os.time/os.clock は非決定的なので型・範囲のみ検証。os.date は UTC 固定時刻で検証。）

-- io.write は改行を付けない（複数呼び出しが連結される）
io.write("a")
io.write("b", "c")
io.write("\n")
io.write("num=", 42, "\n")      -- 数値は文字列化される
print("after-io")               --> after-io

-- print と io.write の違い: print はタブ区切り＋改行
io.write("x", "y", "z", "\n")   --> xyz （区切り無し）
print("x", "y", "z")            --> x  y  z （タブ区切り）

-- os.time は数値
print(type(os.time()))          --> number

-- os.clock は数値（CPU 時間）
print(type(os.clock()))         --> number

-- os.date を UTC 固定エポックで（"!" は UTC 指定）
print(os.date("!%Y-%m-%d", 0))      --> 1970-01-01
print(os.date("!%H:%M:%S", 0))      --> 00:00:00
print(os.date("!%Y-%m-%d %H:%M:%S", 86400))     --> 1970-01-02 00:00:00

-- os.getenv の戻り（存在しない変数は nil）
print(os.getenv("RUA_DEFINITELY_NOT_SET_12345"))    --> nil

-- os.difftime
print(os.difftime(100, 40))     --> 60
