-- FfrUsmapDump: temporary read-only UE4SS mod deployed by `ffr-export fetch-usmap`.
-- Waits until UE5 engine initialization finishes, invokes DumpUSMAP() once on the game thread,
-- writes a JSON completion marker to ue4ss/ffr-export-usmap.done, and never modifies game state or saves.

local TAG = "[FfrUsmapDump] "
local POLL_INTERVAL_MS = 1000
local SETTLE_TICKS_AFTER_READY = 2
local BOOT_WORLD_FALLBACK_TICKS = 120

local completed = false
local inFlight = false
local engineReadyTicks = 0
local titleReadyTicks = 0

local function log(message)
  pcall(print, TAG .. tostring(message) .. "\n")
end

local function dirname(path)
  if type(path) ~= "string" then return nil end
  return path:match("^(.*)[/\\][^/\\]*$")
end

local function locateUe4ssDir()
  local okInfo, info = pcall(debug.getinfo, 1, "S")
  if okInfo and info and type(info.source) == "string" then
    local source = info.source
    local script = source:sub(1, 1) == "@" and source:sub(2) or ""
    local scriptsDir = dirname(script)
    local modDir = scriptsDir and dirname(scriptsDir) or nil
    local modsDir = modDir and dirname(modDir) or nil
    local ue4ssDir = modsDir and dirname(modsDir) or nil
    if ue4ssDir and ue4ssDir ~= "" then
      return ue4ssDir:gsub("/", "\\")
    end
  end

  if type(IterateGameDirectories) ~= "function" then
    return nil
  end

  local okDirs, directories = pcall(IterateGameDirectories)
  if not okDirs or not directories then
    return nil
  end

  local okWin64, win64 = pcall(function()
    local directory = directories.Game.Binaries.Win64
    return directory.__absolute_path or tostring(directory)
  end)
  if not okWin64 or type(win64) ~= "string" or win64 == "" then
    return nil
  end

  return win64:gsub("/", "\\") .. "\\ue4ss"
end

local function escapeJson(value)
  local text = tostring(value)
  text = text:gsub("\\", "\\\\")
  text = text:gsub("\"", "\\\"")
  text = text:gsub("\b", "\\b")
  text = text:gsub("\f", "\\f")
  text = text:gsub("\n", "\\n")
  text = text:gsub("\r", "\\r")
  text = text:gsub("\t", "\\t")
  text = text:gsub("[%z\1-\31]", function(ch)
    return string.format("\\u%04x", string.byte(ch))
  end)
  return text
end

local function jsonStringOrNull(value)
  if value == nil then return "null" end
  return "\"" .. escapeJson(value) .. "\""
end

local function writeAtomic(path, content)
  local temporary = path .. ".tmp"
  local file, openErr = io.open(temporary, "wb")
  if not file then
    error("Failed to open temporary completion marker file: " .. tostring(openErr))
  end
  local okWrite, writeErr = file:write(content)
  if not okWrite then
    pcall(function() file:close() end)
    pcall(os.remove, temporary)
    error("Failed to write completion marker: " .. tostring(writeErr))
  end
  local okClose, closeErr = file:close()
  if not okClose then
    pcall(os.remove, temporary)
    error("Failed to close completion marker: " .. tostring(closeErr))
  end
  pcall(os.remove, path)
  local okRename, renameErr = os.rename(temporary, path)
  if not okRename then
    pcall(os.remove, temporary)
    error("Failed to commit completion marker: " .. tostring(renameErr))
  end
end

local function collectUsmapFilesInNode(node, bucket)
  if type(node) ~= "table" and type(node) ~= "userdata" then
    return
  end
  local okFiles, files = pcall(function() return node.__files end)
  if not okFiles or not files then
    return
  end
  pcall(function()
    for _, entry in pairs(files) do
      local okName, name = pcall(function()
        return entry.__name or entry.__absolute_path
      end)
      if okName and type(name) == "string" then
        local fileName = name:match("([^/\\]+)$") or name
        if fileName:lower():sub(-6) == ".usmap" then
          bucket[fileName] = true
        end
      end
    end
  end)
end

local function listUsmapFiles()
  local ue4ssFiles = {}
  local win64Files = {}
  if type(IterateGameDirectories) ~= "function" then
    return ue4ssFiles, win64Files
  end

  local okDirs, dirs = pcall(IterateGameDirectories)
  if not okDirs or not dirs then
    return ue4ssFiles, win64Files
  end

  pcall(function()
    local win64 = dirs.Game and dirs.Game.Binaries and dirs.Game.Binaries.Win64
    if win64 then
      collectUsmapFilesInNode(win64, win64Files)
      if win64.ue4ss then
        collectUsmapFilesInNode(win64.ue4ss, ue4ssFiles)
      end
    end
  end)

  return ue4ssFiles, win64Files
end

local function pickUsmapFileName(beforeUe4ss, beforeWin64, afterUe4ss, afterWin64)
  local newUe4ss = {}
  for name in pairs(afterUe4ss) do
    if not beforeUe4ss[name] then
      newUe4ss[#newUe4ss + 1] = name
    end
  end
  if #newUe4ss > 0 then
    table.sort(newUe4ss)
    return newUe4ss[#newUe4ss]
  end

  local newWin64 = {}
  for name in pairs(afterWin64) do
    if not beforeWin64[name] then
      newWin64[#newWin64 + 1] = name
    end
  end
  if #newWin64 > 0 then
    table.sort(newWin64)
    return newWin64[#newWin64]
  end

  local existingUe4ss = {}
  for name in pairs(afterUe4ss) do
    existingUe4ss[#existingUe4ss + 1] = name
  end
  if #existingUe4ss > 0 then
    table.sort(existingUe4ss)
    return existingUe4ss[#existingUe4ss]
  end

  local existingWin64 = {}
  for name in pairs(afterWin64) do
    existingWin64[#existingWin64 + 1] = name
  end
  if #existingWin64 > 0 then
    table.sort(existingWin64)
    return existingWin64[#existingWin64]
  end

  return nil
end

local function isValidUObject(obj)
  if obj == nil then return false end
  local ok, valid = pcall(function() return obj:IsValid() end)
  return ok and valid == true
end

local function readFullName(obj)
  if not isValidUObject(obj) then return nil end
  local ok, full = pcall(function() return obj:GetFullName() end)
  if ok and type(full) == "string" and full ~= "" then
    return full
  end
  return nil
end

local function writeDoneMarker(ue4ssDir, okFlag, usmapFileName, worldName, errorMessage)
  local nowUnix = os.time()
  local isoTime = os.date("!%Y-%m-%dT%H:%M:%SZ", nowUnix)
  local payload = "{"
    .. "\"ok\":" .. (okFlag and "true" or "false") .. ","
    .. "\"usmapFile\":" .. jsonStringOrNull(usmapFileName) .. ","
    .. "\"timestamp\":" .. jsonStringOrNull(isoTime) .. ","
    .. "\"unixTime\":" .. tostring(nowUnix) .. ","
    .. "\"world\":" .. jsonStringOrNull(worldName) .. ","
    .. "\"error\":" .. jsonStringOrNull(errorMessage)
    .. "}\n"
  writeAtomic(ue4ssDir .. "\\ffr-export-usmap.done", payload)
end

local function runDumpOnce(ue4ssDir, worldName)
  local beforeUe4ss, beforeWin64 = listUsmapFiles()
  local dumpFn = rawget(_G, "DumpUSMAP")
  if type(dumpFn) ~= "function" then
    error("UE4SS did not expose the DumpUSMAP() function")
  end

  log("calling DumpUSMAP() world=" .. tostring(worldName))
  local returned = dumpFn()

  local usmapFileName = nil
  if type(returned) == "string" and returned:lower():sub(-6) == ".usmap" then
    usmapFileName = returned:match("([^/\\]+)$") or returned
  end

  if usmapFileName == nil then
    local afterUe4ss, afterWin64 = listUsmapFiles()
    usmapFileName = pickUsmapFileName(beforeUe4ss, beforeWin64, afterUe4ss, afterWin64)
  end

  writeDoneMarker(ue4ssDir, true, usmapFileName, worldName, nil)
  log("DumpUSMAP() succeeded file=" .. tostring(usmapFileName))
end

local function checkEngineAndDump(ue4ssDir)
  local world = FindFirstOf("World")
  local gameInstance = FindFirstOf("GameInstance")
  local playerController = FindFirstOf("PlayerController")
  if not (isValidUObject(world) and isValidUObject(gameInstance) and isValidUObject(playerController)) then
    return
  end

  engineReadyTicks = engineReadyTicks + 1
  local worldName = readFullName(world) or ""
  local lowerWorld = worldName:lower()
  local isBootMap = lowerWorld:find("boot", 1, true) ~= nil
    or lowerWorld:find("notify", 1, true) ~= nil
    or lowerWorld:find("transition", 1, true) ~= nil

  if not isBootMap then
    titleReadyTicks = titleReadyTicks + 1
  else
    titleReadyTicks = 0
  end

  if titleReadyTicks < SETTLE_TICKS_AFTER_READY and engineReadyTicks < BOOT_WORLD_FALLBACK_TICKS then
    return
  end

  completed = true
  local okDump, dumpErr = pcall(runDumpOnce, ue4ssDir, worldName)
  if not okDump then
    log("DumpUSMAP() failed: " .. tostring(dumpErr))
    local okMarker, markerErr = pcall(writeDoneMarker, ue4ssDir, false, nil, worldName, tostring(dumpErr))
    if not okMarker then
      log("failed to write error marker: " .. tostring(markerErr))
    end
  end
end

local okInit, initErr = pcall(function()
  local ue4ssDir = locateUe4ssDir()
  if not ue4ssDir then
    log("error: unable to determine UE4SS directory")
    return
  end

  log("loaded ue4ss=" .. ue4ssDir)
  LoopAsync(POLL_INTERVAL_MS, function()
    if completed then
      return true
    end
    if inFlight then
      return false
    end

    inFlight = true
    local okSched, schedErr = pcall(ExecuteInGameThread, function()
      local okStep, stepErr = pcall(checkEngineAndDump, ue4ssDir)
      inFlight = false
      if not okStep then
        log("step error: " .. tostring(stepErr))
      end
    end)
    if not okSched then
      inFlight = false
      log("ExecuteInGameThread error: " .. tostring(schedErr))
    end

    return completed
  end)
end)

if not okInit then
  log("init error: " .. tostring(initErr))
end
