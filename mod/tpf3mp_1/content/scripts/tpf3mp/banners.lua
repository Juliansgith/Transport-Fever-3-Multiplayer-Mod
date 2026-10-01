-- tpf3mp/banners.lua -- the players' banners and where each player's game
-- is with the room's world, as both Multiplayer windows show them: the main
-- menu's (gui/menu/lobby.lua) and the one in the room's game
-- (gui/tpf3mp/tpf3mp.script.lua).
--
-- A banner is one of the game's own pictures, by a short id: the set is
-- tpf3mp_proto::BANNERS, in its order. Pure Lua; the tests load it as the
-- game does.

local banners = {}

-- The banners, as { id, picture }.
banners.LIST = {
	{ "m01", "::/gui/menu/images/m01_ingame.tga" },
	{ "m02", "::/gui/menu/images/m02_ingame.tga" },
	{ "m03", "::/gui/menu/images/m03_ingame.tga" },
	{ "m04", "::/gui/menu/images/m04_ingame.tga" },
	{ "m05", "::/gui/menu/images/m05_ingame.tga" },
	{ "m06", "::/gui/menu/images/m06_ingame.tga" },
	{ "m07", "::/gui/menu/images/m07_ingame.tga" },
	{ "m08", "::/gui/menu/images/m08_ingame.tga" },
	{ "temperate", "::/gui/menu/images/temperate_ingame.tga" },
	{ "subarctic", "::/gui/menu/images/subarctic_ingame.tga" },
	{ "tropical", "::/gui/menu/images/tropical_ingame.tga" },
	{ "dry", "::/gui/menu/images/dry_ingame.tga" },
	{ "mapeditor", "::/gui/menu/images/mapeditor_ingame.tga" },
	{ "mapeditor2", "::/gui/menu/images/mapeditor_ingame_2.tga" },
	{ "mod01", "::/gui/menu/images/mod01_ingame.tga" },
	{ "mod02", "::/gui/menu/images/mod02_ingame.tga" },
	{ "main", "::/gui/menu/images/main.tga" },
	{ "loadgame", "::/gui/menu/images/loadgame.tga" },
	{ "loading1", "::/gui/menu/images/loading_background_1.tga" },
	{ "loading2", "::/gui/menu/images/loading_background_2.tga" },
	{ "loading3", "::/gui/menu/images/loading_background_3.tga" },
	{ "loading4", "::/gui/menu/images/loading_background_4.tga" },
}
local PATH = {}
for _i, banner in ipairs(banners.LIST) do PATH[banner[1]] = banner[2] end

-- The banner a player shows: the one they picked, or one chosen from their
-- key (its first eight hex digits, modulo the set), the same in every
-- player's game.
function banners.of(member)
	if member.banner and PATH[member.banner] then return member.banner end
	local n = tonumber(tostring(member.id or ""):sub(1, 8), 16) or 0
	return banners.LIST[(n % #banners.LIST) + 1][1]
end

function banners.picture(id)
	return PATH[id] or banners.LIST[1][2]
end

local function same(text) return text end

-- Where a player's game is with the room's world, in words: its download,
-- its load, then in the game; before the room's game, whether it is ready.
-- `tr` translates, where the state has the game's `_`.
function banners.stage(member, playing, tr)
	tr = tr or same
	if member.loading == "fetching" then
		return string.format(tr("Downloading %d%%"), math.floor(tonumber(member.percent) or 0))
	elseif member.loading == "loading" then
		return tr("Loading...")
	elseif playing then
		return member.connected and tr("Playing") or nil
	end
	return member.ready and tr("Ready") or tr("Not ready")
end

return banners
