-- tools/probe/tf3/dump_core.lua
--
-- The script-API dump, for Transport Fever 3: the sandbox, number formatting,
-- math.random, pairs() order, the global environment and the api.* tree, of
-- whichever Lua state runs it. The TF3 probe mods carry a copy of this block
-- between the "dump core" marks (tpf3mp_apidump_1 in the GUI state,
-- tpf3mp_rundump_1 in the run script's), and a test keeps the copies equal to
-- this file. It is the TPF2 probe's dump (../script_api_dump), with what TF3's
-- mods showed added: api.cmd.make*Cmd factories, api.gui, ug_require and the
-- whole global table (investigation/TF3_MODS_2026-09-27.md).
--
-- Rules: pcall around every probe; "absent"/"error" instead of a guess;
-- sorted output; nothing written back to the game.
--
-- TPF3MP_DUMP_CORE_BEGIN
local function tpf3mpDump(stateName)
  local out = {}
  local function line(fmt, ...)
    if select("#", ...) == 0 then out[#out + 1] = fmt
    else out[#out + 1] = string.format(fmt, ...) end
  end

  local function sortedKeys(t)
    local keys = {}
    local ok = pcall(function()
      for k in pairs(t) do keys[#keys + 1] = k end
    end)
    if not ok then return nil end
    table.sort(keys, function(a, b) return tostring(a) < tostring(b) end)
    return keys
  end

  -- A global by name, without _G (a state need not have it) and without
  -- raising where undeclared globals raise.
  local function global(name)
    local ok, v = pcall(function()
      local env = _G
      if type(env) == "table" then return env[name] end
      return nil
    end)
    if ok then return v end
    return nil
  end

  local function typeTag(v)
    local t = type(v)
    if t == "function" then return "fn" end
    if t == "userdata" then return "ud" end
    if t == "boolean" then return "bool=" .. tostring(v) end
    if t == "number" then return "num=" .. tostring(v) end
    if t == "string" then return "str" end
    return t
  end

  local function sandbox()
    line("## Sandbox")
    line("_VERSION = %s", tostring(global("_VERSION")))
    local names = { "io", "os", "require", "package", "debug", "load", "loadstring",
      "collectgarbage", "debugPrint", "ug_require", "print", "dofile", "loadfile",
      "setmetatable", "getmetatable", "rawget", "unpack", "bit32", "utf8", "game", "api" }
    local found = {}
    for _, name in ipairs(names) do
      found[#found + 1] = name .. "=" .. type(global(name))
    end
    line("globals: %s", table.concat(found, " "))
    local t = global("table")
    line("dialect markers: table.unpack=%s goto(compiles)=%s integer division(//)=%s",
      type(t) == "table" and type(t.unpack) or "?",
      (function()
        local mk = global("load") or global("loadstring")
        if type(mk) ~= "function" then return "no-load" end
        local ok, f = pcall(mk, "::x:: goto x")
        return (ok and f) and "yes" or "no"
      end)(),
      (function()
        local mk = global("load") or global("loadstring")
        if type(mk) ~= "function" then return "no-load" end
        local ok, f = pcall(mk, "return 7 // 2")
        return (ok and f) and "yes" or "no"
      end)())
    for _, lib in ipairs({ "os", "io", "string", "math", "table", "package", "debug" }) do
      local value = global(lib)
      local keys = type(value) == "table" and sortedKeys(value) or nil
      if keys then
        local shown = {}
        for _, k in ipairs(keys) do shown[#shown + 1] = tostring(k) end
        line("  %s = { %s }", lib, table.concat(shown, ", "))
      else
        line("  %s = %s", lib, type(value))
      end
    end
    local pkg = global("package")
    if type(pkg) == "table" then
      line("package.path = %s", tostring(pkg.path))
      line("package.cpath = %s", tostring(pkg.cpath))
    end
    line("")
  end

  local function numbers()
    line("## Number formatting")
    local function fmt(f, v) local ok, s = pcall(string.format, f, v); return ok and s or "err" end
    line("string.format('%%.17g', 0.1)  = %s", fmt("%.17g", 0.1))
    line("string.format('%%.17g', 1/3)  = %s", fmt("%.17g", 1 / 3))
    line("string.format('%%.17g', 2^53) = %s", fmt("%.17g", 2 ^ 53))
    line("tie rounding %%.0f: 0.5=%s 1.5=%s 2.5=%s 3.5=%s (half-even => 0,2,2,4)",
      fmt("%.0f", 0.5), fmt("%.0f", 1.5), fmt("%.0f", 2.5), fmt("%.0f", 3.5))
    line("math.type = %s, 7/2 = %s, 5%%3 = %s", type(math.type), fmt("%.17g", 7 / 2), tostring(5 % 3))
    line("")
    line("## math.random")
    local function seq(n, hi)
      local t = {}
      for _ = 1, n do
        local ok, v = pcall(function() if hi then return math.random(hi) end return math.random() end)
        t[#t + 1] = ok and tostring(v) or "err"
      end
      return table.concat(t, ", ")
    end
    pcall(math.randomseed, 1)
    line("after randomseed(1): math.random() x5   = %s", seq(5))
    pcall(math.randomseed, 1)
    line("after randomseed(1): math.random(6) x10 = %s", seq(10, 6))
    line("")
  end

  local function pairsOrder()
    line("## pairs() order for string keys")
    local function orderOf()
      local t = {}
      for _, k in ipairs({ "zebra", "alpha", "mike", "bravo", "yankee", "charlie",
                           "november", "delta", "oscar", "echo" }) do
        t[k] = true
      end
      local seen = {}
      for k in pairs(t) do seen[#seen + 1] = k end
      return table.concat(seen, ",")
    end
    local first = orderOf()
    local same = true
    for _ = 1, 4 do if orderOf() ~= first then same = false end end
    line("order: %s", first)
    line("stable across 5 constructions in this state: %s", tostring(same))
    line("")
  end

  local MAX_DEPTH, MAX_BREADTH = 3, 400
  local function walk(name, value, depth)
    local tag = typeTag(value)
    if type(value) ~= "table" or depth >= MAX_DEPTH then
      line("%s%s : %s", string.rep("  ", depth), name, tag)
      return
    end
    local keys = sortedKeys(value)
    if keys == nil then
      line("%s%s : table(opaque, pairs refused)", string.rep("  ", depth), name)
      return
    end
    line("%s%s : table(%d keys)", string.rep("  ", depth), name, #keys)
    for i, k in ipairs(keys) do
      if i > MAX_BREADTH then
        line("%s... (%d more keys)", string.rep("  ", depth + 1), #keys - MAX_BREADTH)
        break
      end
      local ok, child = pcall(function() return value[k] end)
      if ok then walk(tostring(k), child, depth + 1)
      else line("%s%s : <error reading>", string.rep("  ", depth + 1), tostring(k)) end
    end
  end

  local function globalsList()
    line("## The global table")
    local env = global("_G")
    local keys = type(env) == "table" and sortedKeys(env) or nil
    if not keys then
      line("_G is %s", type(env))
    else
      for _, k in ipairs(keys) do
        local ok, v = pcall(function() return env[k] end)
        line("  %s : %s", tostring(k), ok and typeTag(v) or "error")
      end
    end
    line("")
  end

  local function apiTrees()
    local api = global("api")
    line("## api.* tree (depth %d)", MAX_DEPTH)
    if type(api) == "table" or type(api) == "userdata" then walk("api", api, 0)
    else line("api is %s (absent)", type(api)) end
    line("")
    line("## Command factories: api.cmd.make*Cmd, and TPF2's api.cmd.make.*")
    local cmd = nil
    pcall(function() cmd = api.cmd end)
    local keys = (type(cmd) == "table") and sortedKeys(cmd) or nil
    if keys then
      local n = 0
      for _, k in ipairs(keys) do
        if tostring(k):match("^make") then
          n = n + 1
          local ok, v = pcall(function() return cmd[k] end)
          line("  api.cmd.%s : %s", tostring(k), ok and typeTag(v) or "error")
        end
      end
      line("count=%d", n)
      local make = cmd.make
      local makeKeys = (type(make) == "table") and sortedKeys(make) or nil
      if makeKeys then
        for _, k in ipairs(makeKeys) do line("  api.cmd.make.%s", tostring(k)) end
      end
    else
      line("api.cmd is %s", type(cmd))
    end
    line("")
    line("## game.interface.* tree (depth %d)", MAX_DEPTH)
    local gi = nil
    pcall(function() gi = global("game").interface end)
    if gi ~= nil then walk("game.interface", gi, 0) else line("game.interface is absent") end
    line("")
  end

  local function modules()
    line("## ug_require of the game's GUI modules")
    local req = global("ug_require")
    if type(req) ~= "function" then
      line("ug_require is %s", type(req))
    else
      for _, path in ipairs({ "::/gui/main/react.lua", "::/gui/main/builtin.lua",
                              "::/gui/game_bar/game_bar_widgets.tl" }) do
        local ok, value = pcall(req, path)
        line("  %s -> %s", path, ok and typeTag(value) or ("error: " .. tostring(value):sub(1, 160)))
      end
    end
    line("")
  end

  line("# TPF3-MP script_api_dump (TF3) -- state: %s", stateName)
  for _, part in ipairs({ sandbox, numbers, pairsOrder, globalsList, apiTrees, modules }) do
    local ok, err = pcall(part)
    if not ok then line("!! a part failed: %s", tostring(err)) end
  end
  return out
end

-- Writes `lines` to <dir>/<name> where the state can write files, else to
-- the game's log through debugPrint, each line tagged, between BEGIN and END
-- lines the collector (tools/dayone/dayone.py collect) looks for. Returns
-- where they went.
local function tpf3mpEmit(tag, name, lines)
  local function getenv(key)
    local ok, v = pcall(function() return os.getenv(key) end)
    if ok and type(v) == "string" and #v > 0 then return v end
    return nil
  end
  local dir = getenv("TPF3MP_PROBE_DIR")
    or (getenv("LOCALAPPDATA") and (getenv("LOCALAPPDATA") .. "/tpf3mp/probe"))
    or (getenv("HOME") and (getenv("HOME") .. "/.local/share/tpf3mp/probe"))
  if dir then
    local ok, f = pcall(function() return io.open(dir .. "/" .. name, "w") end)
    if ok and f then
      f:write(table.concat(lines, "\n"))
      f:write("\n")
      f:close()
      return "file " .. dir .. "/" .. name
    end
  end
  local say = function(text)
    local ok = pcall(function() debugPrint(text) end)
    if not ok then pcall(function() print(text) end) end
  end
  say("[tpf3mp-probe " .. tag .. "] BEGIN " .. name)
  for _, l in ipairs(lines) do say("[tpf3mp-probe " .. tag .. "] " .. l) end
  say("[tpf3mp-probe " .. tag .. "] END " .. name .. " lines=" .. #lines)
  return "log"
end
-- TPF3MP_DUMP_CORE_END
