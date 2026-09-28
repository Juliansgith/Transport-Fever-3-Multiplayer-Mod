# The minimap

A map of the whole world in a window: where the towns and industries are,
the roads, tracks and stations, whose they are, and where the camera is.
Click to go there. It is TPF2 Big Maps' minimap (`tpf2-bigmap`,
`docs/minimap.md`, by silver2127), brought to Transport Fever 3 and to
TPF3-MP's companies. This page is its spec; nothing of it is built yet
(PLAN.md, Part 4).

## What the player sees

A **Minimap** button on the game's bottom bar
(`gui/tpf3mp/icons/button_minimap*.tga`, TF3's disc style; selected while
the window is open) opens a window, as TPF2's did:

- **The map**: the terrain (land by height, water by depth, lightly
  shaded), the road and track network, stations, towns and industries, and
  a circle where the camera is. Clicking or dragging on it moves the
  camera there.
- **Show**: toggles for Towns, Industries, Roads, Tracks, Stations and
  Camera.
- **Companies**: one row per company, in its colour, with a toggle; the
  player's own first, marked "(you)". In a single-player game, one row.
- **Industry types**: one row per type, with the icon of the cargo it
  produces, how many there are, and a toggle.
- **Refresh**: redraws the network after building.

In a TPF3-MP room it also shows, each with a toggle:

- **Other players' cameras**, as small circles in their company's colour,
  from the cursors TpF2 Multiplayer already shared (`mp/cursors.lua`):
  advisory, over QUIC datagrams, never part of the world.
- **What others are building**, as the markers
  (`gui/tpf3mp/icons/marker_*.tga`) TpF2 Multiplayer drew for other
  players' previews.

## How it is made

TF3's interface is script (Teal, react-style; see
investigation/TF3_MODS_2026-09-27.md), so the minimap is a game bar plugin
in the TPF3-MP mod, like its loader, and works in every game, with or
without the hook. What only a room has (companies' colours from the
server, others' cameras and builds) comes over the link when the hook is
there.

In two steps, so something useful ships early:

1. **Script only.** Everything but the terrain picture:
   - towns and industries as markers, placed from
     `api.engine.getEntitiesWithComponent` (TOWN, INDUSTRY) and their
     constructions' positions;
   - roads, tracks and stations as lines;
   - the camera circle, click to move;
   - the map's extent by binary search on `api.engine.terrain.
     isValidCoordinate`, as TPF2's did, not a scan: big maps are
     ±65 km;
   - the network read on a per-frame budget ("Loading network N%"), never
     one component per road segment. If TF3's GUI can draw lines only as
     components, draw each road type as few polylines as possible, and cap
     them.
2. **The terrain picture, natively.** As TPF2 Big Maps did: the script
   asks for a picture of a world rectangle (a token in an image path),
   the hook renders the heights and colours natively on a few threads and
   hands the image to the GUI. This needs the hook, so it is only drawn in
   a game the launcher started (D11); elsewhere the map shows a plain
   background under the same markers and lines. Its hook targets join the
   profile's list (HOOKS.md) and fail closed like the others: without
   them, the picture is off, not wrong.

TPF2's lessons, kept:

- One definition of the world-to-picture mapping, in the script, shared by
  the picture, the markers and the clicks.
- UI units and screen pixels are different in TPF2's GUI; measure the UI
  scale rather than assume it. Check TF3's layout rules the same way, and
  log what was found.
- Colours per company from one algorithm, shared with the launcher and
  the in-game room panel (D17), so a company looks the same everywhere.
- An industry type's cargo from what its industries produce, then its
  construction file, then a built-in table of the stock ones.

## To find out on release day

- Whether TF3's GUI has a canvas or line view (TPF2 had `LineRenderView`)
  and an image view that takes raw pixels (TPF2's `ImageView::SetImage`),
  or only components. `grep` the game's `.tl` GUI sources and `.d.tl`
  declarations (DAY_ONE.md §3).
- The camera: how to read its position and move it from script
  (TPF2: `gui.getCamera`, `gui.setCamera`).
- Station and edge components: `BASE_EDGE`, `BASE_NODE` and `STATION`
  are TPF2 names, not yet seen in a TF3 mod.
- Whether the stock game already has a minimap. If it has a good one,
  add TPF3-MP's layers to it instead of drawing another.
