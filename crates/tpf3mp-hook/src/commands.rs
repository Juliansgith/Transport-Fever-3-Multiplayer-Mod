//! The command-capture policy: what the hook does with each native
//! Transport Fever 3 command it sees at `CommandList::Add`.
//!
//! Every player or script action becomes a `Command` whose kind is an
//! integer selector (a byte in the command's payload, dispatched by the
//! apply jump table; see `investigation/TPF3_RECON_2026-09-29.md`). This
//! module is the pure decision the detour consults for each one; it reads
//! and patches nothing. The detour itself (in the game process) is separate
//! and is not here.
//!
//! The rule is **fail closed** (docs/PLAN.md, Part 3): a command is
//! [`Disposition::Capture`]d and sent to the room only when it is a player
//! action the room already understands, [`Disposition::Pass`]ed to run
//! locally only when it changes nothing the room must agree on, and
//! otherwise [`Disposition::Refuse`]d -- cancelled at `CommandList::Add`
//! rather than let diverge the world. The captured set here is deliberately
//! small and conservative; **which commands a multiplayer game allows is
//! the owner's to widen** (docs/DECISIONS.md), by moving kinds from
//! `Refuse` to `Capture` (with an [`ActionKind`] and the decode) or to
//! `Pass`.
//!
//! `CommandKind`'s integer values are not fixed here: they are the apply
//! dispatcher's jump-table order for a given build, resolved on the native
//! side. This module keys on identity, not the integer.

/// A native TF3 command, named by its `api.cmd.make*Cmd` factory (the 61
/// factories the API declares, plus `SimPersonSetHappiness`; see
/// `investigation/TF3_OFFICIAL_API_2026-09-29.md`). Order is the API's, not
/// the build's jump-table order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandKind {
    // World building and terrain.
    WorldBuildProposal,
    WorldReplaceTerrain,
    WorldSetBulldozable,
    WorldChangeWind,
    // Vehicles.
    VehicleBuy,
    VehicleReplace,
    VehicleSell,
    VehicleReverse,
    VehicleSendToDepot,
    VehicleSetLine,
    VehicleSetManualDeparture,
    VehicleSetStoppedByUser,
    VehicleSetModifiers,
    VehicleTryToDepart,
    CustomVehicleCreateOrUpdate,
    // Lines.
    LineCreate,
    LineUpdate,
    LineDestroy,
    // Players and companies.
    GameAddPlayer,
    EntitySetPlayer,
    // Time and simulation (the room's clock; server-driven).
    GameSetSpeed,
    GameSetCalendarSpeed,
    GameSetDate,
    GameSetTimeOfDay,
    GameSetCloudCoverage,
    // Towns.
    TownCreate,
    TownDestroy,
    TownDevelopAt,
    TownSetDevelopmentActive,
    TownSetInitialLandUseCapacities,
    TownUpdateSize,
    TownUpdateCargoNeeds,
    TownConnectWithIndustries,
    TownAutoDetectConnections,
    TownCustomDistributionWeights,
    TownBuildingSetBlockedDevelopment,
    // Industries and stocks.
    IndustrySetManualDevelopment,
    IndustrySetDespawnTime,
    CreateIndustryExtendProposal,
    StockListSetModifiers,
    StockListSetStocksCargoType,
    StockListDiscardCargo,
    // Money, journal, maintenance.
    JournalBookAsset,
    JournalLogEntry,
    JournalClearAll,
    MaintenanceCostUpdate,
    ClearLogbooks,
    // Naming, colour, emissions, components.
    EntitySetName,
    EntitySetColor,
    EntitySetEmissions,
    ComponentExchange,
    // Scripting, custom entities, sim people, animals.
    ScriptingSendEvent,
    CustomEntityCreate,
    CustomEntityDestroy,
    CustomEntityUpdateState,
    CustomEntityUpdateTransformation,
    SimPersonSetState,
    SimPersonSetHappiness,
    StockSetCargoAmount,
    AnimalSpawnAt,
    AnimalSetState,
    // The lockstep step-budget primitive (docs/HOOKS.md): the room drives
    // it; a player never issues it.
    GamePerformSimulationSteps,
}

/// Every command kind, for exhaustive handling and tests.
pub const ALL: &[CommandKind] = {
    use CommandKind::*;
    &[
        WorldBuildProposal,
        WorldReplaceTerrain,
        WorldSetBulldozable,
        WorldChangeWind,
        VehicleBuy,
        VehicleReplace,
        VehicleSell,
        VehicleReverse,
        VehicleSendToDepot,
        VehicleSetLine,
        VehicleSetManualDeparture,
        VehicleSetStoppedByUser,
        VehicleSetModifiers,
        VehicleTryToDepart,
        CustomVehicleCreateOrUpdate,
        LineCreate,
        LineUpdate,
        LineDestroy,
        GameAddPlayer,
        EntitySetPlayer,
        GameSetSpeed,
        GameSetCalendarSpeed,
        GameSetDate,
        GameSetTimeOfDay,
        GameSetCloudCoverage,
        TownCreate,
        TownDestroy,
        TownDevelopAt,
        TownSetDevelopmentActive,
        TownSetInitialLandUseCapacities,
        TownUpdateSize,
        TownUpdateCargoNeeds,
        TownConnectWithIndustries,
        TownAutoDetectConnections,
        TownCustomDistributionWeights,
        TownBuildingSetBlockedDevelopment,
        IndustrySetManualDevelopment,
        IndustrySetDespawnTime,
        CreateIndustryExtendProposal,
        StockListSetModifiers,
        StockListSetStocksCargoType,
        StockListDiscardCargo,
        JournalBookAsset,
        JournalLogEntry,
        JournalClearAll,
        MaintenanceCostUpdate,
        ClearLogbooks,
        EntitySetName,
        EntitySetColor,
        EntitySetEmissions,
        ComponentExchange,
        ScriptingSendEvent,
        CustomEntityCreate,
        CustomEntityDestroy,
        CustomEntityUpdateState,
        CustomEntityUpdateTransformation,
        SimPersonSetState,
        SimPersonSetHappiness,
        StockSetCargoAmount,
        AnimalSpawnAt,
        AnimalSetState,
        GamePerformSimulationSteps,
    ]
};

/// Which `tpf3mp_proto::Action` a captured command becomes. The decode from
/// the native command's fields to the action's data is separate, native,
/// and per command (it reads the live `Command` payload); this only names
/// the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    /// A `WorldBuildProposal`: which of `BuildRoad`, `BuildTrack`,
    /// `Bulldoze`, `BuildConstruction` or `Terraform` it is depends on the
    /// proposal's shape, read at decode (docs/BUILDING.md).
    BuildProposal,
    BuyVehicle,
    SellVehicle,
    CreateLine,
    EditLine,
    AssignLine,
}

/// What the hook does with a command at `CommandList::Add`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// A player action: decode it, send it to the room, and cancel the
    /// native command (the room replays it in order).
    Capture(ActionKind),
    /// Not shared state: let it run natively, do not replicate. (None yet;
    /// kept for kinds a later decision moves here, e.g. a purely local or
    /// cosmetic command.)
    Pass,
    /// Fail closed: not a handled multiplayer action, so cancel it and tell
    /// the player. The default for everything not explicitly captured.
    Refuse,
}

/// The hook's disposition for a command kind. Conservative by default: only
/// the player actions the room already models are captured; everything else
/// is refused until a decision widens the set (see the module comment).
pub const fn disposition(kind: CommandKind) -> Disposition {
    use ActionKind as A;
    use CommandKind as C;
    match kind {
        C::WorldBuildProposal => Disposition::Capture(A::BuildProposal),
        C::VehicleBuy => Disposition::Capture(A::BuyVehicle),
        C::VehicleSell => Disposition::Capture(A::SellVehicle),
        C::VehicleSetLine => Disposition::Capture(A::AssignLine),
        C::LineCreate => Disposition::Capture(A::CreateLine),
        C::LineUpdate => Disposition::Capture(A::EditLine),
        // Everything else fails closed: it is not yet a handled multiplayer
        // action. This includes the clock commands (server-driven through
        // the control channel, not free local commands), the player/company
        // commands (companies mode is undecided), and every town, industry,
        // stock, journal, naming, scripting, custom-entity, person and
        // animal command.
        _ => Disposition::Refuse,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_appears_once_in_all() {
        // ALL is the exhaustive list the native side iterates.
        for kind in ALL {
            assert_eq!(
                ALL.iter().filter(|k| *k == kind).count(),
                1,
                "{kind:?} is listed more than once"
            );
        }
        // 61 API factories + SimPersonSetHappiness.
        assert_eq!(ALL.len(), 62);
    }

    #[test]
    fn the_captured_set_is_exactly_the_modelled_player_actions() {
        let mut captured: Vec<CommandKind> = ALL
            .iter()
            .copied()
            .filter(|k| matches!(disposition(*k), Disposition::Capture(_)))
            .collect();
        let mut expected = vec![
            CommandKind::WorldBuildProposal,
            CommandKind::VehicleBuy,
            CommandKind::VehicleSell,
            CommandKind::VehicleSetLine,
            CommandKind::LineCreate,
            CommandKind::LineUpdate,
        ];
        // Order-independent: sort both by their position in ALL.
        let pos = |k: &CommandKind| ALL.iter().position(|a| a == k).unwrap();
        captured.sort_by_key(pos);
        expected.sort_by_key(pos);
        assert_eq!(captured, expected);
    }

    #[test]
    fn everything_not_captured_fails_closed() {
        for kind in ALL {
            match disposition(*kind) {
                Disposition::Capture(_) => {}
                Disposition::Pass => panic!("{kind:?} passes; nothing should pass yet"),
                Disposition::Refuse => {}
            }
        }
        // The clock and player commands specifically fail closed for now.
        for kind in [
            CommandKind::GameSetSpeed,
            CommandKind::GameAddPlayer,
            CommandKind::EntitySetPlayer,
            CommandKind::GamePerformSimulationSteps,
            CommandKind::ScriptingSendEvent,
        ] {
            assert_eq!(disposition(kind), Disposition::Refuse, "{kind:?}");
        }
    }
}
