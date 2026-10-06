//! The game's own `EmissionEmitterSystem::Update2`, run from the mapped
//! executable ([`super::mapped`]) on random worlds, against the hook's
//! splat: every bit of both grids equal.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::Cell;
use std::sync::Mutex;
use std::time::Instant;

use super::mapped::Mapped;
use super::*;
use crate::bigmap::original::Exe;

const UPDATE2_RVA: u64 = 0xaa51c0;
const THREAD_POOL_RVA: u64 = 0x3056580;
const THREAD_INDEX_RVA: u64 = 0x3056cc0;
const CHUNK_INDEX_RVA: u64 = 0x3056560;
const TYPE_INDEX_RVA: u64 = 0xa4cc0;
const DATA_INDEX_RVA: u64 = 0xa4b90;
const COMPONENT_RVA: u64 = 0x144920;
const ASSERT_RVA: u64 = 0x303d380;
/// Lambda_1's and lambda_2's parallel dispatchers, replaced in the
/// benchmark by the test's own threads.
const LAMBDA1_DISPATCH_RVA: u64 = 0xaa3e70;
const LAMBDA1_RVA: u64 = 0xaa4310;
const LAMBDA2_PARALLEL_RVA: u64 = 0xaa33a0;
const LAMBDA2_RVA: u64 = 0xaa4ab0;

/// One test at a time: the game's code and the stubs share globals.
static SERIAL: Mutex<()> = Mutex::new(());

extern "system" fn failed_assert() {
    eprintln!("the game's code asserted");
    std::process::abort();
}

thread_local! {
    static THREAD: Cell<i32> = const { Cell::new(0) };
    static CHUNK: Cell<i32> = const { Cell::new(0) };
}

static COMPONENTS: Mutex<Vec<usize>> = Mutex::new(Vec::new());
static POOL_THREADS: Mutex<usize> = Mutex::new(1);

extern "system" fn thread_index() -> i32 {
    THREAD.with(Cell::get)
}

extern "system" fn chunk_index() -> i32 {
    CHUNK.with(Cell::get)
}

extern "system" fn type_index(_manager: usize, _type: usize) -> i32 {
    5
}

extern "system" fn data_index(_engine: usize, entity: i32, _type: i32) -> i32 {
    entity
}

extern "system" fn component(_manager: usize, _type: i32, index: i32) -> usize {
    COMPONENTS.lock().unwrap()[index as usize]
}

/// The game's thread pool as `Update2` reads it: `[pool+0xc8]` to
/// `[pool+0xd0]`, one 16-byte record per thread.
extern "system" fn thread_pool() -> usize {
    static POOL: Mutex<Vec<u64>> = Mutex::new(Vec::new());
    let threads = *POOL_THREADS.lock().unwrap();
    let mut pool = POOL.lock().unwrap();
    if pool.is_empty() {
        pool.resize(32, 0);
    }
    pool[0xc8 / 8] = 0x1000;
    pool[0xd0 / 8] = 0x1000 + 16 * threads as u64;
    pool.as_ptr() as usize
}

/// The game's code, mapped, with the test's stand-ins.
struct Game {
    image: Mapped,
}

impl Game {
    fn load(exe: &Exe) -> Self {
        let image = Mapped::new(exe);
        for (rva, to) in [
            (THREAD_POOL_RVA, thread_pool as *const () as usize),
            (THREAD_INDEX_RVA, thread_index as *const () as usize),
            (CHUNK_INDEX_RVA, chunk_index as *const () as usize),
            (TYPE_INDEX_RVA, type_index as *const () as usize),
            (DATA_INDEX_RVA, data_index as *const () as usize),
            (COMPONENT_RVA, component as *const () as usize),
            (ASSERT_RVA, failed_assert as *const () as usize),
        ] {
            image.jump(rva, to);
        }
        Self { image }
    }

    fn update2(&self) -> unsafe extern "system" fn(usize, usize, i32, f32) {
        // SAFETY: Update2's entry in the mapping, with its signature.
        unsafe { std::mem::transmute(self.image.at(UPDATE2_RVA)) }
    }

    fn logf(&self) -> Logf {
        // The import slot the game's `logf` thunk jumps through.
        // SAFETY: a resolved import of the C runtime.
        unsafe { std::mem::transmute(self.image.pointer(LOGF_SLOT_RVA)) }
    }
}

/// A random number source the tests can repeat.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    fn unit(&mut self) -> f32 {
        (self.next() % 1_000_001) as f32 / 1_000_000.0
    }

    fn pick<T: Copy>(&mut self, of: &[T]) -> T {
        of[self.below(of.len() as u64) as usize]
    }
}

/// An `EmissionGrid` component (the fields `Update2` and the inserts
/// read), its concentration grid's buffer, and the world's geometry.
struct GridComp {
    bytes: Vec<u64>,
    conc: Vec<f32>,
}

impl GridComp {
    fn new(origin: [i32; 2], size: [i32; 2], gp: [f32; 2], conc: Vec<f32>) -> Self {
        let mut c = Self {
            bytes: vec![0u64; 0x80 / 8],
            conc,
        };
        c.write(origin, size, gp);
        c
    }

    fn write(&mut self, origin: [i32; 2], size: [i32; 2], gp: [f32; 2]) {
        let b = self.bytes.as_mut_ptr().cast::<u8>();
        let begin = self.conc.as_mut_ptr() as u64;
        let end = begin + 4 * self.conc.len() as u64;
        // SAFETY: inside the component's 0x80 bytes.
        unsafe {
            let put32 = |at: usize, v: i32| b.add(at).cast::<i32>().write_unaligned(v);
            let putf = |at: usize, v: f32| b.add(at).cast::<f32>().write_unaligned(v);
            let put64 = |at: usize, v: u64| b.add(at).cast::<u64>().write_unaligned(v);
            put32(0, origin[0]);
            put32(4, origin[1]);
            put32(8, size[0]);
            put32(0xc, size[1]);
            putf(0x10, gp[0]);
            putf(0x14, gp[1]);
            put32(0x18, origin[0] - 1);
            put32(0x1c, origin[1] - 1);
            put32(0x20, size[0] + 2);
            put32(0x24, size[1] + 2);
            put64(0x28, begin);
            put64(0x30, end);
            put64(0x38, end);
        }
    }

    fn addr(&self) -> usize {
        self.bytes.as_ptr() as usize
    }
}

/// A random world: the grids, the emitters, the system's parameters.
#[derive(Clone)]
struct World {
    origin: [i32; 2],
    size: [i32; 2],
    gp: [f32; 2],
    /// Diffuse's spread, which `0x140aa8ba0` takes the log of.
    spread: f32,
    dt: f32,
    grids: [Vec<f32>; 2],
    /// (entity, component index) per node.
    nodes: Vec<[i32; 2]>,
    data: Vec<Emitter>,
}

impl World {
    fn random(rng: &mut Rng, size: [i32; 2], emitters: usize, specials: bool) -> Self {
        let gp = rng.pick(&[[16.0f32, 16.0], [16.0, 16.0], [8.0, 8.0], [32.0, 12.5]]);
        let origin = [
            -(size[0] / 32) * 16 + rng.pick(&[0, 0, 3, -7]),
            -(size[1] / 32) * 16 + rng.pick(&[0, 0, 5, -2]),
        ];
        let cells = ((size[0] + 2) * (size[1] + 2)) as usize;
        let grid = |rng: &mut Rng| {
            (0..cells)
                .map(|_| match rng.below(100) {
                    0..=59 => 0.0,
                    60..=61 if specials => -0.0,
                    62..=63 if specials => -rng.unit(),
                    64 if specials => f32::from_bits(1 + rng.below(0x7f_ffff) as u32),
                    _ => rng.unit() * 10f32.powi(rng.below(8) as i32 - 4),
                })
                .collect::<Vec<f32>>()
        };
        let grids = [grid(rng), grid(rng)];
        let extent = |axis: usize, rng: &mut Rng| {
            let lo = (origin[axis] - 3) as f32 * gp[axis];
            let span = (size[axis] + 6) as f32 * gp[axis];
            let x = lo + rng.unit() * span;
            match rng.below(20) {
                // On a cell edge or centre exactly.
                0 => (x / gp[axis]).round() * gp[axis],
                1 => ((x / gp[axis]).round() + 0.5) * gp[axis],
                _ => x,
            }
        };
        let mut data = Vec::new();
        for _ in 0..emitters + rng.below(8) as usize {
            let radius = |rng: &mut Rng, g: f32| match rng.below(20) {
                0..=7 => 0.0,
                8 => -rng.unit() * g,
                9 => (rng.below(6) as f32) * g,
                10 if specials => f32::NAN,
                11 => rng.unit() * g * 12.0,
                _ => rng.unit() * g * 4.0,
            };
            let power = |rng: &mut Rng| match rng.below(30) {
                0 => 0.0,
                1 => -rng.unit() * 50.0,
                2 if specials => f32::NAN,
                3 if specials => f32::INFINITY,
                4 => 1e-8,
                _ => rng.unit() * 10f32.powi(rng.below(7) as i32 - 2),
            };
            let x = extent(0, rng);
            let y = extent(1, rng);
            let dist = match rng.below(10) {
                0 => -rng.unit(),
                1 => 0.0,
                2 => rng.unit() * 5000.0,
                _ => rng.unit() * 300.0,
            };
            let g = gp[0].max(gp[1]);
            data.push([
                x,
                y,
                dist,
                radius(rng, g),
                radius(rng, g),
                power(rng),
                rng.unit(),
                power(rng),
                rng.unit(),
            ]);
        }
        // The nodes name the components in a shuffled order.
        let mut order: Vec<i32> = (0..data.len() as i32).collect();
        for i in (1..order.len()).rev() {
            let j = rng.below(i as u64 + 1) as usize;
            order.swap(i, j);
        }
        let nodes = order
            .iter()
            .take(emitters)
            .enumerate()
            .map(|(i, &c)| [i as i32 + 100, c])
            .collect();
        Self {
            origin,
            size,
            gp,
            spread: rng.pick(&[0.05f32, 0.1, 0.2, 0.24, 0.01]),
            dt: rng.pick(&[0.2f32, 0.2, 0.4, 0.19]),
            grids,
            nodes,
            data,
        }
    }
}

/// The objects `Update2` reads, built for a world, kept alive while it
/// runs.
struct Objects {
    comps: [GridComp; 2],
    entities: Box<[i32; 2]>,
    grid_system: Vec<u64>,
    nodes_vec: Box<[u64; 3]>,
    data_vec: Box<[u64; 3]>,
    buckets: Vec<u64>,
    system: Vec<u64>,
    engine: Vec<u64>,
}

impl Objects {
    fn new(world: &mut World) -> Box<Self> {
        let comps = [0, 1].map(|k| {
            GridComp::new(
                world.origin,
                world.size,
                world.gp,
                std::mem::take(&mut world.grids[k]),
            )
        });
        let mut o = Box::new(Self {
            comps,
            entities: Box::new([0, 1]),
            grid_system: vec![0; 16],
            nodes_vec: Box::new([0; 3]),
            data_vec: Box::new([0; 3]),
            buckets: vec![0; 16 * 0x50 / 8],
            system: vec![0; 8],
            engine: vec![0; 64],
        });
        o.grid_system[1] = o.entities.as_ptr() as u64;
        o.grid_system[0x50 / 8] = u64::from(world.spread.to_bits());
        let nb = world.nodes.as_ptr() as u64;
        *o.nodes_vec = [
            nb,
            nb + 8 * world.nodes.len() as u64,
            nb + 8 * world.nodes.len() as u64,
        ];
        let db = world.data.as_ptr() as u64;
        let de = db + 36 * world.data.len() as u64;
        *o.data_vec = [db, de, de];
        o.system[1] = o.nodes_vec.as_ptr() as u64;
        o.system[2] = o.data_vec.as_ptr() as u64;
        o.system[3] = o.grid_system.as_ptr() as u64;
        o.system[4] = o.buckets.as_ptr() as u64;
        o
    }

    fn take_grids(self) -> [Vec<f32>; 2] {
        let [a, b] = self.comps;
        [a.conc, b.conc]
    }
}

/// Runs the game's `Update2` on `world` (its grids in place).
fn games_update(game: &Game, world: &mut World, threads: usize) {
    let mut o = Objects::new(world);
    *COMPONENTS.lock().unwrap() = vec![o.comps[0].addr(), o.comps[1].addr()];
    *POOL_THREADS.lock().unwrap() = threads;
    // SAFETY: the game's function on the objects it reads, built above.
    unsafe {
        (game.update2())(
            o.system.as_mut_ptr() as usize,
            o.engine.as_mut_ptr() as usize,
            world.nodes.len() as i32,
            world.dt,
        );
    }
    // The bucket vectors the game allocated stay with the test (leaked).
    std::mem::forget(std::mem::take(&mut o.buckets));
    world.grids = o.take_grids();
}

/// The factor `0x140aa8ba0` computes: `((gx + gy) * 0.5) / logf(spread)`.
fn factor(logf: Logf, world: &World) -> f32 {
    let [gx, gy] = world.gp;
    // SAFETY: the C runtime's logf.
    ((gy + gx) * 0.5) / unsafe { logf(world.spread) }
}

/// The frame for `world`'s grids, whole.
fn frame_for(world: &mut World, logf: Logf) -> Frame {
    let f = factor(logf, world);
    let w = world.size[0] + 2;
    let h = world.size[1] + 2;
    let (gx0, gy0) = (world.origin[0] - 1, world.origin[1] - 1);
    let [g0, g1] = &mut world.grids;
    let views = [g0.as_mut_ptr(), g1.as_mut_ptr()].map(|data| View {
        gx0,
        gy0,
        w,
        h,
        data,
        y_lo: gy0,
        y_hi: gy0 + h,
    });
    Frame {
        origin: world.origin,
        quarter: [world.size[0] / 4, world.size[1] / 4],
        dt: world.dt,
        kinds: views.map(|view| Kind {
            gp: world.gp,
            factor: f,
            view,
        }),
        logf,
    }
}

/// The hook's splat on `world` (its grids in place).
fn hooks_update(
    logf: Logf,
    world: &mut World,
    bands: usize,
    pool: Option<&emission::pool::Pool>,
    plan: &mut Plan,
) -> Done {
    let frame = frame_for(world, logf);
    let lens = [world.grids[0].len(), world.grids[1].len()];
    let source = Source {
        nodes: &world.nodes,
        data: &world.data,
    };
    // SAFETY: the world's own grids, used by nothing else.
    unsafe { run(&frame, &source, lens, bands, pool, plan) }
        .unwrap()
        .unwrap()
}

fn bits(v: &[f32]) -> Vec<u32> {
    v.iter().map(|f| f.to_bits()).collect()
}

fn assert_same(theirs: &World, ours: &World, what: &str) {
    for k in 0..2 {
        if bits(&theirs.grids[k]) != bits(&ours.grids[k]) {
            let w = (theirs.size[0] + 2) as usize;
            let i = (0..theirs.grids[k].len())
                .find(|&i| theirs.grids[k][i].to_bits() != ours.grids[k][i].to_bits())
                .unwrap();
            panic!(
                "{what}: grid {k} differs at ({}, {}): the game {:e} ({:#x}), the hook {:e} ({:#x})",
                i % w,
                i / w,
                theirs.grids[k][i],
                theirs.grids[k][i].to_bits(),
                ours.grids[k][i],
                ours.grids[k][i].to_bits()
            );
        }
    }
}

/// Pools of 1 to 4 threads, kept for the whole test run.
fn pool_of(threads: usize) -> Option<&'static emission::pool::Pool> {
    static POOLS: [std::sync::OnceLock<emission::pool::Pool>; 5] =
        [const { std::sync::OnceLock::new() }; 5];
    (threads > 1).then(|| POOLS[threads].get_or_init(|| emission::pool::Pool::new(threads - 1)))
}

#[test]
fn the_splat_is_the_games_update2_bit_for_bit() {
    let Some(exe) = Exe::load() else { return };
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let game = Game::load(&exe);
    let logf = game.logf();
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let sizes = [
        [4, 4],
        [5, 7],
        [8, 9],
        [16, 16],
        [17, 33],
        [32, 64],
        [48, 31],
        [64, 64],
        [80, 400],
        [130, 66],
    ];
    // (bands, threads, scalar only)
    let configs = [
        (1usize, 1usize, false),
        (3, 1, true),
        (17, 4, false),
        (64, 3, false),
        (400, 2, true),
    ];
    let (mut changed, mut kept) = (0usize, 0usize);
    for case in 0..200 {
        let size = sizes[case % sizes.len()];
        let emitters = [0usize, 1, 3, 40, 300, 1500, 4000][case % 7];
        let mut world = World::random(&mut rng, size, emitters, case % 3 == 0);
        let mut plans: Vec<Plan> = configs
            .iter()
            .map(|&(_, _, scalar)| {
                let mut plan = Plan::default();
                plan.scalar = scalar;
                plan
            })
            .collect();
        for update in 0..4 {
            let before = world.clone();
            let mut theirs = world.clone();
            games_update(&game, &mut theirs, 1);
            if bits(&theirs.grids[0]) != bits(&before.grids[0])
                || bits(&theirs.grids[1]) != bits(&before.grids[1])
            {
                changed += 1;
            }
            for (&(bands, threads, _), plan) in configs.iter().zip(&mut plans) {
                let mut ours = world.clone();
                let done = hooks_update(logf, &mut ours, bands, pool_of(threads), plan);
                kept += done.chunks - done.chunks_made;
                assert_same(
                    &theirs,
                    &ours,
                    &format!("case {case} update {update} ({bands} bands, {threads} threads)"),
                );
            }
            world = theirs;
            // Some emitters change between updates; most stay as they were.
            let moved = rng.below(4);
            for e in &mut world.data {
                if rng.below(40) < moved {
                    e[0] += rng.unit() - 0.5;
                    e[2] += rng.unit();
                    e[5] *= 1.0 + rng.unit();
                }
            }
            if update == 2 {
                world.dt = 0.4;
            }
        }
    }
    assert!(changed > 400, "only {changed} updates changed a grid");
    assert!(kept > 100, "only {kept} chunks were kept between updates");
}

/// A window of rows, run on copies, gives the rows of the whole run.
#[test]
fn a_window_of_rows_is_the_whole_runs_rows() {
    let Some(exe) = Exe::load() else { return };
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let logf = Game::load(&exe).logf();
    let mut rng = Rng(5);
    for case in 0..60 {
        let size = [[64, 64], [80, 400], [33, 17]][case % 3];
        let mut world = World::random(&mut rng, size, 600, case % 2 == 0);
        let mut whole = world.clone();
        hooks_update(logf, &mut whole, 7, None, &mut Plan::default());
        let w = (world.size[0] + 2) as usize;
        let h = world.size[1] + 2;
        let lo = rng.below(h as u64 - 1) as i32;
        let hi = (lo + 1 + rng.below(20) as i32).min(h);
        let mut copies = world
            .grids
            .clone()
            .map(|g| g[lo as usize * w..hi as usize * w].to_vec());
        let mut frame = frame_for(&mut world, logf);
        for (k, copy) in copies.iter_mut().enumerate() {
            let v = &mut frame.kinds[k].view;
            v.data = copy.as_mut_ptr();
            v.y_lo = v.gy0 + lo;
            v.y_hi = v.gy0 + hi;
        }
        let source = Source {
            nodes: &world.nodes,
            data: &world.data,
        };
        let lens = [copies[0].len(), copies[1].len()];
        // SAFETY: the copies hold the window's rows.
        unsafe { run(&frame, &source, lens, 3, None, &mut Plan::default()) }
            .unwrap()
            .unwrap();
        for (k, copy) in copies.iter().enumerate() {
            assert_eq!(
                bits(copy),
                bits(&whole.grids[k][lo as usize * w..hi as usize * w]),
                "case {case}, grid {k}, rows {lo}..{hi}"
            );
        }
    }
}

/// The benchmark's pool: lambda_1's chunks and lambda_2's 32 tasks run
/// on it, as the game's pool would run them.
static BENCH_POOL: std::sync::OnceLock<emission::pool::Pool> = std::sync::OnceLock::new();
static MAPPED_AT: Mutex<usize> = Mutex::new(0);

fn bench_pool() -> &'static emission::pool::Pool {
    BENCH_POOL.get_or_init(|| {
        let threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(16);
        emission::pool::Pool::new(threads - 1)
    })
}

/// `LoopImpl` for lambda_1 (`0x140aa3e70`): chunks of at least 1,024
/// emitters on the pool, each thread pushing into its own vectors.
extern "system" fn lambda1_dispatch(
    _pool: usize,
    ctx: usize,
    count: i32,
    min_chunk: i32,
    _a5: usize,
    _a6: usize,
    _a7: usize,
) {
    let base = *MAPPED_AT.lock().unwrap();
    // SAFETY: lambda_1's body in the mapping.
    let body: unsafe extern "system" fn(usize, i32, i32) =
        unsafe { std::mem::transmute(base + LAMBDA1_RVA as usize) };
    if *POOL_THREADS.lock().unwrap() == 1 {
        // The inline path: one call over every emitter, on this thread.
        THREAD.with(|t| t.set(0));
        CHUNK.with(|t| t.set(0));
        // SAFETY: the game's lambda on its own captures.
        unsafe { body(ctx, 0, count) };
        return;
    }
    let pool = bench_pool();
    // The game's pool as `Update2` saw it: its per-thread vectors.
    let threads = POOL_THREADS.lock().unwrap().min(pool.participants());
    let chunk = (count as usize)
        .div_ceil(threads * 4)
        .max(min_chunk as usize);
    let chunks = (count as usize).div_ceil(chunk);
    let next = std::sync::atomic::AtomicUsize::new(0);
    pool.run(&|thread| {
        if thread >= threads {
            return;
        }
        THREAD.with(|t| t.set(thread as i32));
        loop {
            let c = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if c >= chunks {
                break;
            }
            CHUNK.with(|t| t.set(c as i32));
            let end = ((c + 1) * chunk).min(count as usize);
            // SAFETY: the game's lambda on its own captures.
            unsafe { body(ctx, (c * chunk) as i32, end as i32) };
        }
    });
}

/// `LoopImpl` for lambda_2 (`0x140aa33a0`): its 32 tasks on the pool.
extern "system" fn lambda2_dispatch(
    _pool: usize,
    ctx: usize,
    tasks: i32,
    _min: i32,
    _futures: usize,
    _a6: usize,
) {
    let base = *MAPPED_AT.lock().unwrap();
    // SAFETY: lambda_2's loop in the mapping.
    let body: unsafe extern "system" fn(usize, i32, i32) =
        unsafe { std::mem::transmute(base + LAMBDA2_RVA as usize) };
    let next = std::sync::atomic::AtomicUsize::new(0);
    let threads = *POOL_THREADS.lock().unwrap();
    bench_pool().run(&|thread| {
        if thread >= threads {
            return;
        }
        THREAD.with(|t| t.set(thread as i32));
        loop {
            let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if i >= tasks as usize {
                break;
            }
            // SAFETY: the game's lambda on its own captures.
            unsafe { body(ctx, i as i32, i as i32 + 1) };
        }
    });
}

/// A big world: `towns` clusters of emitters on a `tiles`-tile map.
fn big_world(rng: &mut Rng, tiles: [i32; 2], emitters: usize, towns: usize, wide: f32) -> World {
    let size = [tiles[0] * 16, tiles[1] * 16];
    let origin = [-(tiles[0] / 2) * 16, -(tiles[1] / 2) * 16];
    let gp = [16.0f32, 16.0];
    let cells = ((size[0] + 2) * (size[1] + 2)) as usize;
    let grids = [vec![0.0f32; cells], vec![0.0f32; cells]];
    let centres: Vec<[f32; 2]> = (0..towns)
        .map(|_| {
            [
                (origin[0] as f32 + rng.unit() * size[0] as f32) * gp[0],
                (origin[1] as f32 + rng.unit() * size[1] as f32) * gp[1],
            ]
        })
        .collect();
    let mut data = Vec::new();
    for _ in 0..emitters {
        let c = rng.pick(&centres);
        let spread = |rng: &mut Rng| (rng.unit() + rng.unit() + rng.unit() - 1.5) * 1200.0;
        let (x, y) = (c[0] + spread(rng), c[1] + spread(rng));
        let roll = rng.below(100);
        let (rn, rp) = match roll {
            0..=54 => (0.0, 0.0),
            55..=89 => (rng.unit() * wide, rng.unit() * wide),
            _ => (
                40.0 + rng.unit() * 4.0 * wide,
                60.0 + rng.unit() * 6.0 * wide,
            ),
        };
        data.push([
            x,
            y,
            rng.unit() * 200.0,
            rn,
            rp,
            0.5 + rng.unit() * 20.0,
            0.0,
            if roll.is_multiple_of(3) {
                0.0
            } else {
                rng.unit() * 5.0
            },
            0.0,
        ]);
    }
    let nodes = (0..data.len() as i32).map(|i| [i, i]).collect();
    World {
        origin,
        size,
        gp,
        spread: 0.2,
        dt: 0.2,
        grids,
        nodes,
        data,
    }
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn QueryThreadCycleTime(thread: isize, cycles: *mut u64) -> i32;
    fn GetCurrentThread() -> isize;
}

/// The CPU cycles this thread has run (not counting time it waited).
fn thread_cycles() -> u64 {
    let mut c = 0;
    // SAFETY: the current thread's pseudo handle and a u64 to fill.
    unsafe { QueryThreadCycleTime(GetCurrentThread(), &mut c) };
    c
}

/// The splat's speed-up on a 78 x 390-tile world: the game's `Update2`,
/// its lambdas on a pool of this PC's threads (up to 16) as the game's
/// pool runs them, against the band splat on the same threads; then both
/// on one thread, in that thread's cycles. Run with
/// `cargo test -p tpf3mp-hook --release -- --ignored emitters_speed --nocapture`.
#[test]
#[ignore = "a benchmark: a few seconds"]
fn emitters_speed() {
    let Some(exe) = Exe::load() else { return };
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let game = Game::load(&exe);
    *MAPPED_AT.lock().unwrap() = game.image.at(0);
    with_pool_stand_ins(&game);
    let logf = game.logf();
    let pool = bench_pool();
    let threads = pool.participants();
    let mut rng = Rng(77);
    let bands: usize = std::env::var("TPF3MP_BENCH_BANDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(BANDS_PER_THREAD * threads);
    for (emitters, towns, wide) in [
        (20_000usize, 60usize, 24.0f32),
        (100_000, 150, 24.0),
        (100_000, 150, 64.0),
        (300_000, 300, 16.0),
    ] {
        let world = big_world(&mut rng, [78, 390], emitters, towns, wide);
        let mut theirs = world.clone();
        let mut ours = world.clone();
        let mut times: [Vec<f64>; 4] = Default::default();
        let mut plan = Plan::default();
        let mut cold = Plan::default();
        hooks_update(logf, &mut world.clone(), bands, None, &mut cold);
        for update in 0..9 {
            // From the third update on, 5% of the emitters change.
            if update >= 2 {
                for i in 0..theirs.data.len() {
                    if rng.below(20) == 0 {
                        theirs.data[i][5] *= 1.01;
                        ours.data[i][5] = theirs.data[i][5];
                    }
                }
            }
            let start = Instant::now();
            games_update(&game, &mut theirs, threads);
            times[0].push(start.elapsed().as_secs_f64() * 1e3);
            let start = Instant::now();
            let done = hooks_update(logf, &mut ours, bands, Some(pool), &mut plan);
            times[1].push(start.elapsed().as_secs_f64() * 1e3);
            if std::env::var_os("TPF3MP_BENCH_PHASES").is_some() {
                eprintln!("  {done:?}");
            }
        }
        let mut kept_cycles = Vec::new();
        for _ in 0..3 {
            let start = thread_cycles();
            games_update(&game, &mut theirs, 1);
            times[2].push((thread_cycles() - start) as f64 / 1e6);
            // Every emitter changed (the plan's buffers warm).
            cold.forget();
            let start = thread_cycles();
            hooks_update(logf, &mut ours, bands, None, &mut cold);
            times[3].push((thread_cycles() - start) as f64 / 1e6);
        }
        for _ in 0..3 {
            games_update(&game, &mut theirs, 1);
            // No emitter changed.
            let start = thread_cycles();
            hooks_update(logf, &mut ours, bands, None, &mut cold);
            kept_cycles.push((thread_cycles() - start) as f64 / 1e6);
        }
        drop(cold);
        assert_same(&theirs, &ours, &format!("{emitters} emitters"));
        let best = |v: &[f64]| v.iter().copied().fold(f64::MAX, f64::min);
        let [g, h, g1, h1] = times.each_ref().map(|t| best(t));
        let k1 = best(&kept_cycles);
        eprintln!(
            "{emitters} emitters in {towns} towns (radius up to {wide} m), 1250x6242 cells: {threads} threads, 5% of the emitters changing each update: the game's Update2 {g:.2} ms, the band splat {h:.2} ms ({bands} bands): {:.2}x; one thread: the game's {g1:.0} Mcycles, the band splat {h1:.0} Mcycles with every emitter changed ({:.2}x), {k1:.0} Mcycles with none ({:.2}x)",
            g / h,
            g1 / h1,
            g1 / k1,
        );
    }
}

#[test]
#[ignore = "a profile of the splat's parts"]
fn emitters_parts() {
    let Some(exe) = Exe::load() else { return };
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let game = Game::load(&exe);
    let logf = game.logf();
    let mut rng = Rng(77);
    for (emitters, wide) in [(100_000usize, 24.0f32), (100_000, 64.0), (300_000, 16.0)] {
        let mut world = big_world(&mut rng, [78, 390], emitters, 150, wide);
        let frame = frame_for(&mut world, logf);
        let n = world.data.len() as f64;
        let source = Source {
            nodes: &world.nodes,
            data: &world.data,
        };
        for (bands, cached) in [(64usize, false), (256, false), (1024, false), (256, true)] {
            // SAFETY: the world's grids; logf; SSE4.1 on this PC.
            let (a, b) = unsafe { bands::bench_phases(&frame, &source, bands, 5, cached) };
            eprintln!(
                "{emitters} emitters, radius up to {wide} m, {bands} bands{}: phase A {:.0} cycles, phase B {:.0} cycles per emitter",
                if cached { ", kept" } else { "" },
                a as f64 / n,
                b as f64 / n
            );
        }
    }
}

/// Redirects the mapped `Update2`'s three calls to the hook.
fn hook(game: &Game) -> Vec<tpf3mp_hookcore::detour::CallRedirect> {
    let update2 = game.image.at(UPDATE2_RVA);
    let at = |offset: i64| update2.wrapping_add_signed(offset as isize);
    // SAFETY: the mapping's own `Update2`, which nothing runs meanwhile.
    unsafe {
        install_at(
            super::Game::from_update2(update2),
            [
                at(LAMBDA1_CALL),
                at(LAMBDA2_LOOP_CALL),
                at(LAMBDA2_POOL_CALL),
            ],
        )
    }
    .unwrap()
}

/// Runs `updates` updates of `world` the game's way (`stock`), and the
/// same with the hook's redirects (`hooked`, the redirects installed), on
/// `threads` pool threads; both end the same, bit for bit. Emitters change
/// between updates.
fn compare_updates(
    stock_game: &Game,
    hooked_game: &Game,
    seed: u64,
    size: [i32; 2],
    emitters: usize,
    threads: usize,
    updates: usize,
) {
    let mut rng = Rng(seed);
    let world = World::random(&mut rng, size, emitters, seed.is_multiple_of(2));
    let changes: Vec<Vec<usize>> = (0..updates)
        .map(|_| {
            (0..world.data.len())
                .filter(|_| rng.below(10) == 0)
                .collect()
        })
        .collect();
    let mut stock = world.clone();
    let mut hooked = world;
    let step = |w: &mut World, u: usize| {
        for &i in &changes[u] {
            w.data[i][0] += 3.0;
            w.data[i][5] *= 1.5;
        }
    };
    for u in 0..updates {
        *MAPPED_AT.lock().unwrap() = stock_game.image.at(0);
        games_update(stock_game, &mut stock, threads);
        step(&mut stock, u);
        *MAPPED_AT.lock().unwrap() = hooked_game.image.at(0);
        games_update(hooked_game, &mut hooked, threads);
        step(&mut hooked, u);
    }
    assert_same(
        &stock,
        &hooked,
        &format!("seed {seed}, {size:?}, {emitters} emitters, {threads} threads"),
    );
}

/// The game's code twice: as it is, and with the hook's redirects;
/// mapped once for every test (installing is slow next to a mapping).
struct Pair {
    stock: Game,
    hooked: Game,
    _redirects: Vec<tpf3mp_hookcore::detour::CallRedirect>,
}

// SAFETY: the tests that use it hold SERIAL.
unsafe impl Send for Pair {}
// SAFETY: as above.
unsafe impl Sync for Pair {}

fn stock_and_hooked(exe: &Exe) -> &'static Pair {
    static PAIR: std::sync::OnceLock<Pair> = std::sync::OnceLock::new();
    PAIR.get_or_init(|| {
        let stock = Game::load(exe);
        let hooked = Game::load(exe);
        with_pool_stand_ins(&stock);
        with_pool_stand_ins(&hooked);
        let _redirects = hook(&hooked);
        Pair {
            stock,
            hooked,
            _redirects,
        }
    })
}

/// The parallel dispatchers stood in for, so the mapped `Update2` can run
/// with more than one pool thread.
fn with_pool_stand_ins(game: &Game) {
    game.image
        .jump(LAMBDA1_DISPATCH_RVA, lambda1_dispatch as *const () as usize);
    game.image
        .jump(LAMBDA2_PARALLEL_RVA, lambda2_dispatch as *const () as usize);
}

#[test]
fn the_hooked_update2_leaves_the_grids_as_the_games() {
    let Some(exe) = Exe::load() else { return };
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let Pair { stock, hooked, .. } = stock_and_hooked(&exe);
    reset();
    for (seed, size, emitters, threads) in [
        (1u64, [64, 64], 500usize, 1usize),
        (2, [80, 400], 3000, 1),
        (3, [33, 17], 40, 1),
        (4, [80, 400], 3000, 4),
        (5, [130, 66], 2000, 4),
        (6, [4, 4], 3, 4),
    ] {
        compare_updates(stock, hooked, seed, size, emitters, threads, 8);
    }
    assert!(!BROKEN.load(Ordering::SeqCst));
    assert_eq!(CHECKED.load(Ordering::SeqCst), FIRST_CHECKS);
    assert!(BANDED.load(Ordering::SeqCst) >= 6 * 8 - FIRST_CHECKS - 6);
    assert_eq!(GAMES_WAY.load(Ordering::SeqCst), 0);
}

#[test]
fn a_thread_without_the_default_mxcsr_runs_the_games_way() {
    let Some(exe) = Exe::load() else { return };
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let Pair { stock, hooked, .. } = stock_and_hooked(&exe);
    reset();
    UPDATES.store(FIRST_CHECKS + 1, Ordering::SeqCst);
    let saved = emission::fused::mxcsr();
    let set = |value: u32| {
        // SAFETY: loads MXCSR from a local.
        unsafe { std::arch::asm!("ldmxcsr [{}]", in(reg) &value, options(nostack)) };
    };
    set((saved & emission::fused::MXCSR_CONTROL) | 0x8040);
    compare_updates(stock, hooked, 9, [80, 400], 2000, 1, 3);
    set(saved);
    assert_eq!(GAMES_WAY.load(Ordering::SeqCst), 3);
    assert_eq!(BANDED.load(Ordering::SeqCst), 0);
}

#[test]
fn a_failed_self_check_leaves_the_games_result_and_turns_it_off() {
    let Some(exe) = Exe::load() else { return };
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let Pair { stock, hooked, .. } = stock_and_hooked(&exe);
    reset();
    SABOTAGE.store(true, Ordering::SeqCst);
    compare_updates(stock, hooked, 11, [64, 64], 800, 1, 5);
    SABOTAGE.store(false, Ordering::SeqCst);
    assert!(BROKEN.load(Ordering::SeqCst));
    assert_eq!(BANDED.load(Ordering::SeqCst), 0);
    reset();
}

#[test]
fn the_recorded_code_is_the_executables() {
    let Some(exe) = Exe::load() else { return };
    let update2 = UPDATE2_RVA as usize;
    check_code(update2, &|at, len| {
        exe.try_bytes(at as u64, len).map(<[u8]>::to_vec)
    })
    .unwrap();
    let callee = |site: usize| {
        let code = exe.bytes(site as u64, 5);
        assert_eq!(code[0], 0xE8, "a call at {site:#x}");
        let rel = i32::from_le_bytes(code[1..5].try_into().unwrap());
        site.wrapping_add_signed(5 + rel as isize)
    };
    let game = super::Game::from_update2(update2);
    let at = |offset: i64| update2.wrapping_add_signed(offset as isize);
    assert_eq!(callee(at(LAMBDA1_CALL)), game.lambda1_dispatch);
    assert_eq!(callee(at(LAMBDA2_LOOP_CALL)), game.lambda2_loop);
    assert_eq!(callee(at(LAMBDA2_POOL_CALL)), game.lambda2_pool);
    assert_eq!(
        [game.lambda1_dispatch, game.lambda2_loop, game.lambda2_pool].map(|a| a as u64),
        [LAMBDA1_DISPATCH_RVA, LAMBDA2_RVA, LAMBDA2_PARALLEL_RVA]
    );
    // Lambda_2's body calls the logf thunk, which jumps through the slot.
    assert_eq!(callee(0xaa47f0), at(LOGF_THUNK));
    let thunk = exe.bytes(at(LOGF_THUNK) as u64, 6);
    assert_eq!(&thunk[..2], &[0xFF, 0x25]);
    let disp = i32::from_le_bytes(thunk[2..6].try_into().unwrap());
    assert_eq!(
        at(LOGF_THUNK).wrapping_add_signed(6 + disp as isize),
        game.logf_slot
    );
    assert_eq!(game.logf_slot as u64, LOGF_SLOT_RVA);
}
