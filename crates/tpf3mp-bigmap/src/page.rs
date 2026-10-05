//! The mod's copy of TF3's New Game settings page,
//! `mod/tpf3mp_bigmap_1/content/gui/menu/new_game_or_map_settings_page.tl`.
//!
//! TF3's map sizes are a table in that page's `getNumTiles`, and its size
//! dropdown reads its rows from the base mod's `map.size` parameter, which a
//! mod cannot extend. A mod cannot replace a main-menu file either, so the
//! hook serves this copy in place of the game's, as it serves the main
//! page (`crates/tpf3mp-hook/src/menu_entry.rs`). This replaces what
//! silver2127's Big Maps for TPF2 (tpf2-bigmap) did natively: the
//! `GetNumTilesNew` detour and the added dropdown rows.
//!
//! Every change is a marked block of whole lines,
//!
//! ```text
//! -- TPF3-MP begin: why
//! the copy's lines
//! -- TPF3-MP was:
//! --the game's lines it replaces, each behind "--"
//! -- TPF3-MP end
//! ```
//!
//! (an insertion has no "was" part), so [`original`] gives back the game's
//! file exactly and a test holds it to the game's SHA-256: the copy is the
//! game's file plus marked blocks, and nothing else. On a game patch,
//! `cargo run -p tpf3mp-bigmap -- page <the game's file>` applies the
//! blocks to the new file ([`build`]); an anchor that moved fails loudly.

use thiserror::Error;

/// SHA-256 of `gui/menu/new_game_or_map_settings_page.tl` in TF3 build
/// 40408's `base/content/gui.zip`, the file the copy is made from.
pub const GAME_PAGE_SHA256: &str =
    "5c353d53b3ff5ef385764557e54c120e48b492e78c94e4dfae60d87021688890";

/// Where the copy lives in the mod, and where the hook serves it from.
pub const COPY_PATH: &str = "gui/menu/new_game_or_map_settings_page.tl";

const BEGIN: &str = "-- TPF3-MP begin: ";
const WAS: &str = "-- TPF3-MP was:";
const END: &str = "-- TPF3-MP end";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PageError {
    #[error("the anchor of {why:?} is found {found} times, expected {expected}")]
    Anchor {
        why: &'static str,
        found: usize,
        expected: usize,
    },
    #[error("line {line}: {problem}")]
    Block { line: usize, problem: &'static str },
}

/// How an edit places its block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    Before,
    After,
    Replace,
}

/// One change to the game's file.
struct Edit {
    why: &'static str,
    /// Whole lines of the game's file, each ending in a newline.
    anchor: &'static str,
    place: Place,
    /// The copy's lines, each ending in a newline.
    lines: &'static str,
    /// How many times the anchor occurs; each is changed.
    count: usize,
}

const REQUIRES: &str = r#"local builtin = ug_require "/gui/main/builtin.lua" as Builtin
local content_card = ug_require "::/gui/main/content_card.tl" as ContentCard
local difficulty_util = ug_require "/base/difficulty_util.tl" as DifficultyUtil
local lang_util = ug_require "/scripts/lang_util.tl" as LangUtil
local menu_icon_react_util = ug_require "/gui/menu/menu_icon_react_util.tl" as MenuIconReactUtil
local new_game_react_util = ug_require "new_game_react_util.tl" as NewGameReactUtil
local react = ug_require "/gui/main/react.lua" as React
local savegame_react_util = ug_require "/gui/menu/savegame_react_util.tl" as SavegameReactUtil
local callout_react_util = ug_require "/gui/main/callout_react_util.tl" as CalloutReactUtil
local script_param_util = ug_require "/gui/main/script_param_util.tl" as ScriptParamUtil
local table_util = ug_require "/scripts/table_util.tl" as TableUtil

local GameplaySettings = ug_require "advanced_settings.tl" as Recipe<IMenuPageParam>
local ModSelectorPage = ug_require "mod_selector_page.tl" as Recipe<IMenuPageParam>
"#;

const REQUIRES_COPY: &str = r#"-- TPF3-MP: the game's gui/menu/new_game_or_map_settings_page.tl with big
-- maps' size rows added to the size dropdown (docs/BIGMAPS.md, "Stage 1").
-- The hook serves this copy in place of the game's file and falls back to
-- the game's own page if it does not load. Each change is a marked block,
-- the game's lines it replaces kept under "was" as comments: re-apply them
-- to a patched game file with `cargo run -p tpf3mp-bigmap -- page <file>`
-- (crates/tpf3mp-bigmap/src/page.rs), never by hand. After silver2127's
-- Big Maps for TPF2 (tpf2-bigmap), whose native size detour this replaces.
-- Paths are absolute ("::/"): a leading-slash or relative path would be
-- looked up in this mod's folder.
local builtin = ug_require "::/gui/main/builtin.lua" as Builtin
local content_card = ug_require "::/gui/main/content_card.tl" as ContentCard
local difficulty_util = ug_require "::/base/difficulty_util.tl" as DifficultyUtil
local lang_util = ug_require "::/scripts/lang_util.tl" as LangUtil
local menu_icon_react_util = ug_require "::/gui/menu/menu_icon_react_util.tl" as MenuIconReactUtil
local new_game_react_util = ug_require "::/gui/menu/new_game_react_util.tl" as NewGameReactUtil
local react = ug_require "::/gui/main/react.lua" as React
local savegame_react_util = ug_require "::/gui/menu/savegame_react_util.tl" as SavegameReactUtil
local callout_react_util = ug_require "::/gui/main/callout_react_util.tl" as CalloutReactUtil
local script_param_util = ug_require "::/gui/main/script_param_util.tl" as ScriptParamUtil
local table_util = ug_require "::/scripts/table_util.tl" as TableUtil

local GameplaySettings = ug_require "::/gui/menu/advanced_settings.tl" as Recipe<IMenuPageParam>
local ModSelectorPage = ug_require "::/gui/menu/mod_selector_page.tl" as Recipe<IMenuPageParam>
"#;

const MODULE: &str = r#"-- The ladder and the logic are plain Lua in this mod
-- (scripts/tpf3mp_bigmap/, tested in crates/tpf3mp-bigmap/tests/mod_lua.rs).
-- Should they not load, the page offers the game's own sizes only.
local record BigmapRow
	label : string
	tiles : integer
	peakMb : number
end

local record BigmapMenu
	ramMb : function() : number
	offered : function(ladder : {BigmapRow}, ramMb : number) : {BigmapRow}, {string}
	sizeValues : function(stockValues : {string}, rows : {BigmapRow}) : {string}
	indexOf : function(value : number, pick : integer, stockCount : integer, stockNumbers : {number}, rowCount : integer) : integer
	choose : function(index : integer, stockCount : integer, stockNumbers : {number}) : number, integer
	gameTiles : function(rows : {BigmapRow}, pick : integer, ratioIndex : integer) : any, any
	densityValues : function(stockValues : {string}, ladder : {BigmapRow}) : {string}
	STOCK_DENSITY_LEVELS : integer
end

local bigmapMenuLoaded, bigmapMenuValue = pcall(ug_require, "tpf3mp_bigmap_1::/scripts/tpf3mp_bigmap/menu.lua")
local bigmapLadderLoaded, bigmapLadderValue = pcall(ug_require, "tpf3mp_bigmap_1::/scripts/tpf3mp_bigmap/ladder.lua")
local bigmap : BigmapMenu = nil
local bigmapLadder : {BigmapRow} = nil
if bigmapMenuLoaded and bigmapLadderLoaded and bigmapMenuValue ~= nil and bigmapLadderValue ~= nil then
	bigmap = bigmapMenuValue as BigmapMenu
	bigmapLadder = bigmapLadderValue as {BigmapRow}
	pcall(debugPrint, "[tpf3mp] big maps: TPF3-MP new_game_or_map_settings_page.tl is in effect")
else
	local reason = bigmapMenuLoaded and bigmapLadderValue or bigmapMenuValue
	pcall(debugPrint, "[tpf3mp] big maps: the mod's scripts did not load (" .. tostring(reason) .. "); the sizes are the game's own")
end

-- The added row picked (1 is the first), 0 for one of the game's own. It
-- lasts the session; the page's state carries it between renders.
local bigmapPick = 0
local bigmapPickState : ReactStateT<integer> = nil
-- The added rows this machine has the memory to generate, and why any
-- other is missing.
local bigmapRows : {BigmapRow} = {}
local bigmapNotes : {string} = {}

local function bigmapRefresh()
	bigmapRows = {}
	bigmapNotes = {}
	if bigmap == nil then
		return
	end
	local ok, rows, notes = pcall(function() : {BigmapRow}, {string}
		return bigmap.offered(bigmapLadder, bigmap.ramMb())
	end)
	if ok then
		bigmapRows = rows as {BigmapRow}
		bigmapNotes = notes as {string}
	else
		pcall(debugPrint, "[tpf3mp] big maps: " .. tostring(rows) .. "; the sizes are the game's own")
	end
end

-- The tiles of the added row picked at the ratio `format` (0-based, as
-- getNumTiles has it), or nil for one of the game's own rows.
local function bigmapNumTiles(format : integer) : Vec2i
	if bigmap == nil or bigmapPick < 1 then
		return nil
	end
	local ok, x, y = pcall(bigmap.gameTiles, bigmapRows, bigmapPick, format)
	if not ok or type(x) ~= "number" or type(y) ~= "number" then
		pcall(debugPrint, "[tpf3mp] big maps: no shape for row " .. tostring(bigmapPick) .. " at ratio " .. tostring(format) .. " (" .. tostring(y or x) .. "); the game's own size is used")
		return nil
	end
	return api.type.Vec2i.new(x as integer, y as integer)
end

-- The size dropdown: the game's rows, then the added rows this machine can
-- generate. A game row sets "map.size" as the game's own dropdown does. An
-- added row sets it to the game's largest size, so the save and every other
-- reader of "map.size" see a value the game knows, and keeps its own pick
-- here. A line under the dropdown says why a row is missing.
local function bigmapAddMapSizeSettingsEntry(settings : {NewGameReactUtil.SettingsEntry}, activeModsParamsState : ReactStateT<{string : {string : integer}}>, filterTags : {string})
	local scriptParam = script_param_util.getScriptParam("map.size", filterTags)
	if bigmap == nil or scriptParam == nil or bigmapPickState == nil or #bigmapRows == 0 then
		bigmapPick = 0
		new_game_react_util.addMapSizeSettingsEntry(settings, activeModsParamsState, filterTags)
	else
		local stockCount = #scriptParam.values
		local stockNumbers = scriptParam.numbers
		local paramForUi : ScriptParamUtil.ParamForUi = {
			uiType = scriptParam.uiType,
			defaultIndex = scriptParam.defaultIndex,
			name = scriptParam.name,
			values = bigmap.sizeValues(scriptParam.values, bigmapRows),
			numbers = nil,
			tooltips = nil,
			allowCoalesce = true,
		}
		local element = script_param_util.buildScriptParamCompSimple({
			scriptParam = paramForUi,
			currentValue = bigmap.indexOf(activeModsParamsState:old()[""]["map.size"], bigmapPick, stockCount, stockNumbers, #bigmapRows),
			onValueChange = function(value : number)
				local mapSize, pick = bigmap.choose(math.floor(value), stockCount, stockNumbers)
				bigmapPick = pick
				bigmapPickState:set(pick)
				local copy = table_util.copy(activeModsParamsState:old())
				copy[""]["map.size"] = mapSize as integer
				if not table_util.deepEquals(copy, activeModsParamsState:old()) then
					activeModsParamsState:set(copy)
				end
			end,
			toggleButtonsFlowLayout = false,
			vertical = false,
			addSpacer = true,
		})
		table.insert(settings, {
			title = scriptParam.name,
			description = scriptParam.tooltip,
			hintIdKey = "hintIdKey" .. scriptParam.name,
			element = element,
		})
	end
	if bigmap ~= nil and #bigmapNotes > 0 then
		table.insert(settings, {
			title = "",
			description = "",
			element = script_param_util.wrap(
				_("Big maps"),
				false,
				builtin.TextView{
					meta = { class = "right-parameters, font-scale-body" },
					text = table.concat(bigmapNotes, "\n"),
				},
				true
			),
		})
	end
end

"#;

const DENSITY: &str = r#"
-- A density slider (towns, or industries with their runtime target) with
-- big maps' levels after the game's own: "Gigantomaniac count at <size>"
-- (scripts/tpf3mp_bigmap/menu.lua). The level is stored where the game's
-- own slider stores it; the hook's difficulty_util wrap gives it its
-- factor. Without the mod's scripts, or a slider of another length, the
-- game's own slider.
local function bigmapAddDensitySettingsEntry(settings : {NewGameReactUtil.SettingsEntry}, activeModsParamsState : ReactStateT<{string : {string : integer}}>, key : string, filterTags : {string}, otherKeys : {string})
	local scriptParam = script_param_util.getScriptParam(key, filterTags)
	if bigmap == nil or bigmapLadder == nil or #bigmapLadder == 0 or scriptParam == nil or #scriptParam.values ~= bigmap.STOCK_DENSITY_LEVELS then
		new_game_react_util.searchBuildAndAddScriptParamComp(settings, activeModsParamsState, key, false, filterTags, otherKeys)
		return
	end
	local paramForUi : ScriptParamUtil.ParamForUi = {
		uiType = scriptParam.uiType,
		defaultIndex = scriptParam.defaultIndex,
		name = scriptParam.name,
		values = bigmap.densityValues(scriptParam.values, bigmapLadder),
		numbers = nil,
		tooltips = nil,
		allowCoalesce = true,
	}
	local element = script_param_util.buildScriptParamCompSimple({
		scriptParam = paramForUi,
		currentValue = activeModsParamsState:old()[""][key],
		onValueChange = function(value : number)
			local level = math.floor(value)
			local copy = table_util.copy(activeModsParamsState:old())
			copy[""][key] = level
			for __, other in ipairs(otherKeys or {}) do
				copy[""][other] = level
			end
			if not table_util.deepEquals(copy, activeModsParamsState:old()) then
				activeModsParamsState:set(copy)
			end
		end,
		toggleButtonsFlowLayout = false,
		vertical = false,
		addSpacer = false,
	})
	table.insert(settings, {
		title = scriptParam.name,
		description = scriptParam.tooltip .. "

Big maps: the levels after the game's own give a big map the count Gigantomaniac has at Medium; pick the one that names your size.",
		hintIdKey = "hintIdKey" .. scriptParam.name,
		element = element,
	})
end
"#;

const TOWN_DENSITY: &str = r#"		new_game_react_util.searchBuildAndAddScriptParamComp(
			settingsCiv,
			activeModsParamsState,
			"locations.towns.frequency",
			false, -- addSpacer
			filterTags
		)
"#;

const INDUSTRY_DENSITY: &str = r#"		new_game_react_util.searchBuildAndAddScriptParamComp(
			settingsCiv,
			activeModsParamsState,
			"locations.industry.initialIndustryDensity",
			false, -- addSpacer
			filterTags,
			{"locations.industry.targetIndustryDensity"}
		)
"#;

const NUM_TILES: &str = r#"	local bigmapSize = bigmapNumTiles(format)
	if bigmapSize ~= nil then
		return bigmapSize
	end
"#;

const RENDER: &str = r#"	bigmapRefresh()
	bigmapPickState = react.useState(bigmapPick) as ReactStateT<integer>
	bigmapPick = bigmapPickState:old()
	if bigmapPick > #bigmapRows then
		bigmapPick = 0
	end
"#;

const EDITS: [Edit; 8] = [
    Edit {
        why: "the game's files by absolute path",
        anchor: REQUIRES,
        place: Place::Replace,
        lines: REQUIRES_COPY,
        count: 1,
    },
    Edit {
        why: "big maps' rows, from the mod tpf3mp_bigmap_1",
        anchor: "-- zero-based getter -.-\n",
        place: Place::Before,
        lines: MODULE,
        count: 1,
    },
    Edit {
        why: "an added size row answers for itself",
        anchor: "\tformat = script_param_util.clampScriptParamValueIndex(format, mapFormatParam) - 1\n",
        place: Place::After,
        lines: NUM_TILES,
        count: 1,
    },
    Edit {
        why: "the size dropdown with the added rows",
        anchor: "\t\tnew_game_react_util.addMapSizeSettingsEntry(settingsWorld, activeModsParamsState, filterTags)\n",
        place: Place::Replace,
        lines: "\t\tbigmapAddMapSizeSettingsEntry(settingsWorld, activeModsParamsState, filterTags)\n",
        count: 2,
    },
    Edit {
        why: "big maps' density levels",
        anchor: "-- zero-based getter -.-
",
        place: Place::Before,
        lines: DENSITY,
        count: 1,
    },
    Edit {
        why: "the town density slider with big maps' levels",
        anchor: TOWN_DENSITY,
        place: Place::Replace,
        lines: "		bigmapAddDensitySettingsEntry(settingsCiv, activeModsParamsState, \"locations.towns.frequency\", filterTags, nil)
",
        count: 1,
    },
    Edit {
        why: "the industry density slider with big maps' levels",
        anchor: INDUSTRY_DENSITY,
        place: Place::Replace,
        lines: "		bigmapAddDensitySettingsEntry(settingsCiv, activeModsParamsState, \"locations.industry.initialIndustryDensity\", filterTags, {\"locations.industry.targetIndustryDensity\"})
",
        count: 1,
    },
    Edit {
        why: "the added row picked is page state, so a pick redraws the page and its preview",
        anchor: "local NewGameOrMapSettingsPage = react.RegisterRecipe(\"NewGameOrMapSettingsPage\", function(createNewMapPageParams : MenuPageParam<CreateNewMapParam>) : TreeNodeId\n",
        place: Place::After,
        lines: RENDER,
        count: 1,
    },
];

/// The byte offsets where `anchor` starts a line of `text`.
fn line_matches(text: &str, anchor: &str) -> Vec<usize> {
    text.match_indices(anchor)
        .map(|(at, _)| at)
        .filter(|&at| at == 0 || text.as_bytes()[at - 1] == b'\n')
        .collect()
}

/// The leading whitespace of the first line of `lines`.
fn indent(lines: &str) -> &str {
    let end = lines
        .find(|c: char| c != '\t' && c != ' ')
        .unwrap_or(lines.len());
    &lines[..end]
}

/// The marked block `edit` puts in place of `anchor`.
fn block(edit: &Edit) -> String {
    // The markers line up with the code they mark.
    let pad = match edit.place {
        Place::After => indent(edit.lines),
        Place::Before | Place::Replace => indent(edit.anchor),
    };
    let mut out = format!("{pad}{BEGIN}{}\n", edit.why);
    match edit.place {
        Place::Before => {
            out.push_str(edit.lines);
            out.push_str(&format!("{pad}{END}\n"));
            out.push_str(edit.anchor);
        }
        Place::After => {
            let mut after = String::from(edit.anchor);
            after.push_str(&out);
            after.push_str(edit.lines);
            after.push_str(&format!("{pad}{END}\n"));
            return after;
        }
        Place::Replace => {
            out.push_str(edit.lines);
            out.push_str(&format!("{pad}{WAS}\n"));
            for line in edit.anchor.lines() {
                out.push_str("--");
                out.push_str(line);
                out.push('\n');
            }
            out.push_str(&format!("{pad}{END}\n"));
        }
    }
    out
}

/// The mod's copy, from the game's file. Fails if an anchor is not where it
/// was: a game patch moved what the copy changes, and a person must look.
pub fn build(game: &str) -> Result<String, PageError> {
    let mut text = game.to_owned();
    for edit in &EDITS {
        let found = line_matches(&text, edit.anchor);
        if found.len() != edit.count {
            return Err(PageError::Anchor {
                why: edit.why,
                found: found.len(),
                expected: edit.count,
            });
        }
        let replacement = block(edit);
        // From the last match back, so earlier offsets stay valid.
        for at in found.into_iter().rev() {
            text.replace_range(at..at + edit.anchor.len(), &replacement);
        }
    }
    Ok(text)
}

/// The game's file the copy was made from: the copy without its marked
/// blocks, the lines each block replaced put back.
pub fn original(copy: &str) -> Result<String, PageError> {
    #[derive(PartialEq)]
    enum State {
        Game,
        Copy,
        Was,
    }
    let mut state = State::Game;
    let mut out = String::with_capacity(copy.len());
    for (index, line) in copy.split_inclusive('\n').enumerate() {
        let number = index + 1;
        let marker = line.trim_start().trim_end_matches('\n');
        let problem = |problem| PageError::Block {
            line: number,
            problem,
        };
        if marker.starts_with(BEGIN) {
            if state != State::Game {
                return Err(problem("a block begins inside a block"));
            }
            state = State::Copy;
        } else if marker == WAS {
            if state != State::Copy {
                return Err(problem("\"was\" outside a block"));
            }
            state = State::Was;
        } else if marker == END {
            if state == State::Game {
                return Err(problem("a block ends that never began"));
            }
            state = State::Game;
        } else {
            match state {
                State::Game => {
                    if line.contains("TPF3-MP") {
                        return Err(problem("a change outside a marked block"));
                    }
                    out.push_str(line);
                }
                State::Copy => {}
                State::Was => match line.strip_prefix("--") {
                    Some(game_line) => out.push_str(game_line),
                    None => return Err(problem("a replaced line without its \"--\"")),
                },
            }
        }
    }
    if state != State::Game {
        return Err(PageError::Block {
            line: copy.lines().count(),
            problem: "the last block never ends",
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in for the game's file: every anchor, in order.
    fn game() -> String {
        format!(
            "{REQUIRES}{TOWN_DENSITY}{INDUSTRY_DENSITY}\n-- zero-based getter -.-\nlocal function getNumTiles(size : integer, format : integer) : Vec2i\n\
             \tformat = script_param_util.clampScriptParamValueIndex(format, mapFormatParam) - 1\n\
             \treturn api.type.Vec2i.new(16, 16)\nend\n\
             \t\tnew_game_react_util.addMapSizeSettingsEntry(settingsWorld, activeModsParamsState, filterTags)\n\
             \t\tnew_game_react_util.addMapSizeSettingsEntry(settingsWorld, activeModsParamsState, filterTags)\n\
             local NewGameOrMapSettingsPage = react.RegisterRecipe(\"NewGameOrMapSettingsPage\", function(createNewMapPageParams : MenuPageParam<CreateNewMapParam>) : TreeNodeId\n\
             end)\n"
        )
    }

    #[test]
    fn the_copy_gives_back_the_game_file() {
        let game = game();
        let copy = build(&game).unwrap();
        assert_ne!(copy, game);
        assert_eq!(original(&copy).unwrap(), game);
        assert_eq!(copy.matches(BEGIN).count(), 9, "eight edits, one twice");
        assert!(copy.contains("\t\tbigmapAddMapSizeSettingsEntry("));
        assert!(copy.contains(
            "\t\tbigmapAddDensitySettingsEntry(settingsCiv, activeModsParamsState, \"locations.towns.frequency\""
        ));
        assert!(copy.contains("--\t\tnew_game_react_util.addMapSizeSettingsEntry("));
    }

    #[test]
    fn a_moved_anchor_fails_loudly() {
        let game = game().replace("-- zero-based getter -.-\n", "");
        assert_eq!(
            build(&game),
            Err(PageError::Anchor {
                why: "big maps' rows, from the mod tpf3mp_bigmap_1",
                found: 0,
                expected: 1
            })
        );
        // A commented-out anchor is not the anchor.
        let game = game.replace(
            "\t\tnew_game_react_util.addMapSizeSettingsEntry(settingsWorld, activeModsParamsState, filterTags)\n",
            "--\t\tnew_game_react_util.addMapSizeSettingsEntry(settingsWorld, activeModsParamsState, filterTags)\n",
        );
        assert!(matches!(build(&game), Err(PageError::Anchor { .. })));
    }

    #[test]
    fn a_change_outside_a_block_is_refused() {
        let copy = build(&game()).unwrap();
        let stray = copy.replace("end)\n", "end) -- TPF3-MP: stray\n");
        assert!(matches!(
            original(&stray),
            Err(PageError::Block {
                problem: "a change outside a marked block",
                ..
            })
        ));
        let unclosed = copy.trim_end().to_owned() + "\n-- TPF3-MP begin: open\n";
        assert!(matches!(
            original(&unclosed),
            Err(PageError::Block {
                problem: "the last block never ends",
                ..
            })
        ));
    }
}
