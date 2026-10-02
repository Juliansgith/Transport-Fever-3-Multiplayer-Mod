-- New channels remain refused until ordinary two-game acceptance is recorded.
-- These defaults travel with the mod fingerprint: every room member has the
-- same settings. Fixtures may enable a channel explicitly to test its mechanics.
local acceptance = { subsidies = false, rename = false, waypoints = false,
    notifications = false, discard = false }

function acceptance.check(action)
    local feature
    if action.Subsidy then feature = "subsidies" end
    if action.Rename or (action.VehicleOp and type(action.VehicleOp.change) == "table"
        and action.VehicleOp.change.Recolor) then feature = "rename" end
    -- The notification log is the room's, one for all its players: one
    -- player's dismiss or ignore list is every player's.
    if action.Notification then feature = "notifications" end
    if action.DiscardCargo then feature = "discard" end
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
