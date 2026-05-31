-- 257個以上のハッシュフィールドを持つテーブルコンストラクタのテスト
-- MAXINDEXRK(=255) を超える定数インデックスが RK spill を経由して正しく動作することを確認
local t = {}
for i = 1, 300 do
  t["k" .. i] = i
end
print(t.k257)
print(t.k300)
