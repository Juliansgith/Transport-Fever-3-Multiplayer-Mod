//! What a rail line costs to build, as the game prices it: the AI weighs a
//! route by these, as a player weighs one by the build tool's price. Read
//! from the game's own verdicts on test pieces (1922, the rig's big
//! fixture, 2026-10-30; investigation, "Track costs"), each 200 m long, and
//! checked against the game's config (`base/mod.script.tl`: earthworks at
//! 4.5 a cubic metre raised, 5.25 lowered; fields 200,000 each to remove).
//! A player's game may scale them (its infrastructure price setting, the
//! years), so the AI keeps a scale it reads off the verdicts it gets.

/// Track on the ground, a metre.
pub const TRACK: f64 = 53.0;
/// A level crossing over its own track's price: the crossing and the street
/// rebuilt through it.
pub const CROSSING: f64 = 9_600.0;
/// A one-track cargo station, a module of it (three, 277,298; six,
/// 561,711), and a rail depot.
pub const STATION_MODULE: f64 = 93_000.0;
pub const DEPOT: f64 = 1_030_000.0;
/// The least height a bridge stands above the ground (the game takes one
/// 0.5 m up), and the least ground over a tunnel (a shallower one costs
/// more: 806 a metre at 5 m, 680 at 7), in metres.
pub const BRIDGE_LEAST: f64 = 2.0;
pub const TUNNEL_LEAST: f64 = 9.0;
/// The last year the trestle bridge is offered (`trestle.bridge.lua`), and
/// the fastest train it suits: its speed limit is 60 km/h.
pub const TRESTLE_UNTIL: i32 = 1970;
pub const TRESTLE_SPEED: u32 = 65;

/// The bridges the AI builds: stone, offered every year, for rail and
/// road; the trestle, for rail until 1970, cheaper and slower.
pub const STONE: &str = "::/infrastructure/bridge/stone.bridge";
pub const TRESTLE: &str = "::/infrastructure/bridge/trestle.bridge";
/// The tunnel for rail offered from the start (`tunnel_a.tunnel.lua`).
pub const TUNNEL: &str = "::/infrastructure/tunnel/tunnel_a.tunnel";

/// The prices a line is planned by.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Prices {
    /// The game's prices over these, read off its verdicts.
    pub scale: f64,
    /// It bridges with trestles: before 1970, for a slow train.
    pub trestle: bool,
}

impl Default for Prices {
    fn default() -> Self {
        Self {
            scale: 1.0,
            trestle: false,
        }
    }
}

impl Prices {
    /// Track `h` metres above the ground (an embankment) or below it (a
    /// cutting), a metre: the track and the earth moved for it, a bed about
    /// 4 m wide with sides as steep as they are high (fill at 5 m: 201 a
    /// metre over the track's 53, the earth 4.5 a cubic metre).
    pub fn ground(&self, h: f64) -> f64 {
        let earth = if h >= 0.0 {
            4.5 * (4.0 + h) * h
        } else {
            5.25 * (3.5 - h) * -h
        };
        self.scale * (TRACK + earth)
    }

    /// A bridge `h` metres above the ground, a metre, if it may stand
    /// there: the stone bridge's verdicts at 2, 3.5, 4.5, 6, 10, 15 and 25 m
    /// (197, 203, 212, 232, 333, 585 and 1625 a metre), or the trestle's at
    /// 6 and 15 (138, 315).
    pub fn bridge(&self, h: f64) -> Option<f64> {
        if h < BRIDGE_LEAST {
            return None;
        }
        let metre = if self.trestle {
            20.0 + 19.7 * h.max(6.0)
        } else if h <= 6.0 {
            190.0 + 7.0 * h
        } else {
            273.0 - 26.0 * h + 3.2 * h * h
        };
        Some(self.scale * metre)
    }

    /// A tunnel `d` metres under the ground, a metre, if the ground over it
    /// is deep enough: its verdicts at 10 and 20 m (605, 819 a metre).
    pub fn tunnel(&self, d: f64) -> Option<f64> {
        (d >= TUNNEL_LEAST).then_some(self.scale * (391.0 + 21.4 * d))
    }

    /// The bridge it builds.
    pub fn bridge_type(&self) -> &'static str {
        if self.trestle { TRESTLE } else { STONE }
    }

    /// A level crossing, over its track.
    pub fn crossing(&self) -> f64 {
        self.scale * CROSSING
    }

    /// A rail line's stations of `modules` modules each and its depot.
    pub fn stations_and_depot(&self, modules: i32) -> f64 {
        self.scale * (2.0 * f64::from(modules) * STATION_MODULE + DEPOT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prices_match_the_games_verdicts() {
        let p = Prices::default();
        // 200 m pieces as the game priced them, within a tenth.
        let near = |got: f64, want: f64| (got * 200.0 - want).abs() <= want * 0.1;
        assert!(near(p.ground(0.0), 10_615.0));
        assert!(near(p.ground(5.0), 50_834.0), "{}", p.ground(5.0) * 200.0);
        assert!(near(p.ground(8.0), 102_975.0), "{}", p.ground(8.0) * 200.0);
        assert!(near(p.ground(-5.0), 54_502.0), "{}", p.ground(-5.0) * 200.0);
        assert!(near(p.bridge(10.0).unwrap(), 66_540.0));
        assert!(near(p.bridge(25.0).unwrap(), 325_070.0));
        assert!(near(p.tunnel(10.0).unwrap(), 121_002.0));
        assert!(near(p.tunnel(20.0).unwrap(), 163_819.0));
        let t = Prices { trestle: true, ..p };
        assert!(near(t.bridge(15.0).unwrap(), 63_010.0));
        // Low bridges, the game's 2026-10-30 verdicts at 2 and 4.5 m.
        assert!(near(p.bridge(2.0).unwrap(), 39_352.0));
        assert!(near(p.bridge(4.5).unwrap(), 42_436.0));
        // Too low for a bridge, too shallow for a tunnel.
        assert!(p.bridge(1.0).is_none() && p.tunnel(5.0).is_none());
    }

    #[test]
    fn a_bridge_beats_a_high_embankment_and_a_tunnel_a_deep_cutting() {
        let p = Prices::default();
        assert!(p.ground(4.0) < p.bridge(6.0).unwrap());
        assert!(p.bridge(10.0).unwrap() < p.ground(10.0));
        assert!(p.ground(-6.0) < p.tunnel(10.0).unwrap() * 1.5);
        assert!(p.tunnel(12.0).unwrap() < p.ground(-12.0));
        // The scale prices everything alike.
        let dear = Prices { scale: 2.0, ..p };
        assert!((dear.ground(3.0) - 2.0 * p.ground(3.0)).abs() < 1e-9);
    }
}
