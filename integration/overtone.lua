-- Overtone mpv adapter, protocol 1. This script reads only private local state.
local mp = require("mp")
local utils = require("mp.utils")
local map_path = os.getenv("OVERTONE_MPV_MAP")
local serial = 0
local info
local override

local function publish()
    mp.set_property_native("user-data/overtone", info)
end

local function read(path, limit)
    local file = io.open(path, "rb")
    if not file then return nil end
    local data = file:read(limit + 1)
    file:close()
    if data and #data <= limit then return data end
end

local function find_rule(path)
    local json = map_path and read(map_path, 1048576)
    local map = json and utils.parse_json(json)
    if type(map) ~= "table" or map.version ~= 1 or type(map.pid) ~= "number" then return nil end
    local stat = read("/proc/" .. tostring(map.pid) .. "/stat", 4096)
    local rest = stat and stat:match("^.*%) (.*)$")
    if not rest then return nil end
    local fields = {}
    for field in rest:gmatch("%S+") do fields[#fields + 1] = field end
    if fields[1] == "Z" or fields[20] ~= map.proc_start then return nil end
    return map.rules and map.rules[path]
end

mp.set_property_native("user-data/overtone-protocol", 1)
mp.add_hook("on_load", 40, function()
    local path = mp.get_property("path", "")
    serial = serial + 1
    local rule
    if override and override.path == path and override.serial == serial - 1 then
        rule = override.rule
    else
        rule = find_rule(path)
    end
    override = nil
    info = { serial = serial, original = path, state = "loading", time = os.time() }
    if rule and rule.to ~= path then
        info.target = rule.to
        info.origin = rule.origin
        mp.set_property("file-local-options/network-timeout", "4")
        mp.set_property("stream-open-filename", rule.to)
    end
    publish()
end)

mp.add_hook("on_load_fail", 40, function()
    if info and info.target then
        -- mpv 0.41 retries only when the retry URI differs from the queue entry.
        local retry
        if info.original:sub(1, 1) == "/" then
            retry = "/." .. info.original
        elseif info.original:match("^https://") then
            retry = info.original:gsub("#.*$", "") .. "#overtone-original"
        end
        if retry then
            mp.set_property("stream-open-filename", retry)
            info.state = "fallback_pending"
            publish()
        end
    end
end)

mp.register_event("playback-restart", function()
    if info and info.state == "loading" then
        info.state = info.target and "applied" or "original"
        info.time = os.time()
        publish()
    elseif info and info.state == "fallback_pending" then
        info.state = "fallback"
        info.time = os.time()
        publish()
    end
end)

mp.register_event("end-file", function(event)
    if info then
        info.state = event.reason == "error" and "error" or "ended"
        info.time = os.time()
        publish()
    end
end)

mp.register_script_message("reload", function(expected, source, label)
    if not info or tonumber(expected) ~= info.serial or info.state == "ended" or info.state == "error" then
        return
    end
    override = { path = info.original, serial = info.serial }
    if source ~= "" then override.rule = { to = source, origin = label } end
    mp.commandv("playlist-play-index", "current")
end)
