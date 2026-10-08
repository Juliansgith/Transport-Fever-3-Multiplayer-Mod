#!/usr/bin/env python3
"""Builds the mod's gui/menu/main_page.tl from the game's own file.

The mod's copy is the game's main_page.tl plus the Multiplayer entry
(docs/LOBBY.md): a column of two cards right of the game's own
(Multiplayer, and Join a friend), a top-bar button and the in-page lobby
they open. The page routes to the game's own Load Game and Mod Hub pages.
Each addition is applied at an anchor that must match exactly once, so a
game patch that moves things fails loudly here rather than in the game.

    python tools/lobby/make_main_page.py <the game's gui/menu/main_page.tl> [--install <mods folder>]

The game's file is in base/content/gui.zip (`unzip gui.zip gui/menu/main_page.tl`).
With --install, the whole mod is copied into a mods folder (TF3's staging_area).
"""
import argparse
import json
import os
import re
import shutil
import sys

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
MOD = os.path.join(REPO, "mod", "tpf3mp_1")
OUT = os.path.join(MOD, "content", "gui", "menu", "main_page.tl")

HEADER = '''-- TPF3-MP: the game's gui/menu/main_page.tl with a Multiplayer entry added to
-- the main menu. The hook serves this copy in place of the game's file
-- (docs/LOBBY.md). Every change is marked "TPF3-MP:" so a game patch can be
-- re-applied: tools/lobby/make_main_page.py takes the new game file and
-- re-applies the marked blocks. Relative and leading-slash paths are made
-- absolute ("::/") so the copy resolves the game's files from the mod's folder.
'''

LOBBY_PAGE = '''-- TPF3-MP: the Multiplayer page, shown over the main menu while this is
-- set: { focus }, "join" putting joining by invite first. A page as the
-- game's own (load_game_page.tl), with its top bar, not a window. The
-- menu's cards stay, hidden as the game hides them under its windows
-- (title-icon-only): their node refs must stay attached, or the game
-- stops ("Could not initialize all node refs", 2026-10-04).
local record LobbyModule
	prepareNewWorld : function()
	content : function(onClose : function(), focus : string, onNewGame : function(), onPick : function(), onModHub : function(), commonParams : CommonMenuParams) : TreeNodeId
	beginPick : function(setPage : any) : boolean
	endPick : function() : boolean
	leaveFor : function(again : boolean)
	takeReopen : function() : boolean
	CardLine : function(params : any) : TreeNodeId
	joinLine : function(state : any) : string
end
local lobby = ug_require "tpf3mp_1::/gui/menu/lobby.lua" as LobbyModule

local record Tpf3mpLobbyPageParam
	onNewGame : function()
	onPick : function()
	onModHub : function()
	onClose : function()
	focus : string
	commonParams : CommonMenuParams
end

-- TPF3-MP: the lobby's page, or, should its Lua fail, the error and a way
-- out - never a page that cannot be left.
local Tpf3mpLobbyPage = react.RegisterRecipe("Tpf3mpLobbyPage", function(param : Tpf3mpLobbyPageParam) : TreeNodeId
	local ok, result = pcall(lobby.content, param.onClose, param.focus, param.onNewGame, param.onPick,
		param.onModHub, param.commonParams)
	if ok then
		return result as TreeNodeId
	end
	pcall(debugPrint, "[tpf3mp] lobby: content failed: " .. tostring(result))
	return builtin.BoxLayout{
		orientation = builtin.type.Orientation.Vertical,
		children = {
			builtin.TextView{ meta = { class = "font-scale-title-4" }, text = _("Multiplayer") },
			builtin.TextView{ meta = { class = "font-scale-body, error" }, text = tostring(result) },
			builtin.Button{
				meta = { class = "secondary" },
				content = builtin.TextView{ text = _("Close") },
				onClick = param.onClose,
			},
		},
	}
end)

'''

SHOW = '''	-- TPF3-MP: the Multiplayer page, shown over the main menu while this is
	-- set: { focus }, "join" putting joining by invite first. A page as the
	-- game's own (load_game_page.tl), with its top bar, not a window. The
	-- menu's cards stay, hidden as the game hides them under its windows
	-- (title-icon-only): their node refs must stay attached, or the game
	-- stops ("Could not initialize all node refs", 2026-10-04).
	local lobbyState : ReactStateT<{string:string}> = react.useState(nil)
	local showMultiplayer = function(focus : string)
		titleIconOnlyState:set(true)
		lobbyState:set({ focus = focus })
	end
	local closeMultiplayer = function()
		lobbyState:set(nil)
		titleIconOnlyState:set(false)
		fastFadeInState:set(true)
		cardsFadeInStartTimeRef:set(api.util.getApplicationTime())
	end

	-- TPF3-MP: back from a page the Multiplayer window sent the player to
	-- (the room's save and mods, Mod Hub): the window again. A pick under
	-- way ends here whichever way the player came back.
	react.onMount(function()
		local picking = lobby.endPick()
		if lobby.takeReopen() or picking then
			showMultiplayer(nil)
		end
	end)

	if titleIconOnlyState:old() then
		react.setStyleClasses("title-icon-only")'''

TOPBAR_BUTTON = '''	-- TPF3-MP: the Multiplayer button in the top bar, a glyph drawn as the
	-- game's own top-bar icons are (tools/art/icons/menu_icon.py).
	local multiplayer = button_react_util.makeIconButton(nil, "tpf3mp_1::/gui/tpf3mp/icons/menu_multiplayer_50.tga", function()
		if clickAllowed("TopBar") then
			showMultiplayer(nil)
		end
	end, _("Multiplayer"))

	local settings = button_react_util.makeIconButton(nil, "::/gui/menu/icons/settings_50.tga", function()'''

CARD = '''	-- TPF3-MP: the Multiplayer cards, a column right of the game's own cards:
	-- Multiplayer (connect, create or join) and Join a friend (the window with
	-- the invite first). Each card's line under its title is live, from the
	-- lobby the hook has (lobby.CardLine). The label is the game's own
	-- (menu_icon_react_util.makeCardLabelBottomComponent), with that line in
	-- place of the fixed description.
	local tpf3mpCardLabel = function(title : string, line : function(any) : string) : TreeNodeId
		return builtin.FloatingLayout{
			children = {
				builtin.FloatingLayoutChild{
					item = builtin.ShaderQuad{
						meta = {
							mouseTransparent = true,
						},
						scaling = builtin.type.ImageViewScaling.AutoZoom,
						path0 = "::/gui/menu/design/blackOpaque_effects.tga",
						path1 = nil, -- nrm
						path2 = "::/gui/menu/design/allgreen_masks.tga",
						path3 = "::/gui/menu/design/allwhite_main.tga",
						specularColor = api.type.Vec3f.new(0.42, 0.75, 0.87),
						specularMixAmount = 1.0,
						animatedRippleStrength = 0.18,
						mouseGradDist = 360.0,
						mouseGradBaseStr = 0.34,
						mouseClickStr = 0.51,
						useFullOpacity = false,
						rippleEffectOnClick = true,
						useNormalMap = true,
						mouseGradAdditional = true,
					},
				},
				builtin.FloatingLayoutChild{
					item = builtin.Component{
						layout = builtin.BoxLayout{
							orientation = builtin.type.Orientation.Horizontal,
							children = {
								builtin.Component{
									meta = { class = "title-and-description", },
									layout = builtin.BoxLayout{
										orientation = builtin.type.Orientation.Vertical,
										children = {
											builtin.TextView{
												meta = { class = "font-scale-main-card-title" },
												text = title,
											},
											lobby.CardLine{ line = line },
										},
									},
								},
								gui_react_util.makeHorizontalSpacer(),
							},
						},
					},
				},
			},
		}
	end

	local multiplayerCard = menu_icon_react_util.CardButton{
		bottomComponent = tpf3mpCardLabel(_("Multiplayer"), nil),
		onClick = function()
			if clickAllowed("Cards") then
				showMultiplayer(nil)
			end
		end,
		onAttention = function(x : number, y : number)
			mainPageParams.commonParams.triggerBackgroundEvent(api.type.Vec2f.new(x, y))
		end,
		tooltip = _("Play together online: connect, create a room or join one"),
		images = { "::/gui/menu/images/m02_ingame.tga", "::/gui/menu/images/m07_ingame.tga" },
		initialImageIndex = 1,
		imageSwapOffsetSeconds = 0.41 * 36.0,
		displayDurationSeconds = 36.0,
		class = "small-card, top-right",
		extraChildren = {
			builtin.FloatingLayoutChild{
				h = 0.06,
				v = 0.08,
				item = builtin.ImageView{
					meta = { mouseTransparent = true },
					path = "tpf3mp_1::/gui/tpf3mp/icons/menu_multiplayer_50.tga",
				},
			},
		},
	}

	local joinCard = menu_icon_react_util.CardButton{
		bottomComponent = tpf3mpCardLabel(_("Join a friend"), lobby.joinLine),
		onClick = function()
			if clickAllowed("Cards") then
				showMultiplayer("friend")
			end
		end,
		onAttention = function(x : number, y : number)
			mainPageParams.commonParams.triggerBackgroundEvent(api.type.Vec2f.new(x, y))
		end,
		tooltip = _("Join a friend's room with the invite code they send you"),
		images = { "::/gui/menu/images/m05_ingame.tga" },
		initialImageIndex = 1,
		class = "small-card",
	}

	local multiplayerColumn = vBox({
		vBox({ multiplayerCard }, "level2b"),
		vBox({ joinCard }, "level2b"),
	}, "level1")

	local saveId = api.type.SavegameId.new()'''


def build(source_text):
    """Build the mod's current Multiplayer page into the game's main_page.tl."""
    text = source_text

    def edit(anchor, new, count=1):
        nonlocal text
        n = text.count(anchor)
        if n != count:
            raise SystemExit(f"anchor found {n}x, expected {count}: {anchor[:70]!r}")
        text = text.replace(anchor, new)

    edit('local react = ug_require "/gui/main/react.lua" as React\n',
         HEADER + 'local react = ug_require "/gui/main/react.lua" as React\n')
    edit('local table_util = ug_require "/scripts/table_util.tl" as TableUtil\n',
         'local table_util = ug_require "/scripts/table_util.tl" as TableUtil\n'
         '\n-- TPF3-MP: the game log shows the copy is in effect, even if nothing is clicked.\n'
         'pcall(debugPrint, "[tpf3mp] main menu: TPF3-MP main_page.tl is in effect")\n')
    edit('ug_require "configs/deluxe_content.lua"', 'ug_require "::/gui/menu/configs/deluxe_content.lua"')
    edit('ug_require "releasenotes_window.tl"', 'ug_require "::/gui/menu/releasenotes_window.tl"')
    edit('local GameEditionTextMainPage = react.RegisterRecipe(',
         LOBBY_PAGE + 'local GameEditionTextMainPage = react.RegisterRecipe(')
    edit('	if titleIconOnlyState:old() then\n		react.setStyleClasses("title-icon-only")', SHOW)
    edit('	local settings = button_react_util.makeIconButton(nil, "::/gui/menu/icons/settings_50.tga", function()',
         TOPBAR_BUTTON)
    edit('					gui_react_util.makeHorizontalSpacer(),\n					settings, ',
         '					gui_react_util.makeHorizontalSpacer(),\n					multiplayer, -- TPF3-MP\n					settings, ')
    edit('	local saveId = api.type.SavegameId.new()', CARD)
    # The Multiplayer column has the grid's top-right corner now.
    edit('		mapEditorDisplayDuration,\n		"small-card, top-right"\n',
         '		mapEditorDisplayDuration,\n		"small-card" -- TPF3-MP: was "small-card, top-right"; the Multiplayer card has that corner\n')
    edit('	local cardWrap = vBox({\n		hBox({\n			newGameCard, ',
         '	local cardWrap = vBox({\n		hBox({ vBox({ -- TPF3-MP: the game\'s cards, then the Multiplayer column\n		hBox({\n			newGameCard, ')
    edit('			campaignCard\n		}, "level1"),\n	}, "card-wrap", cardWrapRef)',
         '			campaignCard\n		}, "level1"),\n		}), multiplayerColumn }), -- TPF3-MP\n	}, "card-wrap", cardWrapRef)')

    edit('''\t\tmenu_util.EditionLogo{
\t\t\tgameEditionText = gameEditionParamState:old(),
\t\t},''', '''\t\t-- TPF3-MP: no logo under the Multiplayer page, whose top bar has its
\t\t-- title there; each its own key, so one is never turned into the other.
\t\tlobbyState:old() and builtin.Component{
\t\t\tmeta = { localKey = "tpf3mp-no-logo" },
\t\t} or menu_util.EditionLogo{
\t\t\tmeta = { localKey = "edition-logo" },
\t\t\tgameEditionText = gameEditionParamState:old(),
\t\t},''')
    edit('''\t\tbuiltin.FloatingLayoutChild{
\t\t\th = 1.0,
\t\t\tv = 1.0,
\t\t\titem = builtin.TextView{
\t\t\t\tmeta = {
\t\t\t\t\tclass = "version-string, font-scale-body",
\t\t\t\t},
\t\t\t\ttext = api.util.getBuildVersionString(),
\t\t\t},
\t\t},''', '''\t\tbuiltin.FloatingLayoutChild{
\t\t\th = 1.0,
\t\t\tv = 1.0,
\t\t\titem = builtin.TextView{
\t\t\t\tmeta = {
\t\t\t\t\tclass = "version-string, font-scale-body",
\t\t\t\t},
\t\t\t\ttext = api.util.getBuildVersionString(),
\t\t\t},
\t\t},
\t\t-- TPF3-MP: the Multiplayer page over the hidden cards, its own key.
\t\tlobbyState:old() and builtin.FloatingLayoutChild{
\t\t\tlocalKey = "tpf3mp-lobby",
\t\t\th = -1,
\t\t\tv = -1,
\t\t\titem = Tpf3mpLobbyPage{
\t\t\t\tfocus = lobbyState:old().focus,
\t\t\t\tcommonParams = mainPageParams.commonParams,
\t\t\t\tonClose = closeMultiplayer,
\t\t\t\t-- The room's save and mods, on the game's own Load Game
\t\t\t\t-- page (gui/menu/roommods.lua); back here when it is done.
\t\t\t\tonPick = function()
\t\t\t\t\tif lobby.beginPick(mainPageParams.commonParams.setPage) then
\t\t\t\t\t\tmainPageParams.commonParams.setPage("LoadGame", { map = false })
\t\t\t\t\tend
\t\t\t\tend,
\t\t\t\t-- The game's Mod Hub, to sign in; back here from it.
\t\t\t\tonModHub = function()
\t\t\t\t\tlocal setPage = mainPageParams.commonParams.setPage
\t\t\t\t\tlobby.leaveFor(true)
\t\t\t\t\tsetPage("ModManager", { back = function() setPage("Main", nil) end })
\t\t\t\tend,
\t\t\t\tonNewGame = function()
\t\t\t\t\tlobby.prepareNewWorld()
\t\t\t\t\tmainPageParams.commonParams.setPage("NewGame", { map = false })
\t\t\t\tend,
\t\t\t},
\t\t} or nil,''')
    # A leading-slash path is resolved against the requiring file's root, which
    # for the mod's copy is tpf3mp_1::/; name the game's root explicitly.
    text = re.sub(r'"/(gui|scripts|base|mission)/', lambda m: '"::/' + m.group(1) + '/', text)
    return text


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("source", help="the game's gui/menu/main_page.tl")
    parser.add_argument("--install", metavar="MODS", help="also copy the mod into this mods folder")
    args = parser.parse_args()

    source = open(args.source, encoding="utf-8").read()
    text = build(source)
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w", encoding="utf-8", newline="\n") as out:
        out.write(text)
    print(f"wrote {OUT} ({len(text.splitlines())} lines)")

    content = os.path.join(MOD, "_content.json")
    listing = json.load(open(content, encoding="utf-8"))
    for path in ("gui/menu/main_page.tl", "gui/menu/lobby.lua", "tpf3mp/state.lua", "tpf3mp/act.lua",
                 "gui/tpf3mp/icons/menu_multiplayer_50.tga", "gui/tpf3mp/icons/menu_multiplayer_50@2x.tga"):
        if path not in listing["files"]:
            listing["files"].insert(0, path)
            print(f"listed {path} in _content.json")
    with open(content, "w", encoding="utf-8", newline="\n") as out:
        out.write(json.dumps(listing, indent=4) + "\n")

    if args.install:
        dest = os.path.join(args.install, "tpf3mp_1")
        if os.path.isdir(dest):
            shutil.rmtree(dest)
        shutil.copytree(MOD, dest)
        print(f"installed -> {dest}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
