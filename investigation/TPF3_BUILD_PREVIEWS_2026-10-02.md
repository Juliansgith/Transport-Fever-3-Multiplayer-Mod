# Native build previews of another player's proposal: static findings, 2 October 2026

Transport Fever 3 Steam build 40408, Windows x64 (SHA-256
`de1daad3a13f3b7e9f79903361bb43769cf4f15e59271a263aefe1f075f23ef2`).
Static only: `tools/tpfre` over `investigation/dayone-2026-09-29/TransportFever3.tpfdb`,
the game's own Lua read from `base/content/gui.zip` and
`base/tealdef/scripts/builtin.d.tl`, and the TPF2 reference
(`tpf2-multiplayer/native/src/preview_plugin.cpp`, `docs/BUILD_PREVIEWS.md`,
build 35924). The game was not run and nothing was hooked. Every RVA below
is from the indexed binary; every signature is `tpfre q sig` output and
matches once in `.text`. "Not confirmed" marks what the disassembly does not
settle.

## Headline: TF3 ships a native "ProposalViewer"

TF3 has a React builtin that does what the TPF2 plugin built by hand.
`gui/main/builtin.lua:220` declares
`builtin.ProposalViewer = react.DeclareBuiltinWithUserdata("ProposalViewer", ...)`,
and `builtin.d.tl:1450` types its parameters:

```
record ProposalViewerParam
    simpleProposal : SimpleProposal
    proposal : Proposal
    controlPointInfo : Type.ControlPointInfo
    onCreateProposalData : function(ProposalData, Proposal)
    entityForRefundableContext : Engine.Entity
    proposalId : string
end
```

The game itself uses it to preview a proposal that no tool made: the bridge
and tunnel swap window (`gui/entity_window/bridge_and_tunnel.tl:130`) mounts
`builtin.ProposalViewer{ proposal = ..., onCreateProposalData = ...,
entityForRefundableContext = api.engine.util.getPlayer(), proposalId = ... }`
as a child of a `builtin.ActionDescriptor`.

Its native step is `0x2aa3370`, reached from the `_Func_impl` of
`UI::react::InitializeActionDescriptorCache(ReactFramework&, CGame*,
CGameUI*, CRendererComponent*, ...)::<lambda_1>` (`0x29f8690`, call at
`0x29f8736`). In one function it runs the whole TPF2 pipeline:

| step | address in `0x2aa3370` | call |
|---|---|---|
| renderer made once by `InitializeActionDescriptorCache` | `0x29f6a00` | `RendererFactory::Create(*(CGameUI+0x588))` |
| register the renderer | `0x2aa3616` | `CRendererComponent::AddRenderable(rc, r)` |
| clear before refilling | `0x2aa3666` | `BuilderRenderer::Clear(r, 1, 1, 1)` |
| street toolkit from the game state | `0x2aa3467` | `StreetToolkit` ctor `0x2458f0` |
| `simpleProposal` given: convert it | `0x2aa34a2` | `scripting::Convert 0x10c4fd0` |
| preprocess | `0x2aa376a` | `PreprocessProposal 0xa3f9d0` |
| evaluate | `0x2aa37ab` | `CreateProposalData 0xa1fd10` |
| hand the result to Lua | `0x2aa38b7` | `onCreateProposalData(ProposalData, Proposal)` |
| fill the renderer | `0x2aa3b0d` | `AddToRenderer 0x5e2b20` |
| parameters gone: hide | `0x2aa3d59`, `0x2aa3d64` | `Clear(r, 1, 1, 1)`, `RemoveRenderable(rc, r)` |

So the first thing to try for remote previews is a GUI-mod
`ProposalViewer{ proposal = <rebuilt from the wire>, proposalId = "tpf3mp_<origin>" }`
with no renderer hooks at all. Its limits, none confirmed:

- **One instance.** The renderer lives in the action descriptor cache
  (`InitializeActionDescriptorCache` stores it at `[r14+0x20]`; the step
  lambda reads `CachedInstances` at `+0x17f0`/`+0x1d98`). It looks like one
  renderer per cache, so one viewer at a time, not one per peer.
- **It is part of an action.** It is mounted inside an `ActionDescriptor`,
  which looks like the current tool's slot; showing a peer's preview may
  displace the player's own tool. Whether it can live outside an action is
  a Lua-side question to test.
- **Local verdict tint.** Its blue/red comes from the receiver's own
  `ProposalData` (below), not the sender's status. Overriding it still
  needs the error-colour hook, keyed perhaps by the `proposalId` in the
  parameter block `0x2aa3370` receives (`r13` there; not decoded).

The rest of this file is the native route, TPF2's design, with every entry
point found on this build. `0x2aa3370` is its reference implementation:
argument values below are read from it.

## 1. The factory context: `UI::RendererFactory`

**What it is.** Every tool constructor's RTTI lambda names its parameter
`class UI::RendererFactory const &` (for example
`UI::StreetBuilder::StreetBuilder(..., UI::RendererFactory const &, ...)`).
`0x8266d0` is `RendererFactory::Create() const`: `rcx` the factory, returns a
new `UI::BuilderRenderer*` (allocates `0x1d0` at `0x8266ea`, calls the
constructor `0x7b4160`). The factory's fields map one to one onto the
constructor's RTTI-named parameters (`UI::BuilderRenderer::BuilderRenderer(
IRenderContext&, PipelineManager&, RenderPassId, model::MaterialTypeRep const&,
ModelData const*, procedural::GeneratorRep const&, StreetStyleRep const*,
ModelTransformatorRep*, procedural::GeneratorCache*, ModelManager const&,
TextureManager*, TechniqueProvider*, RefCell<VaoManager>&,
terrain::RenderDataManager*, terrain::ViewTerrain*, std::function<float()>,
IGameStateProvider&, RefCell<OctreeSkipManager>&, EdgeObjectArrowRenderer&)`):

| factory offset | constructor argument |
|---|---|
| `+0x00`, `+0x08` | `IRenderContext&`, `PipelineManager&` |
| `+0x14` (dword) | `RenderPassId` (`+0x10` is a dword the factory does not pass) |
| `+0x18`, `+0x20`, `+0x28` | MaterialTypeRep, `ModelData const*`, GeneratorRep |
| `+0x78` | `StreetStyleRep const*` |
| `+0x30` .. `+0x68` | ModelTransformatorRep, GeneratorCache, ModelManager, TextureManager, TechniqueProvider, VaoManager, RenderDataManager, ViewTerrain |
| `+0x80` .. `+0xbf` | `std::function<float()>` (storage, impl pointer at `+0xb8`); copied per call by its impl's slot 0 (`0x826765`), passed by value |
| `+0xc0`, `+0xc8`, `+0xd0` | IGameStateProvider, OctreeSkipManager, EdgeObjectArrowRenderer |

Size `0xd8`, as TPF2's. Constructor `0x826560` (it moves the `std::function`
in through impl slot 1 or steals the heap pointer, slot 4 destroys); the
destructor of a stack copy is `0x6527c0` (destroys only the function).

**Where it lives.** There are two:

- A stack local in the `UI::CGameUI` constructor (`0x647860`, RTTI lambda
  `UI::CGameUI::CGameUI(CMenuUI*, CGame*, ...)`), at `[rbp+0x31f0]`, built at
  `0x649a28` and destroyed at `0x64cfa8`. It makes the tools the constructor
  creates. This is the one TPF2 caught by return address and cloned.
- **A heap one owned by the `CGameUI`, at `CGameUI+0x588`** (a
  `unique_ptr<RendererFactory>`). `UI::CGameUI::CreateUI` (`0x65af20`, RTTI
  lambda names) allocates `0xd8` at `0x65bcf8`, constructs it at
  `0x65bdff` and stores it at `0x65be0c` (`lea rax,[r12+0x588]`). Its
  function is `CreateUI::<lambda_9>` returning `float`, capturing the
  `CGameUI`. `CreateUI` has one caller, the `CGameUI` constructor
  (`0x64c15a`), so the field is set once per `CGameUI`; `~CGameUI` frees it
  (`0x6512ba`).

Code made after construction reads it directly:
`mov rcx,[rsi+0x588]; call 0x8266d0` in `MakeAssetManipulator(CGameUI*, CGame&)`
(`0x7350d8`) and three times in `InitializeActionDescriptorCache`
(`0x29f6a00`, `0x29f6c7d`, `0x29f6cbb`).

**Getting the `CGameUI`.** `UI::CMenuUI::StartGame` (`0x6a35e0`) stores the
new `CGameUI` at **`CMenuUI+0x6b8`** (`0x6a4f52`: `call 0x647860; nop;
mov [rbx+0x6b8],rax`); `UI::CMenuUI::StopGame` (`0x6a6d70`) deletes it and
clears the field (`0x6a6ffd`). The hook already detours `CMenuUI::DoStep`
(`0x6a0160`, `rcx` = `CMenuUI*`), so `*(CMenuUI+0x6b8)` is available every
frame. No Lua binding named `getGameUI` or `getMainRendererComponent`
exists in this build (no such strings).

**Verdict.** No factory hook and no `std::function` clone: read
`*(*(CMenuUI+0x6b8)+0x588)` when a renderer is needed and call `0x8266d0`
with it. Never cache the factory pointer across a `CGameUI` lifetime.

## 2. `AddToRenderer` and a tool's preview sequence

`0x5e2b20` is `UI::builder_renderer_util::AddToRenderer` (its assert strings
name it; `builder_renderer_util.cpp`). Two overloads exist (RTTI lambda
names); this is the one ending in three `bool`s, the one every caller uses:

```
void AddToRenderer(ModelData const&           rcx
                 , street_util::StreetToolkit const& rdx
                 , UI::BuilderRenderer*       r8
                 , construction_builder_util::Proposal const&     r9
                 , construction_builder_util::ProposalData const& [rsp+0x20]
                 , CVec3f const& offset                            [rsp+0x28]
                 , unordered_map<int, pair<ecs::Entity,float>> const& [rsp+0x30]
                 , bool                                            [rsp+0x38]
                 , bool  // true: enqueue catchment areas on the ThreadPool  [rsp+0x40]
                 , bool)                                           [rsp+0x48]
```

Callers (12): the tools' `vf5` (`Step(CRendererComponent*, float)`, for
example `UI::ConstructionBuilder::vf5 0x51cd60`, whose assert strings say
`UI::ConstructionBuilder::Step`), `UI::Bulldozer::vf5`,
`UI::StreetTerminalBuilder::vf5`, the street builder's `0x58a100`,
`UI::TrackModifier::vf5`, `UI::ModuleBuilder::vf5`, the ProposalViewer.
Tools pass `(..., &CVec3f{0,0,0}, &emptyMap, false, true, true)`
(`0x58a2ca`–`0x58a30f`). The ProposalViewer's setup (`0x2aa3acb`–`0x2aa3b0d`)
is the template:

```
rcx = *(CGameUI+0x538)                ; ModelData*
rdx = &toolkit                        ; built by 0x2458f0(&toolkit, gs+8, gs),
                                      ;   gs = (*(CGameUI+0x4c0))->vf1()
r8  = renderer
r9  = &proposal
[rsp+0x20] = &proposalData
[rsp+0x28] = &zero CVec3f, [rsp+0x30] = &empty map (load factor 1.0, 8 buckets)
[rsp+0x38] = 0, [rsp+0x40] = 1, [rsp+0x48] = 1
```

For a peer preview, `[rsp+0x40] = 0` avoids the catchment-area job on the
ThreadPool (`0x5e2bfa`); what that leaves out of the picture is not
confirmed.

**Inside `AddToRenderer`, in order:**

1. `BuilderRenderer::Begin(r, ...)` `0x7b9eb0` (`0x5e2cda`).
2. The error-colour setter `0x7c1ae0(r, hasErrors)` (`0x5e2d08`), where
   `hasErrors` is "`ProposalData+0x578` or `+0x590` non-empty" (the two
   error vectors; `0x5e2cdf`).
3. Entity loops: `InsertEntity` `0x7bc960` and its helpers `0x7b8ff0`,
   `0x7b90e0` over the proposal's entities.
4. `BeginHeightMod` `0x7ba0c0` (`0x5e316d`), `AddHeightMod` `0x7b9300` for
   each element of `ProposalData+0x3a8` (stride `0x28`), `EndHeightMod`
   `0x7bbae0` (`0x5e31ab`; its only caller).
5. `0x7b9e00`, `0x7b9a20`, `AddEntitiesToRenderer` `0x5df590`, `0x7bc5e0`,
   catchment areas (`0x7b8bd0` = `AddCatchmentAreaEdgeIds`), `0x5e25f0`, and
   last `0x7bb730(r, true)` (`0x5e34a6`; also called by six tools' `Step`,
   probably the end/commit; not confirmed).

So unlike TPF2 there is no separate Begin/End the caller must pair:
`Clear` then `AddToRenderer` is the whole refill.

**Terrain heights.** `EndHeightMod` uploads the height mods when
`r+0xf0` is set: `0x396a00(*(r+0x50), state+0x1af8, *(r+0xf3))`
(`0x7bbb8d`). `0x396a00` is `terrain::ViewTerrain::ApplyBlocks(vector<tuple<
CVec2i, CVec2i, unsigned short const*>> const&, bool)` (its RTTI lambdas);
`r+0x50` is the renderer's `terrain::ViewTerrain*`, shared by every
renderer, as in TPF2. The reset is `0x398600(ViewTerrain*, bool)`, reached
only from `Clear` when its second `bool` is set (`0x7ba7af`, passing
`*(r+0xf3)`). Whether it resets every block or only this renderer's is not
confirmed; TPF2's equivalent reset them all, which is why it re-composed
remote and local height buffers after every change. The same composition
is needed here: `ApplyBlocks(viewTerrain, state+0x1af8, flag)` per active
renderer, local last.

## 3. Clear, destructor, error colour

**Clear** `0x7ba590(BuilderRenderer*, bool dl, bool r8b, bool r9b)`: three
flags, TPF2's had two. 25 callers (tools' `Step`, `vf16`, `ProposalAction`).
`dl` releases the model instances (`0x7ba754`, under a spin lock at
`r+0xa0`, `0x818810`); `r8b` resets the view terrain (`0x398600`); `r9b`
handles the height-mod list at `state+0x1b80` against the view terrain
(`0x7ba8df`). The ProposalViewer always passes `(1, 1, 1)`. Exact meaning
of each flag beyond that: not confirmed.

**Destructor.** No separate body: `UI::BuilderRenderer::vf0` `0x7b89a0`
(`rcx` the renderer, `edx` the delete flag) is the scalar deleting
destructor with the body inlined. It destroys the state (`0x7b6bd0`, size
`0x1bd0`, at `r+0x1b8`), the `std::function` at `r+0x58`, puts back
`UI::IRenderable::vftable` and frees `0x1d0` when `edx & 1`. It does
**not** remove itself from a `CRendererComponent` and does not reset the
view terrain. Destroy with `vf0(r, 1)` only after `RemoveRenderable` and
`Clear(r, 1, 1, 1)`.

**Error colour.** `0x7c1ae0` (27 bytes, three callers:
`ConstructionBuilder::vf5`, `StreetBuilder::vf5`, `AddToRenderer`):

```
mov rax,[rcx+0x1b8]; mov [rax+0x193d],dl
mov rax,[rcx+0x1b8]; mov [rax+0x193c],dl
ret
```

It writes two bytes where TPF2's (`0x81df00`) wrote one (`state+0x1504`).
The other writers, as observed (semantics not confirmed):

| function | effect on `state+0x193c` / `+0x193d` |
|---|---|
| constructor helper `0x7b5730` | both 0 (`0x7b5c61`) |
| `BeginHeightMod` `0x7ba0c0` | `0x193c = 0x193d`, then `0x193d = 0` (`0x7ba2f5`) |
| `AddHeightMod` `0x7b9300` | `0x193d = 1` for an invalid mod (`0x7b94bf`), bypassing the setter, as in TPF2 |
| `EndHeightMod` `0x7bbae0` | first `0x193d = 0x193c` (`0x7bbb5d`), then reads `0x193d` to tint the terrain overlay (`0x7bbc02`) |
| render passes `0x7bc600`, `0x7befa0`, `0x7bf590` | read `0x193d` |

The renderer's palette is where TPF2's was. The constructor writes six
32-byte blocks at `r+0xf8` .. `r+0x1b7` (`0x7b4327`–`0x7b437f`); the render
code picks `r+0x118`/`r+0x128` when `0x193d` is 0 and `r+0x158`/`r+0x168`
when it is set (`0x7bc7db`–`0x7bc810`, `0x7bf11f`). The values (read from
`.rdata`): `+0x118` (0.40, 0.73, 1.00, 0.30) and `+0x128` (0.23, 0.60, 0.86,
1.00) blue; `+0x158` (0.90, 0, 0, 0.40) and `+0x168` (0.90, 0, 0, 1.00) red;
`+0x138`/`+0x178` yellow with alpha 0.6/0.3 (use not found); `+0xf8` grey,
`+0x198` magenta. So TPF2's "copy `palette+0x40` over `palette`" maps byte
for byte onto `r+0x118` (0x80 bytes) and also swaps the two yellow blocks.

**Selecting the sender's tint.** `AddToRenderer` calls the setter itself
(step 2), and `AddHeightMod` writes `0x193d` directly. Two ways:

- Hook `0x7c1ae0` and, for a peer's renderer, replace `dl` with the
  sender's verdict. Because `BeginHeightMod` saves and `EndHeightMod`
  restores `0x193d` from `0x193c`, the overlay then takes the sender's
  colour too, whatever `AddHeightMod` wrote in between. This replaces
  TPF2's `EndHeightMod` hook. Not confirmed in the game.
- Or, without any hook: after `AddToRenderer`, call the setter with the
  sender's verdict and overwrite the palette as TPF2 did. The terrain
  overlay is baked inside `EndHeightMod` with the local verdict, so this
  alone leaves the embankment the local colour.

## 4. Drawing: the main `CRendererComponent`

A tool does not draw its renderer itself. `BuilderRenderer` is a
`UI::IRenderable` (its destructor puts back `UI::IRenderable::vftable`,
11 slots). Tools register it in `vf15` (activate, `rdx` =
`CRendererComponent*`) with `AddRenderable` and remove it in `vf16`
(`UI::StreetBuilder::vf15 0x57c1d0`: `mov rdx,[r15+0x970]; mov rcx,r12;
call 0x6ae970`).

- **`UI::CRendererComponent::AddRenderable`** `0x6ae970(rc, IRenderable*)`:
  appends to `m_renderables` (`rc+0x530`..`+0x540`); asserts, fatally, if
  already present ("find(m_renderables.begin(), m_renderables.end(),
  renderable) == m_renderables.end()").
- **`RemoveRenderable`** `0x6afdf0(rc, IRenderable*)`.
- **The main one is `*(CGameUI+0xbf0)`**: `CreateUI` constructs it
  (`0x6ac820` at `0x65b1ab`), stores it at `0x65b1bc` and names it
  `"mainView"` (`0x65b1e9`). `CreateUI` registers sixteen of the `CGameUI`'s
  own renderables on it (`0x65b494`..), `~CGameUI` removes them.
  `InitializeActionDescriptorCache` receives the same component.
- The component calls renderables through slots `+0x10`..`+0x40` and
  `+0x50` (for example `0x6afe60` calls slot 3 and slot 10 for each).
  `BuilderRenderer` overrides slots 2, 4, 6, 7 and 9 (render passes; the
  RTTI lambda `BuilderRenderer::RenderNew(CRendererComponent*,
  engine::RenderHelper const&)` gives their likely arguments); slots 1, 3,
  5 and 8 are `ret` (`0x55b50`); slot 10 (`0xf5070`) returns true and its
  result is OR-ed over all renderables in `0x6afe60`, so it is not a
  per-renderable visibility gate.

TPF2 wrapped slots 1..6 to hide stale geometry and used slot 1 as an
expiry tick. Slot 1 is a no-op here; do expiry from the hook's existing
per-frame detour (`CMenuUI::DoStep`) by calling `Clear` on the GUI thread
instead of wrapping render passes.

**Teardown.** TPF2 hooked the scene destructor. Here:

- **`UI::CGameUI::~CGameUI`** body `0x650470` (called from
  `UI::CGameUI::vf0 0x658af0`; its own RTTI lambda name). At entry the
  `CGameUI` is whole: it removes its renderables from `+0xbf0` from
  `0x650701` on and frees the factory at `+0x588` near `0x6512ba`. Hook its
  entry: for each peer renderer, `Clear(r,1,1,1)`,
  `RemoveRenderable(*(this+0xbf0), r)`, `vf0(r, 1)`; then run the original.
  `StopGame` (`0x6a6d70`) detaches a child component (`0x270feb0` at
  `0x6a6fcd`, a generic swap-out of `CMenuUI+0x3c8`), deletes it through
  its `vf0` (`0x6a6fe4`) and then clears `CMenuUI+0x6b8` (`0x6a6ffd`);
  that the deleted child is the `CGameUI` is consistent but not confirmed.
- Alternative: `~CRendererComponent` body `0x6add40` (only caller
  `UI::CRendererComponent::vf0 0x6ae720`, object `0x630`). There are two
  components (`0x6ac820` is also called from `0x28965a0`), so filter on
  `rc == *(CGameUI+0xbf0)` as TPF2 filtered on its scene. When it runs
  relative to `~CGameUI` is not confirmed; the `~CGameUI` hook is safer.

## 5. `CreateProposalData`, `makeProposalData`, `scripting::Convert`

**`CreateProposalData`** `0xa1fd10` (`create_proposal_data.cpp`; 37
callers, simulation and UI):

```
ProposalData* CreateProposalData(ProposalData* out             rcx (returned)
                               , StreetToolkit const&           rdx
                               , T* r8   // *(*(gs+8)+0x50) in the UI, nullptr in StreetDeveloper
                               , Proposal const&                r9
                               , PreprocessResult const&        [rsp+0x28 at entry]
                               , U* opt  // nullptr from the UI paths
                               , Context const&)                [rsp+0x38 at entry]
```

It copies the preprocess result's flag to `out+0x570` and three vectors to
`out+0x578`, `+0x590`, `+0x5a8`, and computes the rest
(`CreateProposalDataImpl` `0xa1fea0`) only when that flag is 0
(`0xa1fdcc`). `0xa1fea0` uses the ThreadPool internally (its height-mod
lambdas run `ForEachThreaded`). TPF2's 6-argument call becomes 7, and its
`context(-1)` constructor becomes
`construction_builder_util::PreprocessProposal` `0xa3f9d0(PreprocessResult*
out, StreetToolkit const&, nullptr, Proposal const&, Context const&)`
(assert strings name it; 45 callers). The `Context` is `0x78` bytes
(copied by `0x4c6330`); a default one is built inline in `makeProposalData`
(`0x2511fef`–`0x2512096`).

Sizes: `ProposalData` is `0x5c0` (destructor `0x4cbb00`, which starts at
`+0x5a8`; move constructor `0x50e760`, move assignment `0x4cfc20`; an
`optional<ProposalData>`'s flag is at `+0x5c0`). `Proposal` is `0x358`
(destructor `0x48ee90`, copy `0x48bef0`; the optional's flag at `+0x358`).
TPF2 allocated `0x790` for its ProposalData.

**`makeProposalData`.** The name is registered at `0x2538b80` (in the
`api.engine.util.proposal` table builder `0x2536fc0`, with the argument
names `simpleProposal`, `context`) through the binding template
`0x246b100`, whose `__call` is `0x24fb0f0` (`0x24cb202`) →
`0x24e73c0` → the body **`0x2511ec0`** (`scripting\util_interface.cpp`):

1. toolkit `0x2458f0` (`0x2511f41`);
2. `scripting::Convert(&result, &toolkit, simpleProposal)` (`0x2511f58`);
   no result: return an empty optional;
3. the `Context`: the caller's (`0x4c6330`) or a default one;
4. `PreprocessProposal` (`0x25120b9`);
5. `CreateProposalData(&tmp, &toolkit, *(*(gs+8)+0x50), &proposal,
   &preprocess, nullptr, &context)` (`0x25122da`);
6. moves `tmp` into the caller's `optional<ProposalData>` (`0x25122e6`,
   flag `+0x5c0` at `0x25122eb`) and destroys `tmp` (`0x2512304`).

So Lua gets a **copy** of the `ProposalData` in its own userdata; the
converted `Proposal` is destroyed when the call returns. Taking the pointer
out of the userdata would need the luabridge userdata layout (not
confirmed), and `AddToRenderer` needs the `Proposal` as well, so this is
not the way in. The cleaner way in is the call itself:

- **A call-site detour on `0x25122da`** (the `call CreateProposalData`
  inside `makeProposalData`, pattern below, unique). At that instruction
  `rdx` is the toolkit, `r9` the converted `Proposal`, `rcx` the output;
  after the original returns the hook has toolkit, `Proposal` and
  `ProposalData` together, on the Lua caller's thread, and can call
  `Clear` + `AddToRenderer` for the peer the GUI named just before (TPF2's
  request/acknowledge, or a `tpf3mp_native` call). No native `Context` or
  preprocessing is needed: the game made them from the Lua arguments.
- Or TPF2's way: detour `scripting::Convert` and build the rest natively.

**`scripting::Convert`** `0x10c4fd0` (`scripting\proposal_util.cpp`; strings
`"scripting::Convert"`, `"Convert"`):
`optional<Proposal>* Convert(optional<Proposal>* out, StreetToolkit const&,
SimpleProposal const&)`, the same three-argument shape as TPF2's
`0x20e72f0`. Three callers: `makeProposalData` (`0x2511f58`), the
ProposalViewer (`0x2aa34a2`), and `0xe2a770` (Convert, preprocess, then the
command factory `0x9ee860`), which is `makeWorldBuildProposalCmd`'s body
(the name's only reference is `0xe38ae0`; its call chain `0xe21a20` →
`0xe1f330` → `0xe2a770` was not traced through the binding, not
confirmed). There is no `api.cmd.make.buildProposal` in TF3; that is the
equivalent.

**`makeProposalData` is in the GUI Lua state too** (high confidence,
static). The same registrar `0x2536fc0` names `makeProposalData`
(`0x2538b80`), `createProposalRemove`, `createBridgeOrTunnelProposal`,
`getBridgeOrTunnelRelatedRevisions` and
`didBridgeOrTunnelRelatedRevisionsChange` (`0x253900b`..`0x25392b7`); the
game's GUI script `bridge_and_tunnel.tl` calls the bridge functions. The
registrar is reached only through `SetupEngineInterface` (`0xf301b0`) ←
`0x1087b40`, whose callers are the engine side (`0x11c6c0`, `game.cpp`),
`UI::CMenuUI::SwitchToGameUI` (`0x69a160`, the GUI state) and a loader
(`0x27c6460`). So the detour below fires for engine-state calls too: the
mod's own `tpf3mp/apply.lua` calls `makeProposalData` from the engine state.

## 6. Pitfalls

- **Asserts are live and fatal.** `0x303d380` formats "{}:{}: {}{}
  Assertion '{}' failed" and the call is followed by `int3`. In
  `AddToRenderer`: "data.entity.GetId() >= 0" (`0x5e2dea`) fires when an
  entity in `ProposalData+0x2f8` is negative and found neither among the
  proposal's new nodes/edges (`ProposalData+0x00`, `+0x18`, id at `+0x18`)
  nor in the parallel-strip range (`-100000000 - i`); "entity.GetId() >= 0"
  (`0x5e2f65`) over the set at `ProposalData+0x350` (what it holds: not
  confirmed); "parallelShapesResult.shapeLists.size() ==
  parallelProposal.toAdd.size()" (`0x5e2d80`). A `ProposalData` made by
  `CreateProposalData` from a converted proposal satisfies these, as the
  game's own ProposalViewer path shows; a hand-edited one may not.
  `AddRenderable` asserts on a second registration.
- **Exceptions.** `CreateProposalData` throws on bad input; the mod saw
  `makeProposalData` raise "Unknown exception" from its worker threads
  (`docs/BUILDING.md`). In the Lua path the binding turns it into a Lua
  error. A detour that calls the original from Rust has a Rust frame in the
  unwind path: an `extern "C"` Rust frame aborts the game. Declare such
  detours `extern "C-unwind"` with no destructors in flight, or validate the
  proposal (bounds, ids, as TPF2's `safeProposal`) before any native call.
- **Threads.** Tools fill renderers in `Step` (their `vf5`) and register
  them in `vf15`, both driven by the `CGameUI` on the GUI thread; their
  `ProposalData` is computed on the ThreadPool (`ConstructionBuilder::Step`
  enqueues `ProposalDataProduct`, `StreetBuilder::CreateProposalAndUpdate`
  likewise). `Clear`, `AddToRenderer`, `AddRenderable` and the destructor
  belong on the GUI thread; `CreateProposalData` is not tied to it. The
  string "Render Thread" is passed to a compiled-out profiler scope
  (`0x55b50`, a bare `ret`) at the top of `CreateUI` and `~CGameUI`; whether
  render passes run on the same thread as the GUI step is not confirmed
  statically (TPF2 logged it at runtime; do the same).
- **Ownership.** The renderer belongs to whoever made it; the component
  holds a raw pointer. `ProposalData` from the hook's own call must be
  destroyed with `0x4cbb00`; one inside `makeProposalData` belongs to that
  call (the temporary) or to Lua (the copy). The factory belongs to the
  `CGameUI`.
- **Shared view terrain.** Every renderer's `r+0x50` is the one
  `terrain::ViewTerrain`; a local tool's `Clear` resets it, so remote height
  buffers need re-composing after local changes, as in TPF2.
- **Temporary ids.** `Convert` asserts that an edge object's edge is a new
  segment ("eo.edgeEntity.GetId() < 0 && eo.edgeEntity.GetId() >=
  -(int)result.proposal.addedSegments.size()"); keep TPF2's rule of
  isolated negative ids and no removals or edge objects for remote
  previews.

## TPF2 build 35924 → TF3 build 40408

| TPF2 entry (RVA) | TF3 | note |
|---|---|---|
| renderer vtable `30665e0` | `0x36dca68` | 11 slots; render passes 2, 4, 6, 7, 9 |
| renderer factory `859240` | `0x8266d0` | factory read from `CGameUI+0x588`, no clone |
| main scene Add / Remove `6d32e0` / `6d9290` | `0x6ae970` / `0x6afdf0` | component at `CGameUI+0xbf0` |
| scene destructor body `6d22d0` | `~CGameUI` `0x650470` (or `0x6add40`) | |
| `scripting::Convert` `20e72f0` | `0x10c4fd0` | same shape |
| CreateProposalData / destructor `a072b0` / `3e5030` | `0xa1fd10` / `0x4cbb00` | 7 arguments |
| context `431560(ctx,-1)` / `3e3d30` | `PreprocessProposal 0xa3f9d0` + `Context` (`0x78`) | |
| AddToRenderer `48d8e0` | `0x5e2b20` | ModelData and Proposal added; 10 arguments |
| Clear `817f70` (2 bools) | `0x7ba590` (3 bools) | |
| EndHeightMod `8191d0` | `0x7bbae0` | upload inside; setter hook replaces it |
| renderer destructor body `814b20` | inlined in `vf0` `0x7b89a0` | |
| upload / reset heights `34cd90` / `34e5a0` | `0x396a00` / `0x398600` | `ViewTerrain::ApplyBlocks` |
| error-colour setter `81df00` (`state+0x1504`) | `0x7c1ae0` (`state+0x193c`, `+0x193d`) | |
| renderer `+0x50`, `+0xf0`, `+0x118` palette, `+0x1b8` state | same offsets | object `0x1d0`, state `0x1bd0` |
| heights `state+0x16b0`, flag `r+0xf4` | `state+0x1af8`, flag `r+0xf3` | |

## Targets for the hook

Function starts: `tpfre q sig` output, each unique in `.text`. Call-site
and field anchors: byte patterns checked unique with `tpfre q bytes`.

| name | RVA | signature | confidence |
|---|---|---|---|
| `RendererFactory::Create` (call, no hook) | `0x8266d0` | `48 89 5C 24 08 55 56 57 41 54 41 55 41 56 41 57 48 81 EC F0 00 00 00 4C 8B E9` | confirmed (RTTI, field map) |
| `CGameUI+0x588` factory (anchor, `lea rax,[r12+0x588]` in `CreateUI`) | `0x65be0c` | `49 8D 84 24 88 05 00 00 48 8D 8D C0 03 00 00` | confirmed |
| `CGameUI+0xbf0` main component (anchor, store in `CreateUI`) | `0x65b1b8` | `48 89 4D 70 49 89 8C 24 F0 0B 00 00` (store at +4) | confirmed |
| `CMenuUI+0x6b8` = `CGameUI*` (anchor, after the ctor call) | `0x6a4f4c` | `E8 ?? ?? ?? ?? 90 48 89 83 B8 06 00 00` | confirmed |
| `CRendererComponent::AddRenderable` (call) | `0x6ae970` | `48 89 5C 24 08 48 89 74 24 18 48 89 54 24 10 57 48 83 EC 20 48 8B F2 48 8D B9 30 05 00 00` | confirmed (assert string) |
| `CRendererComponent::RemoveRenderable` (call) | `0x6afdf0` | `40 53 48 83 EC 20 4C 8B C2 48 8B D9 48 8B 91 38 05 00 00` | high (mirror of Add, tools' vf16) |
| `BuilderRenderer::Clear(r,bool,bool,bool)` (call) | `0x7ba590` | `48 89 5C 24 10 48 89 74 24 18 48 89 7C 24 20 55 41 54 41 55 41 56 41 57 48 8D 6C 24 C9 48 81 EC F0 00 00 00 45 0F B6 F9` | high (callers, ProposalViewer) |
| `BuilderRenderer` deleting destructor `vf0` (call) | `0x7b89a0` | `48 89 5C 24 10 48 89 6C 24 18 48 89 74 24 20 57 48 81 EC 10 01 00 00 48 8B 05 ?? ?? ?? ?? 48 33 C4 48 89 84 24 00 01 00 00` | confirmed (RTTI slot 0) |
| `builder_renderer_util::AddToRenderer` (call) | `0x5e2b20` | `40 55 53 56 57 41 54 41 55 41 56 41 57 48 8D AC 24 18 FE FF FF 48 81 EC E8 02 00 00 48 8B 05 ?? ?? ?? ?? 48 33 C4 48 89 85 D0 01 00 00 49 8B F1` | confirmed (assert string, RTTI) |
| error-colour setter (hook, or verify bytes and call) | `0x7c1ae0` | `48 8B 81 B8 01 00 00 88 90 3D 19 00 00 48 8B 81 B8 01 00 00` (whole body: `+ 88 90 3C 19 00 00 C3`) | high (behaviour); hook not tried |
| `BeginHeightMod` (reference) | `0x7ba0c0` | `48 89 5C 24 18 48 89 74 24 20 57 48 83 EC 50 C5 FA 10 02` | confirmed (assert string) |
| `AddHeightMod` (reference) | `0x7b9300` | `48 8B C4 48 89 58 18 55 56 57 41 54 41 55 41 56 41 57 48 81 EC C0 00 00 00` | confirmed (assert string) |
| `EndHeightMod` (hook only if the setter hook is not used) | `0x7bbae0` | `48 8B C4 48 89 58 10 48 89 70 18 48 89 78 20 55 41 54 41 55 41 56 41 57 48 8D 6C 24 90 48 81 EC 70 01 00 00 C5 F8 29 70 C8 C5 F8 29 78 B8 C5 78 29 40 A8` | high (position in AddToRenderer, file) |
| `ViewTerrain::ApplyBlocks` = height upload (call) | `0x396a00` | `48 89 5C 24 18 55 56 57 41 54 41 55 41 56 41 57 48 8D AC 24 50 FF FF FF 48 81 EC B0 01 00 00 C5 F8 29 B4 24 A0 01 00 00 48 8B 05 ?? ?? ?? ?? 48 33 C4 48 89 85 98 00 00 00` | confirmed (RTTI lambdas) |
| view terrain height reset (call) | `0x398600` | `48 89 5C 24 10 55 56 57 41 54 41 55 41 56 41 57 48 8D 6C 24 D9 48 81 EC C0 00 00 00 44 0F B6 FA` | medium (only caller is Clear) |
| `~CGameUI` body (hook: dispose before original) | `0x650470` | `48 89 5C 24 10 48 89 74 24 18 55 57 41 54 41 56 41 57 48 8D 6C 24 C0 48 81 EC 40 01 00 00 48 8B 05 ?? ?? ?? ?? 48 33 C4 48 89 45 38` | confirmed (RTTI) |
| `~CRendererComponent` body (alternative teardown) | `0x6add40` | `48 89 5C 24 10 48 89 74 24 18 57 48 81 EC 90 00 00 00 48 8B 05 ?? ?? ?? ?? 48 33 C4 48 89 84 24 80 00 00 00 48 8B D9 48 89 4C 24 30 33 F6` | confirmed (vtable write) |
| `CreateProposalData` (call) | `0xa1fd10` | `48 89 5C 24 10 48 89 6C 24 18 48 89 4C 24 08 56 57 41 56 48 83 EC 50 49 8B F1 49 8B E8` | confirmed (profile, file) |
| `ProposalData` destructor (call) | `0x4cbb00` | `48 89 5C 24 10 57 48 83 EC 20 48 8B D9 48 81 C1 A8 05 00 00` | high |
| `PreprocessProposal` (call, native route only) | `0xa3f9d0` | `40 53 56 57 41 54 41 55 41 56 41 57 48 81 EC 80 07 00 00` | confirmed (assert string) |
| `StreetToolkit` constructor (call, native route only) | `0x2458f0` | `48 89 5C 24 20 55 56 57 41 54 41 55 41 56 41 57 48 81 EC 50 02 00 00` | high (type flow into AddToRenderer) |
| `scripting::Convert` (TPF2-style hook) | `0x10c4fd0` | `48 8B C4 53 56 57 41 54 41 55 41 56 41 57 48 81 EC 00 0C 00 00` | confirmed (assert string) |
| `makeProposalData` body (reference) | `0x2511ec0` | `48 8B C4 48 89 58 18 55 56 57 41 54 41 55 41 56 41 57 48 8D A8 28 F3 FF FF` | high (binding chain) |
| `makeProposalData`'s call of `CreateProposalData` (call-site detour, `E8` at +25) | `0x25122c1` | `4C 8D 8D 40 03 00 00 4D 8B 45 50 48 8D 95 80 01 00 00 48 8D 8D A0 06 00 00 E8 ?? ?? ?? ??` | high; detour not tried |
| `makeProposalData`'s call of `Convert` (call-site, `E8` at +17) | `0x2511f47` | `4C 8B C3 48 8D 95 80 01 00 00 48 8D 8D 40 03 00 00 E8 ?? ?? ?? ??` | high |
| ProposalViewer step (reference only) | `0x2aa3370` | `40 55 53 56 57 41 54 41 55 41 56 41 57 48 8D AC 24 98 EF FF FF` | high |

## Call sequence

**Create a renderer for a peer** (GUI thread, in a game, `gameUI =
*(CMenuUI+0x6b8)` non-null):

1. `factory = *(gameUI+0x588)`; `r = 0x8266d0(factory)`; null → refuse.
2. `rc = *(gameUI+0xbf0)`; `0x6ae970(rc, r)` once; remember `(gameUI, r)`.
3. Keep a copy of the 0x80 palette bytes at `r+0x118`.

**Show or replace its proposal, ok or error tint:**

1. Mark the peer and tint as pending (GUI Lua tells the hook), then GUI Lua
   calls `api.engine.util.proposal.makeProposalData(simpleProposal,
   context)` with the rebuilt, isolated proposal.
2. In the detour on `0x25122da`: call the original `CreateProposalData`;
   if this is not the GUI thread (the thread id captured in the
   `CMenuUI::DoStep` detour, as TPF2 captured its GUI thread), pass the
   result through untouched: engine-state Lua calls the same function.
   Otherwise, if it returned a `ProposalData` whose flag `+0x570` is 0 and a peer is
   pending: `0x7ba590(r, 1, 1, 1)`; set the palette (sender's choice) and
   arm the setter override for `r`; `0x5e2b20(*(gameUI+0x538), rdx_toolkit,
   r, r9_proposal, out, &zeroVec3, &emptyMap, 0, 0, 1)`; disarm. Return
   `out` unchanged so Lua still gets its copy.
3. In the setter hook `0x7c1ae0`: if `rcx` is the armed peer renderer,
   replace `dl` with the sender's verdict (0 blue, 1 red).
4. Re-compose the view terrain: reset once, then `0x396a00(*(r+0x50),
   *(r+0x1b8)+0x1af8, *(r+0xf3))` for each active peer, local tool last
   (TPF2's `composeTerrain`; the reset call is not confirmed to be needed
   separately).

**Clear it** (cancel, tool switch, or expiry checked from the per-frame
`CMenuUI::DoStep` detour): `0x7ba590(r, 1, 1, 1)`, then re-compose the view
terrain for the remaining peers and the local tool. The renderer stays
registered.

**Destroy on teardown** (detour on `~CGameUI` `0x650470`, before the
original, `rcx` the `CGameUI`): for each peer renderer of this `CGameUI`:
`0x7ba590(r, 1, 1, 1)`; `0x6afdf0(*(rcx+0xbf0), r)`; `vf0(r, 1)`
(`0x7b89a0`). Forget every pointer, including any cached factory. Then run
the original.

**Resolved 2026-10-03:** `0x398600` resets every changed block of the
view terrain, not one renderer's: it walks the hash sets at
`ViewTerrain+0x80` (`0x398668`, `0x398725`). So any renderer's
`Clear(r, _, 1, _)` takes every renderer's heights away, and the hook
composes after each one (docs/HOOKS.md, "Their terrain").

## Not resolved

- Whether one `ProposalViewer` per peer works, and whether it can be
  mounted outside an `ActionDescriptor` without displacing the player's
  tool.
- The meaning of `Clear`'s three flags beyond what is listed, of
  `ProposalData+0x350`, and of `0x7bb730`.
- Whether render passes run on the GUI thread.
- That `makeProposalData` works from GUI Lua in the running game (static
  evidence says it is registered there).
- The luabridge layout of a `ProposalData` userdata.
