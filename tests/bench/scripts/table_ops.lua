-- テーブル操作: 挿入・参照・ソートのコスト。
local t = {}
for i = 1, 100000 do
  t[i] = (i * 2654435761) % 1000003
end

table.sort(t)

local sum = 0
for i = 1, #t do
  sum = sum + t[i]
end
print("n=" .. #t .. " sum=" .. sum .. " first=" .. t[1] .. " last=" .. t[#t])
