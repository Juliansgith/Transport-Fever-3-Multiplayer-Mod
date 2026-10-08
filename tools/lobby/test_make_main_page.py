#!/usr/bin/env python3
"""Regression tests for the TPF3-MP main-menu page generator."""

import unittest

import make_main_page


# Only the game's stable insertion anchors are needed here. Keeping the
# fixture small avoids copying the game's proprietary main_page.tl into Git.
GAME_MAIN_PAGE = (
    'local react = ug_require "/gui/main/react.lua" as React\n'
    'local table_util = ug_require "/scripts/table_util.tl" as TableUtil\n'
    'local deluxe = ug_require "configs/deluxe_content.lua"\n'
    'local notes = ug_require "releasenotes_window.tl"\n'
    'local GameEditionTextMainPage = react.RegisterRecipe(\n'
    '\tif titleIconOnlyState:old() then\n'
    '\t\treact.setStyleClasses("title-icon-only")\n'
    '\tlocal settings = button_react_util.makeIconButton(nil, "::/gui/menu/icons/settings_50.tga", function()\n'
    '-- local notifications = button_react_util.makeIconButton(nil, "::/gui/menu/icons/news_50.tga",\n'
    '\t\t\t\t\t--notifications,\n'
    '\t\t\t\t\tgui_react_util.makeHorizontalSpacer(),\n'
    '\t\t\t\t\tsettings, credits,\n'
    '\tlocal saveId = api.type.SavegameId.new()\n'
    '\t\tmapEditorDisplayDuration,\n'
    '\t\t"small-card, top-right"\n'
    '\tlocal cardWrap = vBox({\n'
    '\t\thBox({\n'
    '\t\t\tnewGameCard, \n'
    '\t\t\tcampaignCard\n'
    '\t\t}, "level1"),\n'
    '\t}, "card-wrap", cardWrapRef)\n'
    '\tlocal startMenu = vBox({\n'
    '\t\tmenu_util.EditionLogo{\n'
    '\t\t\tgameEditionText = gameEditionParamState:old(),\n'
    '\t\t},\n'
    '\t\tcardWrap,\n'
    '\t}, "level0", toSelect)\n'
    '\t\tbuiltin.FloatingLayoutChild{\n'
    '\t\t\th = 1.0,\n'
    '\t\t\tv = 1.0,\n'
    '\t\t\titem = builtin.TextView{\n'
    '\t\t\t\tmeta = {\n'
    '\t\t\t\t\tclass = "version-string, font-scale-body",\n'
    '\t\t\t\t},\n'
    '\t\t\t\ttext = api.util.getBuildVersionString(),\n'
    '\t\t\t},\n'
    '\t\t},\n'
)


class MainPageBuilderTests(unittest.TestCase):
    def test_rebuild_keeps_the_current_lobby_page_and_native_page_routes(self):
        built = make_main_page.build(GAME_MAIN_PAGE)

        self.assertIn('local Tpf3mpLobbyPage = react.RegisterRecipe("Tpf3mpLobbyPage"', built)
        self.assertNotIn("Tpf3mpLobbyWindow", built)
        self.assertIn("lobby.beginPick(mainPageParams.commonParams.setPage)", built)
        self.assertIn('setPage("LoadGame", { map = false })', built)
        self.assertIn("lobby.leaveFor(true)", built)
        self.assertIn('setPage("ModManager", { back = function()', built)
        self.assertIn("lobby.takeReopen()", built)
        self.assertIn('localKey = "tpf3mp-lobby"', built)
        self.assertIn('localKey = "tpf3mp-no-logo"', built)
        self.assertIn('localKey = "edition-logo"', built)

    def test_rebuild_adds_multiplayer_entry_without_reactivating_native_news_icon(self):
        built = make_main_page.build(GAME_MAIN_PAGE)

        self.assertIn("multiplayer, -- TPF3-MP", built)
        self.assertIn("local multiplayerCard = menu_icon_react_util.CardButton", built)
        self.assertIn("local joinCard = menu_icon_react_util.CardButton", built)
        self.assertIn("-- local notifications = button_react_util.makeIconButton", built)
        self.assertIn("--notifications,", built)
        self.assertNotIn('\n\tlocal notifications = button_react_util.makeIconButton', built)
        self.assertNotIn("Tpf3mpLobbyWindow", built)


if __name__ == "__main__":
    unittest.main()
