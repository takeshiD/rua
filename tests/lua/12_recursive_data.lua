-- 再帰的データ構造: 連結リスト・二分木・スタック/キュー

-- 連結リスト
local function cons(head, tail) return {head = head, tail = tail} end
local function from_array(arr)
    local list = nil
    for i = #arr, 1, -1 do
        list = cons(arr[i], list)
    end
    return list
end
local function list_sum(list)
    if list == nil then return 0 end
    return list.head + list_sum(list.tail)
end
local function list_to_string(list)
    local parts = {}
    while list do
        parts[#parts + 1] = tostring(list.head)
        list = list.tail
    end
    return table.concat(parts, "->")
end

local l = from_array({1, 2, 3, 4, 5})
print(list_to_string(l))        --> 1->2->3->4->5
print(list_sum(l))              --> 15

-- 二分探索木
local function insert(node, value)
    if node == nil then
        return {value = value, left = nil, right = nil}
    end
    if value < node.value then
        node.left = insert(node.left, value)
    else
        node.right = insert(node.right, value)
    end
    return node
end
local function inorder(node, out)
    if node == nil then return end
    inorder(node.left, out)
    out[#out + 1] = node.value
    inorder(node.right, out)
end

local tree = nil
for _, v in ipairs({5, 3, 8, 1, 4, 7, 9, 2, 6}) do
    tree = insert(tree, v)
end
local sorted = {}
inorder(tree, sorted)
print(table.concat(sorted, " "))    --> 1 2 3 4 5 6 7 8 9

-- スタック（LIFO）
local stack = {}
local function push(x) stack[#stack + 1] = x end
local function pop() local x = stack[#stack]; stack[#stack] = nil; return x end
push(10); push(20); push(30)
print(pop(), pop(), pop())      --> 30  20  10

-- 深いネスト構造のアクセス
local deep = {a = {b = {c = {d = {e = "found"}}}}}
print(deep.a.b.c.d.e)           --> found

-- 自己参照テーブル（循環）— 識別のみ（出力は固定）
local node = {name = "self"}
node.me = node
print(node.me.me.me.name)       --> self

-- メモ化付き再帰（フィボナッチ）
local memo = {}
local function mfib(n)
    if n < 2 then return n end
    if memo[n] then return memo[n] end
    memo[n] = mfib(n - 1) + mfib(n - 2)
    return memo[n]
end
print(mfib(30))                 --> 832040
