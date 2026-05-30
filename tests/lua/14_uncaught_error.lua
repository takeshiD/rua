-- 捕捉されないエラーでの終了コード検証。
-- 本家 lua5.1 はエラーメッセージを stderr に出し、終了コード 1 で終了する。
-- stdout（エラー前の print）と終了コード(1) を比較対象とする。
-- （stderr はパス・行番号を含むため厳密比較しない。サイドカー .exitcode=1 を参照。）

print("before error")           --> before error（stdout に出る）
print("still running")          --> still running

error("intentional failure")    -- ここで停止。終了コード 1。

print("never reached")          -- 実行されない
