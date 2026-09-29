#!/usr/bin/env python3
"""Builds the mod's gui/menu/main_page.tl from the game's own file.

The mod's copy is the game's main_page.tl plus the Multiplayer entry
(docs/LOBBY.md): a card, a top-bar button and the window they open. Each
addition is applied at an anchor that must match exactly once, so a game
patch that moves things fails loudly here rather than in the game.

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

LOBBY_WINDOW = '''-- TPF3-MP: the Multiplayer window, opened from the main menu into the menu's
-- own window container (as DeluxeContentWindow is). Its content is the mod's
-- gui/menu/lobby.lua: the lobby, talking to the hook.
local record LobbyModule
	content : function(onClose : function()) : TreeNodeId
end
local lobby = ug_require "tpf3mp_1::/gui/menu/lobby.lua" as LobbyModule

local record Tpf3mpLobbyWindowParam
	onClose : function()
	pos : Vec2f
end

local Tpf3mpLobbyWindow = react.RegisterWrapperRecipe("Tpf3mpLobbyWindow", builtin.Window, function(param : Tpf3mpLobbyWindowParam) : TreeNodeId
	return builtin.Window{
		title = _("Multiplayer"),
		id = "window.tpf3mp.lobby",
		meta = {
			class = "fade-in"
		},
		initialX = param.pos and param.pos.x or nil,
		initialY = param.pos and param.pos.y or nil,
		movable = false,
		closable = true,
		onClose = param.onClose,
		content = lobby.content(param.onClose),
	}
end)

'''

SHOW = '''	-- TPF3-MP: open the Multiplayer window, as showDeluxeContent opens its window.
	local showMultiplayer = function()
		local pos = api.type.Vec2f.new(0.5, 0.5)
		titleIconOnlyState:set(true)
		local wc = mainPageParams.commonParams.windowContainer:get():getApi()
		wc.addSingletonWindow(Tpf3mpLobbyWindow, {
			onClose = function()
				titleIconOnlyState:set(false)
				fastFadeInState:set(true)
				cardsFadeInStartTimeRef:set(api.util.getApplicationTime())
				mainPageParams.commonParams.windowContainer:get():getApi().removeAllWindows(Tpf3mpLobbyWindow)
			end,
			pos = pos,
		})
	end

	if titleIconOnlyState:old() then
		react.setStyleClasses("title-icon-only")'''

TOPBAR_BUTTON = '''	-- TPF3-MP: the Multiplayer button in the top bar.
	local multiplayer = button_react_util.makeIconButton(nil, "tpf3mp_1::/gui/tpf3mp/icons/button_multiplayer.tga", function()
		if clickAllowed("TopBar") then
			showMultiplayer()
		end
	end, _("Multiplayer"))

	local settings = button_react_util.makeIconButton(nil, "::/gui/menu/icons/settings_50.tga", function()'''

CARD = '''	-- TPF3-MP: the Multiplayer card. Placeholder art until the mod has its own.
	local multiplayerCard = menu_icon_react_util.makeCardButton(
		_("Multiplayer"),
		function()
			if clickAllowed("Cards") then
				showMultiplayer()
			end
		end,
		function(x : number, y : number) -- onAttention
			mainPageParams.commonParams.triggerBackgroundEvent(api.type.Vec2f.new(x, y))
		end,
		"",
		{ "::/gui/menu/images/m03_ingame.tga" },
		1,
		nil,
		nil
	)

	local saveId = api.type.SavegameId.new()'''


def build(source_text):
    """The mod's main_page.tl from the game's."""
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
         LOBBY_WINDOW + 'local GameEditionTextMainPage = react.RegisterRecipe(')
    edit('	if titleIconOnlyState:old() then\n		react.setStyleClasses("title-icon-only")', SHOW)
    edit('	local settings = button_react_util.makeIconButton(nil, "::/gui/menu/icons/settings_50.tga", function()',
         TOPBAR_BUTTON)
    edit('					gui_react_util.makeHorizontalSpacer(),\n					settings, ',
         '					gui_react_util.makeHorizontalSpacer(),\n					multiplayer, -- TPF3-MP\n					settings, ')
    edit('	local saveId = api.type.SavegameId.new()', CARD)
    edit('			}, "level2b")\n		}, "level1"),',
         '			}, "level2b"),\n			multiplayerCard, -- TPF3-MP\n		}, "level1"),')

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
    for path in ("gui/menu/main_page.tl", "gui/menu/lobby.lua", "tpf3mp/state.lua", "tpf3mp/act.lua"):
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
