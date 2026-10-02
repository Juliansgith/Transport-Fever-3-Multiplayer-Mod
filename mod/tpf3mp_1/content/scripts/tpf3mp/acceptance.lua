-- New channels remain refused until ordinary two-game acceptance is recorded.
-- These defaults travel with the mod fingerprint: every room member has the
-- same settings. Fixtures may enable a channel explicitly to test its mechanics.
-- `bridges`: the bridge and tunnel window's in-place rebuild, which travels as
-- an ordinary road or track build and so is gated where it is captured
-- (capture.windowBuild).
local acceptance = { subsidies = false, rename = false, waypoints = false, bridges = false }

function acceptance.check(action)
    local feature
    if action.Subsidy then feature = "subsidies" end
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
