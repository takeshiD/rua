-- エラー処理: pcall / error / assert / xpcall
--
-- 注意: error の既定 level(1) はメッセージに "ファイル名:行: " を前置するため、
-- 出力がパス・行番号依存になる。ゴールデン比較を決定的にするため、位置情報を
-- 出力したい箇所では string.match で「サフィックスのみ」を検証し真偽値を表示する。
-- 位置情報が不要な箇所は error(msg, 0) を使う。

-- pcall 成功（true と戻り値群）
print(pcall(function() return 1, 2, 3 end))     --> true  1  2  3

-- pcall 失敗: 既定 level の error はメッセージ末尾に元の文字列を含む
local ok, e = pcall(function() error("boom") end)
print(ok, type(e), e:match("boom$") ~= nil)     --> false  string  true

-- error の level 0（位置情報を付けない → 決定的）
print(pcall(function() error("plain", 0) end))  --> false  plain

-- error に非文字列（テーブル）も渡せる（そのまま err として返る）
local ok2, err = pcall(function() error({code = 42}) end)
print(ok2, type(err), err.code)                 --> false  table  42

-- assert（成功時は全引数をそのまま返す）
print(assert(10, "unused"))                     --> 10  unused（第2引数も返る）
print(assert("x", "y"))                         --> x  y

-- assert 失敗: 本家 assert は luaL_error 経由なのでメッセージに位置情報
-- "ファイル名:行: " が前置される（error の level 1 相当）。
-- ハーネスは cwd=tests/lua・相対パスで起動するためチャンク名は basename になる。
print(pcall(function() assert(false, "custom message") end))    --> false  08_errors.lua:LINE: custom message
print(pcall(function() assert(nil) end))        --> false  08_errors.lua:LINE: assertion failed!

-- ランタイムエラー（nil の算術 / nil の呼び出し）
local rok, rmsg = pcall(function() return 1 + nil end)
print(rok, type(rmsg))                          --> false  string
print((pcall(function() local x = nil; return x() end)))    --> false

-- pcall のネスト（内側 level 0 で決定的に）
print(pcall(function()
    return pcall(function() error("inner", 0) end)
end))                                           --> true  false  inner

-- xpcall（メッセージハンドラ経由）
local function handler(msg) return "handled: " .. msg end
print(xpcall(function() error("oops", 0) end, handler))     --> false  handled: oops
print(xpcall(function() return "ok" end, handler))          --> true  ok

-- error からの回復後も実行は継続する
local results = {}
for i = 1, 3 do
    pcall(function()
        if i == 2 then error("skip " .. i) end
        results[#results + 1] = i
    end)
end
print(table.concat(results, ","))               --> 1,3
