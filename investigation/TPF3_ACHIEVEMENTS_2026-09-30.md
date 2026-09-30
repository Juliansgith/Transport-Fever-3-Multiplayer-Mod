# Achievements with TPF3-MP active (build 40408, 2026-09-30)

Transport Fever 3 turns achievements off for a save whose mods it does not
trust. A mod can keep them on with `"forceActivateAchievements": true` in
its `mod.json`. TPF3-MP now sets it.

## How the game decides

- **The menus ask the mod repository.** The new-game page shows "Achievements
  cannot be earned." when `modRep:couldAchievementsBeEarned(modIdList)` is
  false (`gui/menu/new_game_or_map_settings_page.tl` line 883, in
  `base/content/gui.zip`). The load page re-checks a save marked
  unachievable when `modRep:wouldForceActivateAchievements(mods)` is true
  (`gui/menu/savegame_react_util.tl` lines 1063 to 1064). SEEN in code.
- **The rule is native.** `couldAchievementsBeEarned` is bound to
  `sub_2f929f0` (`framework/mod/modrep.cpp`, bound in `sub_16c13b0` at
  0x16c59e1). For each mod of the list the game knows, it reads the mod's
  description:
  1. any mod with the byte at +0x7b set: true;
  2. otherwise true only if every known mod has the byte at +0x79 set.

  Unknown mods are skipped. SEEN in the disassembly.
- **The bytes are `mod.json` keys.** The manifest parser `sub_2f238e0` reads
  `visible` into +0x78, `cosmetic` into +0x79, `autoActivate` into +0x7a and
  `forceActivateAchievements` into +0x7b. SEEN in the disassembly.
- **The DLCs are cosmetic.** `dlcs/urbangames_deluxe_upgrade_pack/mod.json`
  sets `"cosmetic": true` and `"autoActivate": true`, so a save with only
  DLCs keeps achievements. SEEN.

## Why the force flag, not `cosmetic`

- **Nothing else reads +0x7b.** A byte-pattern search of the code finds it
  read only by `couldAchievementsBeEarned` and `wouldForceActivateAchievements`
  (`sub_2f95000`). So the flag changes nothing but achievements. SEEN, for
  the `cmp byte ptr [reg+0x7b], 0` forms searched.
- **`cosmetic` has other readers.** They were not traced. It might change
  how the game treats the mod elsewhere, such as whether a save records it,
  which a room's world depends on. INFERRED risk.
- **The flag is broad.** With it, any save that includes TPF3-MP earns
  achievements, even if other mods that change the game are active too.
  That is how the game's own flag works; TPF3-MP cannot narrow it.

## Not yet seen in the game

- **The warning.** The "Achievements cannot be earned." tape on the new-game
  and load pages should be gone for a save with TPF3-MP.
- **An unlock.** An achievement should unlock in a room's game. The game
  script also checks `api.gui.achievements.areStatsDisabled()` at runtime
  (`game_mechanics/achievements/achievements.script.tl` lines 510 and 809);
  whether that follows the same rule is INFERRED.
