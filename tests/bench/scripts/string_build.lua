-- 文字列構築: table.concat とパターン処理のコスト。
local parts = {}
for i = 1, 200000 do
  parts[i] = tostring(i)
end
local s = table.concat(parts, ",")

-- gsub で軽い書き換えを一度かける。
local _, n = s:gsub("%d+", "#")
print("len=" .. #s .. " count=" .. n)
