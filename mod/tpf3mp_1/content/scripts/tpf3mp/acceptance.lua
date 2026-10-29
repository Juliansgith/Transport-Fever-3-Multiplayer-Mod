-- New channels remain refused until ordinary two-game acceptance is recorded.
-- These defaults travel with the mod fingerprint: every room member has the
-- same settings. Fixtures may enable a channel explicitly to test its mechanics.
-- Height brushes passed local two-game acceptance on build 40408, 2026-10-02.
-- See investigation/STATION_TERRAIN_2026-10-02.md for evidence and limits.
-- Bridge/tunnel window rebuilds use ordinary road/track actions, gated at
-- capture.windowBuild; they passed local two-game acceptance on build
-- 40420, 2026-10-11 (track and road bridges, a road tunnel, a road bridge
-- with a stop on it): investigation/TPF3_BRIDGE_TUNNEL_WINDOW_2026-10-11.md.
-- Signals a script places along tracks (PlaceSignals: Auto Signals) passed
-- local two-game acceptance on build 40408, 2026-10-06 (runs
-- run-1006-225251 and run-1006-230635; docs/MODS.md).
-- Signals kept on the pieces of a track cut between them (splitSignals:
-- a track started or ended there, Link.kept) passed local two-game
-- acceptance on build 40420, 2026-10-17 (run-1017-024110, Japan_2: two
-- cuts by a moved node, each signal applied in both games and moved
-- 0.00 m, every checkpoint's free-id fingerprint equal).
-- A signal the track tool puts back where it stood (refitSignals: a track
-- started close to a node of a track with signals, the tool replacing the
-- signal on the edge whose node it removes) is captured as the old signal
-- kept on that piece, a split's Link.kept on the wire; only the capture
-- asks this gate (tpf3mp/engine.lua refitSignals). It passed two-player
-- acceptance on build 40420, 2026-10-17 (run-1017-refit2, Japan_2: one
-- build by each player, a mod's signal and the game's own, each put back
-- on the piece between the tool's own nodes about the removed one, kept in
-- both games and moved 0.00 and 0.01 m, every rolling world check equal).
-- Subsidies passed local two-game acceptance on build 40408, 2026-10-07:
-- investigation/TPF3_SUBSIDIES_2026-10-07.md.
-- Line waypoints passed local two-game acceptance on build 40420, 2026-10-16:
-- a line edited with waypoints in a room, both games' rolling world checks alike.
-- Terrain paint and the asset brush (action schema 27) passed local two-game
-- acceptance on build 40420, 2026-10-16: paint strokes of several sizes, the
-- large ones in bands, and tree strokes of 78 and 124 trees, alike in both.
-- The crossing type-only path has staged Japan_2 runs, but not ordinary-start
-- two-player acceptance yet; keep it refused until that run is recorded.
-- The road and track tools' toolbar crossing type (BuildRoad/BuildTrack
-- crossing_type, schema 28) passed staged two-game runs on build 40420,
-- 2026-10-17, on Japan_2 (crossing-tool-1017b, crossing-tool-1017c), and
-- ordinary-start two-game acceptance on build 40420, 2026-10-19: two
-- launchers against a local server, the room created, joined by invite and
-- started from the launcher, Japan_2 with its mods; a road across a track and
-- a track across a road, each with a picked type (jp_double_rht,
-- jp_gantry_single_lht), laid alike in both games, every rolling world check
-- equal.
-- Construction rejoin's two native commands have no proven atomic failure
-- path; while off, a refused first proposal cannot remove any existing track.
-- Stops on streets, as tracks' signals go (2026-10-19, refused in a room
-- before, single player does both): bulldozeStops lets the bulldozer remove
-- a street with the stops on it (Bulldoze::Edges, every game removing the
-- stops it finds on those streets; the capture and every game's replay ask
-- it); keptStops lets a street build keep a stop on the piece that goes on
-- from its street or runs where it stood (a stop's Link.kept, its removal a
-- street's). Both passed a staged two-game room on build 40420,
-- 2026-10-19 (run-1019-221901, Japan_2, room.ps1 with both on locally):
-- four street builds beside junctions, one the tool's put-back of a
-- two-sided stop, each kept stop in its station group in both games and
-- moved 0.00 m; a street bulldozed with a stop and one with a two-sided
-- stop, removed in both; every rolling world check equal through step
-- 400. Both passed ordinary-start two-game acceptance on build 40423,
-- 2026-10-22 (gates-40423-a1: two launchers against a local server, the
-- room created, joined by invite and started from the launcher): three
-- streets bulldozed with their stops, two of them two-sided, removed in
-- both games; two street builds beside a junction keeping a two-sided
-- stop, both stops in their station group in both games, moved 0.01 m.
-- Historic Preservation (preservation) and the company perks (perks:
-- Industry Greenification, a marketing campaign) passed in the same run
-- and in gates-40423-a2 (the campaign with the game's infinite money):
-- set and cleared from either game, the campaign started and booked in
-- both, every rolling world check equal.
local acceptance = {
    subsidies = true,
    rename = true,
    waypoints = true,
    terraform = true,
    bridges = true,
    perks = true,
    preservation = true,
    signals = true,
    splitSignals = true,
    refitSignals = true,
    paint = true,
    assets = true,
    crossings = false,
    crossingTypes = true,
    rejoinConstruction = false,
    bulldozeStops = true,
    keptStops = true,
    -- A road, track or bulldozer edit across an industry (a farm's fields,
    -- its buildings, its entrance road). Work in progress, OFF: the capture
    -- and the industry's regenerated replacement work in the game, but the
    -- road laid after it, at the next room step, was laid only sometimes
    -- (build 40425, 2026-10-29). See
    -- investigation/INDUSTRY_EDITS_2026-10-29.md before turning it on.
    industryFields = false,
}

function acceptance.check(action)
    local feature
    -- A stop kept on a street's new piece: its removal is a street's (only
    -- stops stand on streets; signals on tracks). Asked apart from the
    -- feature below, which a later one may replace.
    local polyline = (action.BuildTrack or action.BuildRoad or {}).polyline
        or (action.BuildConstruction or {}).connection
    for _, link in ipairs(polyline and polyline.links or {}) do
        for _, kept in ipairs(link.kept or {}) do
            local removal = type(kept.removal) == "number" and (polyline.removals or {})[kept.removal + 1]
            if type(removal) == "table" and removal.network ~= "Track" and acceptance.keptStops ~= true then
                return false, "keptStops awaits two-player game acceptance"
            end
        end
    end
    if action.Subsidy then feature = "subsidies" end
    if action.Terraform then feature = "terraform" end
    -- Terrain paint (action::TerrainPaint, schema 27).
    if action.Paint then feature = "paint" end
    -- The asset brush (action::AssetStroke, schema 27).
    if action.PlantAssets then feature = "assets" end
    if action.Perk then feature = "perks" end
    -- A town building's Historic Preservation (action::Preservation).
    if action.Preserve then feature = "preservation" end
    if action.PlaceSignals then feature = "signals" end
    -- A track cut where a signal stands, the signal kept on the piece it
    -- stood on (action::KeptSignal).
    for _, link in ipairs(polyline and polyline.links or {}) do
        if link.kept and #link.kept > 0 then feature = "splitSignals" end
    end
    -- A level crossing's type from its window (action::CrossingType).
    if action.SetCrossingType then feature = "crossings" end
    -- The level crossing type a road or track tool's toolbar picked for the
    -- crossings its build lays (RoadBuild.crossing_type, schema 28).
    local build = action.BuildRoad or action.BuildTrack
    if type(build) == "table" and build.crossing_type ~= nil then feature = "crossingTypes" end
    if action.Rename or (action.VehicleOp and type(action.VehicleOp.change) == "table"
        and action.VehicleOp.change.Recolor) then feature = "rename" end
    local line = action.CreateLine and action.CreateLine.line
    if action.EditLine and type(action.EditLine.change) == "table" then line = action.EditLine.change.Update end
    for _, stop in ipairs(line and line.stops or {}) do
        if #(stop.waypoints or {}) > 0 then feature = "waypoints" end
    end
    if feature and acceptance[feature] ~= true then
        return false, feature .. " awaits two-player game acceptance"
    end
    return true
end

return acceptance
