-- tools/probe/tf3/tpf3mp_buildprobe_1: what Transport Fever 3's native
-- build tools tell game scripts, and whether a game script can stop a
-- build (docs/HOOKS.md, "The player's commands").
--
-- The game's own scripts show the native tools (streetBuilder,
-- trackBuilder, constructionBuilder, streetTerminalBuilder, bulldozer)
-- sending game scripts' guiHandleEvent `builder.proposalCreate` on every
-- preview, `builder.proposalPrepareForApply` on the click and
-- `builder.proposalApply` after it, with param { Proposal, ProposalData,
-- [entities] }; the company script refuses a proposal by returning
-- { errorMessages = { [text] = true } } from proposalCreate. The simulation
-- side hears `apply_command` onPreBuildProposal/onPostBuildProposal.
--
-- This probe logs each event with the proposal's shape (the first few
-- proposalCreate of each tool only, and a construction proposal's first
-- entry field by field), and returns an error from the street builder's
-- proposalPrepareForApply.
--
-- Found with it on build 40408: the street builder sends no
-- proposalPrepareForApply (a click is proposalCreate twice, then the
-- simulation's onPreBuildProposal, onPostBuildProposal, then the GUI's
-- proposalApply), and nothing a game script does stops the build (an error
-- raised in onPreBuildProposal is logged and the build goes on; emptying
-- the proposal's lists there crashes the game). The hook stops the player's
-- builds natively (crates/tpf3mp-hook/src/builds.rs).
--
-- Output: the TPF3-MP hook's log (tpf3mp_native.log) in a game its
-- launcher started, else the game's log, as "[tpf3mp-probe build] ...".
function data()
	local printed = false
	local function log(line)
		if not printed then
			printed = true
			pcall(print, "[tpf3mp-probe build] looking for TPF3-MP's hook")
		end
		local text = "[tpf3mp-probe build] " .. line
		local ok, native = pcall(function() return tpf3mp_native end)
		if ok and type(native) == "table" and type(native.log) == "function" then
			if pcall(native.log, text) then return end
		end
		pcall(debugPrint, text)
	end

	local function get(value, key)
		local ok, v = pcall(function() return value[key] end)
		if ok then return v end
		return nil
	end

	local function count(list)
		if list == nil then return "nil" end
		local ok, n = pcall(function() return #list end)
		if ok then return tostring(n) end
		return "?"
	end

	local function vec(v)
		if v == nil then return "nil" end
		local x, y, z = get(v, "x"), get(v, "y"), get(v, "z")
		if x == nil then x, y, z = get(v, 1), get(v, 2), get(v, 3) end
		return string.format("(%s,%s,%s)", tostring(x), tostring(y), tostring(z))
	end

	-- The proposal's shape: its lists' lengths and its first new node,
	-- segment and construction.
	local function describe(proposal)
		if proposal == nil then return "no proposal" end
		local street = get(proposal, "proposal")
		local parts = {
			"type=" .. type(proposal),
			"addedNodes=" .. count(street and get(street, "addedNodes")),
			"addedSegments=" .. count(street and get(street, "addedSegments")),
			"removedSegments=" .. count(street and get(street, "removedSegments")),
			"removedNodes=" .. count(street and get(street, "removedNodes")),
			"edgeObjectsToAdd=" .. count(street and get(street, "edgeObjectsToAdd")),
			"toAdd=" .. count(get(proposal, "toAdd")),
			"toRemove=" .. count(get(proposal, "toRemove")),
		}
		local nodes = street and get(street, "addedNodes")
		local node = nodes and get(nodes, 1)
		if node then
			local comp = get(node, "comp")
			parts[#parts + 1] = "node1{entity=" .. tostring(get(node, "entity"))
				.. " position=" .. vec(comp and get(comp, "position")) .. "}"
		end
		local segments = street and get(street, "addedSegments")
		local segment = segments and get(segments, 1)
		if segment then
			local comp = get(segment, "comp")
			local se = get(segment, "streetEdge")
			parts[#parts + 1] = "segment1{entity=" .. tostring(get(segment, "entity"))
				.. " type=" .. tostring(get(segment, "type"))
				.. " node0=" .. tostring(comp and get(comp, "node0"))
				.. " node1=" .. tostring(comp and get(comp, "node1"))
				.. " tangent0=" .. vec(comp and get(comp, "tangent0"))
				.. " tangent1=" .. vec(comp and get(comp, "tangent1"))
				.. " typeIndex=" .. tostring(comp and get(comp, "typeIndex"))
				.. " streetType=" .. tostring(se and get(se, "streetType"))
				.. " roadTemplate=" .. tostring(comp and get(comp, "roadTemplate"))
				.. "}"
		end
		local toAdd = get(proposal, "toAdd")
		local con = toAdd and get(toAdd, 1)
		if con then
			parts[#parts + 1] = "con1{fileName=" .. tostring(get(con, "fileName")) .. "}"
		end
		return table.concat(parts, " ")
	end

	local function describeData(data)
		if data == nil then return "no data" end
		local errorState = get(data, "errorState")
		local costs = get(data, "costs")
		return "critical=" .. tostring(errorState and get(errorState, "critical"))
			.. " messages=" .. count(errorState and get(errorState, "messages"))
			.. " costs=" .. tostring(costs)
	end

	-- A construction proposal's first entry, field by field: what a capture
	-- can read of it.
	local function dumpConstruction(proposal)
		local toAdd = get(proposal, "toAdd")
		local con = toAdd and get(toAdd, 1)
		if not con then return "no toAdd[1]" end
		local out = {}
		for _, key in ipairs({ "fileName", "name", "playerEntity", "hasCargoPlatform" }) do
			out[#out + 1] = key .. "=" .. tostring(get(con, key))
		end
		local t = get(con, "transf")
		if t then
			local cells = {}
			for i = 1, 16 do cells[#cells + 1] = tostring(get(t, i)) end
			out[#out + 1] = "transf=" .. table.concat(cells, ",")
		end
		for _, part in ipairs({ "construction", "desc" }) do
			local v = get(con, part)
			out[#out + 1] = part .. "=" .. type(v)
			if v ~= nil then
				for _, key in ipairs({ "fileName", "name", "params", "transf", "frozenNodes", "frozenEdges" }) do
					local f = get(v, key)
					if f ~= nil then
						local text = tostring(f)
						if type(f) == "table" then
							local keys = {}
							for k, value in pairs(f) do
								keys[#keys + 1] = tostring(k) .. ":" .. type(value) .. "=" .. tostring(value)
								if #keys >= 25 then break end
							end
							text = "{" .. table.concat(keys, " ") .. "}"
						end
						out[#out + 1] = part .. "." .. key .. "=" .. text
					end
				end
			end
		end
		local params = get(con, "params")
		if params ~= nil then out[#out + 1] = "params=" .. tostring(params) end
		return table.concat(out, " | ")
	end

	local creates = {}

	return {
		update = function(_params, state, _dt)
			if not state:hasEventSubscriptions() then
				state:subscribeToAllEvents()
				log("subscribed to all events")
			end
		end,

		handleEvent = function(_params, _state, src, id, name, param)
			if id ~= "apply_command" then return end
			log("sim " .. tostring(name) .. " from " .. tostring(src)
				.. " playerInitiated=" .. tostring(get(param, 4))
				.. " " .. describe(get(param, 1)))
			-- Found on build 40408: nothing here stops the build. An error
			-- raised here is logged and the build goes on; emptying the
			-- proposal's lists here crashes the game. The hook stops the
			-- player's builds natively (crates/tpf3mp-hook/src/builds.rs).
		end,

		guiHandleEvent = function(_params, _state, _guiState, src, id, name, param)
			if type(name) ~= "string" or name:sub(1, 8) ~= "builder." then return end
			if name == "builder.proposalCreate" then
				creates[id] = (creates[id] or 0) + 1
				if creates[id] > 3 then return end
			else
				creates[id] = 0
			end
			log("gui " .. tostring(id) .. " " .. name .. " from " .. tostring(src) .. ": "
				.. describe(get(param, 1)) .. " | " .. describeData(get(param, 2))
				.. " | entities=" .. count(get(param, 3)))
			if id == "constructionBuilder" then
				log("construction " .. dumpConstruction(get(param, 1)))
			end
			if name == "builder.proposalPrepareForApply" and id == "streetBuilder" then
				log("refusing the street build at proposalPrepareForApply")
				return { errorMessages = { ["TPF3-MP probe: street builds are refused"] = true } }
			end
		end,
	}
end
