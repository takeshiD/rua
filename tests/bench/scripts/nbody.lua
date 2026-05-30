-- 浮動小数演算中心のループ（簡易積分）。VM の算術ディスパッチを測る。
local function run(steps)
  local x, y, vx, vy = 1.0, 0.0, 0.0, 1.0
  local dt = 0.001
  for _ = 1, steps do
    local r2 = x * x + y * y
    local inv_r3 = 1.0 / (r2 * math.sqrt(r2))
    vx = vx - x * inv_r3 * dt
    vy = vy - y * inv_r3 * dt
    x = x + vx * dt
    y = y + vy * dt
  end
  return x, y
end

local x, y = run(2000000)
print(string.format("x=%.6f y=%.6f", x, y))
