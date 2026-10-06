# TF3 far-from-origin vehicle jitter (build 40408), 2026-10-05

Static research only. Binary: `TransportFever3.exe` SHA-256 `de1daad3…f23ef2` (same as the
`~/tf3-builds/de1daad3…/tf3.tpfdb` index), image base 0x140000000; all addresses below are RVAs.
Shader sources are loose GLSL in `base/content/rendering/programs/shaders/`.
Symptom: on a 100 × 1000 tile map (±128 km on x), planes at an airport near one end
"vibrate left and right a tiny amount".

Evidence labels: **CONFIRMED-static** (read in code, strings, RTTI or shipped source),
**DERIVED** (follows from confirmed facts plus float arithmetic), **GUESS**.

## Short answer

- **Cause:** float32 world coordinates everywhere. No double-precision transform type exists. There
  are three sources of jitter, one on the sim side and two on the render side. Each is about 1–3 ulp,
  and the ulp at 64–128 km is 7.8 mm. Near the origin each is 8–16 times smaller, which is why the
  centre looks fine.
  1. **Sim side (CONFIRMED-static form, DERIVED effect).** The path position is evaluated in f32 in
     a rounding order that makes the error noisy: the cubic Hermite evaluation adds three terms at
     full world magnitude. The result is stored as an f32 `Mat4f` in
     `ModelInstanceList.fatInstances[i].transf`.
  2. **Render side, CPU (CONFIRMED-static).** The renderer interpolates elementwise between `transf0`
     and `transf` in f32 world coordinates. This step is monotone, so it adds staircase steps but no
     noise.
  3. **Render side, GPU (CONFIRMED-static from GLSL, DERIVED magnitude).** The vertex shaders compute
     `instAttrModel * pos` in absolute world f32, then `projView * worldPos`. The second product
     cancels catastrophically, roughly ±1–3 cm at 128 km. Most likely this is the largest visible
     part (DERIVED).
- **Narrowest fix:**
  - (A) Render only: a camera-relative vertex-shader override, done by editing shipped GLSL with no
    exe patch. It is safe for lockstep and removes source 3 and per-vertex snapping.
  - (B) Optionally, sim side: replace the f32 Hermite evaluation `0x963090` with an f64 evaluation
    that rounds once. That turns source 1 from noise into a ≤1-ulp monotone staircase. It changes sim
    floats, so every peer must run it, which they do. It is deterministic.
  - A full rebasing origin, meaning a double or region-relative sim, is far larger and not needed.

## 1. How the sim stores vehicle transforms: f32 only

- **No double math types (CONFIRMED-static).** The RTTI type names hold `CVec2f`, `CVec3f`, `CVec4f`,
  `CVec5f`, `CVec2i` and `CMat4f` (181 hits). There is no `CVec3d`, `CMat4d` or `Transf3d`, and no
  string `Vec3d`/`Mat4d`. The Teal API (`api/tealdef/api/type.d.tl`) exposes only `Vec2f`, `Vec3f`,
  `Vec4f` and `Mat4f`. `View:getEye()` returns `Vec3f`, so the camera eye is float as well.
- **`ModelInstance` (fat instance) layout, stride 0x8c (CONFIRMED-static).** Lua registration is at
  `0x1c546e0`, at the call `0x1c57c2b` (`"ModelInstance"`).

  | off  | field                                 | type |
  |------|---------------------------------------|------|
  | 0x00 | modelId                               | i32  |
  | 0x04 | transf ("the model transform")        | Mat4f (16 × f32, column-major, translation at +0x34..+0x3c) |
  | 0x44 | transf0 ("transform at the previous frame") | Mat4f |
  | 0x84 | transf0-valid flag                    | u8 (set to 1 when transf0 is filled) |
  | 0x88 | transformator                         | i32  |

  `ThinModelInstance` is `{modelId@0, pos Vec3f@4, rot f32@0x10, scale f32@0x14}` (same function,
  call `0x1c57cac`).
- **The vehicle position is a path coordinate, not a world coordinate (CONFIRMED-static, tealdef).**
  `MovePath(Aircraft)` holds `pathPos{edgeIndex, pos, pos01}` (f32), `pathPos0`, `dyn`, `dyn0`
  ("dyn state at the beginning of the simulation frame"). Edge geometry is f32: `Straight`
  pos `{Vec2f}`, `Arc` center `Vec2f`, `CubicSpline` pos/tangent `{Vec2f}`. `PathPosData` is
  `{position Vec3f, pPrime, pPrimePrime}` (0x24 bytes).
- **Where the world transform is produced (sim step, all f32, CONFIRMED-static).**
  - `ecs::AircraftMoveSystem::Update2` is at `0xa83a40` (vtable `0x36fa428`, slot 12; assert string
    `"ecs::AircraftMoveSystem::Update2"`).
    - At `0xa84589` it copies `transf (+4)` into `transf0 (+0x44)` and sets `+0x84 = 1`.
    - It then calls `vehicle_util::SetModelTransfAndBBoxShipOrAircraft` at `0x269bad0` (asserts
      `"!resetTransf0 || !isAircraft"` and `"mil0.thinInstances.empty() && mil0.fatInstances.size() == 1"`).
  - That calls `transport::CarriageListPathTracer<pair<EdgeGeometry,bool>>` at `0x2699f20`.
    - It takes `PathPosData` and computes `pos + normalize(pPrime) * offset` in f32 (`0x269a6d0..0x269a722`).
    - It builds a frame matrix with translation through `sub_ed680`, multiplies it by
      pitch/roll matrices from `sinf`/`cosf`, and writes the result with
      `vmovups [rdi+4]` / `[rdi+0x24]` at `0x269a7a0`.
  - The path evaluation itself is `transport::Move` (`0x25c2fc0`, called from `sub_2563460`), then
    `sub_a808d0`, then **`transport::CalcPosition` at `0x2be0c60`** (assert string
    `"transport::CalcPosition"`; 271 call sites in the binary). By geometry type:
    - STRAIGHT: `p0 + (p1-p0)*t`. One large rounding, monotone.
    - ARC: `center + r*cos/sin(start + t*angle)`. One large rounding, monotone.
    - **CUBIC_SPLINE: `sub_963090`** (4 callers):
      `x = (((P1-P0)*h01 + P0) + h10*T0) + h11*T1` (`0x96324a..0x9632dc`).
      The partial sum reaches world magnitude after the first add and is rounded **three times** at
      7.8 mm granularity. The rounding error is therefore *not monotone* in t: it wanders by about
      ±1 ulp from tick to tick. Across the direction of travel that error shows up as a left-right
      wobble (DERIVED).
    - CUBIC_OFFSET_SPLINE: `0x30537d0` (hermite.cpp, 6 callers), with a similar f32 multi-term form.
- **The movement and transform code contains no f64 arithmetic (CONFIRMED-static).** A count of
  SSE/AVX double ops in `AircraftMoveSystem::*`, `LandVehicleMoveSystem::*`, `AircraftTransformator::*`,
  the tracer, `CRenderer::NewUpdate*` and `DynamicModelRenderer::*` finds only `movsd`/`vmovsd`,
  used as 8-byte copies of two floats. There is no `addsd`, `mulsd` or `cvtss2sd`.
- TF2 notes in `tpf2-bigmap` hold nothing on jitter. A previous claim that "292 km TF2 maps had no
  reported jitter" is anecdotal and is not evidence.

## 2. Render path: float world-space instance matrices, no camera-relative step

- **Interpolation between sim ticks (CONFIRMED-static).**
  - `CRenderer::NewUpdate::<lambda_6>` is at `0x2f4c20`; the string is at `0x3688870`, referenced
    at `0x2f5d4a`. It walks fat instances (`imul rdx, rax, 0x8c` at `0x2f50a3`).
  - When `+0x84` is set, it calls **`sub_2d94d0(out, &transf0, &transf, t=[ctx+0x54])`** (`0x2f50dc`).
  - `sub_2d94d0` is an elementwise f32 lerp over all 16 entries:
    `out[i] = transf0[i] + (transf[i]-transf0[i])*t` (`0x2d951e..0x2d96e5`).
  - The translation is therefore interpolated in absolute f32 world coordinates. There is one
    rounding at world magnitude, so it is monotone in t: it adds 7.8 mm stair steps but no noise
    (DERIVED).
  - The result goes into `RenderModelInstance` and then the `InstAttrBuffer` of
    `DynamicModelRenderer::Render` (`0x2c6d3b0`). No camera subtraction exists anywhere on this path:
    there is no f64 code, and no `camPos` operand is used before the upload.
- **Shaders (CONFIRMED-static, shipped GLSL).** `mat/vs/std/normal.vs`, used by `phys*.prog.lua`,
  does:
  ```glsl
  VERTEX_INPUT(INSTATTR_MDL_LOC, mat4, instAttrModel, ...);   // absolute world matrix, f32
  posAmbient  = instAttrModel * attrPosition;                  // absolute world position, f32
  gl_Position = u_view.projView * vec4(posAmbient.xyz, 1.0);   // projView includes -R*eye
  ```
  The `View` UBO (`pbr/view.h.glsl`) holds `projView`, `proj`, `view`, `viewInverse` and `camPos`,
  all f32. The same pattern appears in `std/color.vs`, `color_diffuse.vs`, `depth.vs`,
  `depth_alpha*.vs`, `cblend.vs`, `skinning/{normal,color,depth,depth_alpha}.vs`, `billboard2.vs`
  and `particle.vs`. Only `sky.vs` adds `viewInverse[3]` itself, to stay camera-relative.
  - Per-vertex: `instAttrModel*pos` rounds every vertex independently to the 7.8 mm grid, so the
    mesh shimmers by up to ±3.9 mm (DERIVED).
  - `projView*world`: terms of about 1.2e5 cancel to a small view-space value. The error is a few
    ulp of 1.2e5, about 1–3 cm, and it is not monotone in position. A moving vehicle with a still
    camera vibrates; a moving camera shakes the whole scene (DERIVED).

## 3. Interpolation between sim ticks

It runs on the render side, in **f32 absolute world coordinates** (`sub_2d94d0`, above). The sim
also keeps `pathPos0`/`dyn0`, but the renderer interpolates the finished `Mat4f`, not the path
coordinate (CONFIRMED-static). The lerp is monotone, so it is not a source of left-right noise by
itself (DERIVED).

## 4. Conclusion

**Where the jitter comes from: both sides.**
- The render side is probably dominant: the `projView*world` cancellation and the per-vertex world
  rounding give about 1–3 cm (DERIVED).
- The sim side adds about ±1 ulp of noise, about 8 mm at 64–128 km, from the f32 Hermite evaluation
  order (CONFIRMED-static form, DERIVED magnitude).
- On stock-size maps both are below 4 mm, and therefore not noticed.
- The direction fits the report: a plane near x = ±120 km moving roughly along y sees x-axis noise
  of ulp(120 km) = 7.8 mm, which is exactly "left and right" (DERIVED).

**A cheap in-game test (no code needed), to tell the sources apart:**
- Pause the game near the far airport and orbit or zoom the camera. If the planes and buildings
  shake, the render projView cancellation is confirmed.
- Unpause with the camera still. If the planes still wobble, that is the per-vertex rounding plus
  the sim Hermite noise.

### Fix A: render-only, camera-relative vertex shaders (recommended first)

- **Patch sites:**
  - Override the model vertex shaders in the mod's res tree:
    - `shaders/mat/vs/std/{normal,color,color_diffuse,depth,depth_alpha,depth_alpha_diffuse,cblend}.vs`
    - `shaders/mat/vs/skinning/{normal,color,depth,depth_alpha}.vs`
  - All of them must change together: a depth prepass and a colour pass that disagree would z-fight.
  - Leave terrain, grass, water and billboards alone. They are static; only moving models wobble.
- **Formula:**
  ```glsl
  vec3 eye = u_view.viewInverse[3].xyz;                         // same trick sky.vs uses
  vec3 rel = (instAttrModel[3].xyz - eye) + mat3(instAttrModel) * attrPosition.xyz; // small numbers
  posAmbient = vec4(rel + eye, attrPosition.w);                 // keep world pos for FS lighting/fog
  gl_Position = u_view.proj * vec4(mat3(u_view.view) * rel, 1.0);
  ```
  - `T - eye` is exact for nearby values (Sterbenz), so every vertex keeps sub-mm precision.
  - The eye's own f32 rounding becomes one rigid shift of the whole view, which is invisible.
  - It still holds for the shadow pass, because it uses that pass's own `view`/`viewInverse`.
    (GUESS: shadow passes fill the same UBO layout. Verify by reading the shadow program defines.)
- **Lockstep:** none. The change is render only.
- **What is left:** the f32 `transf` itself, so up to ±1 ulp of sim noise plus a 7.8 mm rigid
  staircase. This is probably small enough not to see; verify in game.
- **Effort:** about half a day to a day, including checking that TF3 loads mod-overridden shader
  files. The resolver uses absolute res paths (`/rendering/programs/shaders/...`) and glslang runs at
  runtime (`lib\renderer\shadercache.cpp`), so it probably does (DERIVED).
- **Exe-only alternative:** subtract a snapped origin in `sub_2d94d0`'s output and rebase the View
  UBO. This is much more invasive: shadows, terrain, water and every pass share `projView`.

### Fix B: sim-side Hermite evaluation, optional, only if wobble remains after A

- **Patch site:** detour `sub_963090`, the CUBIC_SPLINE branch of `transport::CalcPosition`
  `0x2be0c60` at `0x2be0e11`. Evaluate in f64 and round once, or compute
  `P0 + ((P1-P0)*h01 + h10*T0 + h11*T1)`.
  - Optionally do the same for `0x30537d0` (offset spline) and the tracer's `pos + dir*offset`
    (`0x269a6e0..0x269a722`).
- **Lockstep:** this changes sim floats that feed path positions. Those are used widely, by 271
  `CalcPosition` sites, and `ShipState.dyn.actualPos` is persisted. It is deterministic if every peer
  runs the same patch: Rust f64 on x86-64 SSE2 is reproducible and has no FMA contraction unless you
  ask for it. It is required in all games in the room, and it breaks bit-parity with vanilla. Rerun
  the checkpoint and rolling determinism suites.
- **Effort:** about half a day for the hook, plus a determinism re-validation.

### Not recommended

A floating or rebasing origin for the sim, or f64 `ModelInstance.transf`. Both would touch the
component layout, the Lua API, collision and the octree. That is weeks of work and not needed for
a 7.8 mm ulp.
