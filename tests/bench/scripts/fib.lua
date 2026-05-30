-- 再帰フィボナッチ: 関数呼び出し/再帰のコスト測定。
local function fib(n)
  if n < 2 then return n end
  return fib(n - 1) + fib(n - 2)
end

local N = 32
local r = fib(N)
print("fib(" .. N .. ") = " .. r)
