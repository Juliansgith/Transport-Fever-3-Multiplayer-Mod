# Hook targets in TransportFever3.exe

Made by tools/dayone/dayone.py on 2026-09-29 18:43.

Names carried from `TransportFever2.exe` (tpfre match): matched 10368 functions, 3942 of them named in the old build (calls 6165, file-order 154, rtti 2026, string 2023)
A carried name is shown with source `matched`; check it against the function's source file.

```
arch x86_64
binary_name TransportFever3.exe
binary_path F:\SteamLibrary\steamapps\common\Transport Fever 3\TransportFever3.exe
count.call_edges 1272835
count.data_pointers 58575
count.discovery_rounds 3
count.exports 119
count.functions 136538
count.functions_discovered 19304
count.functions_pdata 117234
count.imports 1186
count.instructions 13648000
count.invalid_instructions 19
count.jump_tables_skipped 931
count.rtti_class_type_descriptors 14578
count.rtti_complete_object_locators 7947
count.rtti_type_descriptors 14674
count.rtti_vtable_slots 46561
count.rtti_vtables 7947
count.strings 57773
count.xrefs 451701
dump 0
entry 0x4312310
format pe
header_image_base 0x140000000
image_base 0x140000000
matched_from C:\Users\Sepgi\Desktop\Transport fever modding\TPF3-MP\investigation\dayone-2026-09-29\TransportFever2.names-from.tpfdb
naming.assert_file_strings 906
naming.assert_funcsig_strings 23
naming.assert_pretty_strings 0
naming.distinct_source_files 858
naming.functions_file_ambiguous 794
naming.functions_file_direct 6204
naming.functions_file_inferred 31124
naming.functions_file_only 31124
naming.functions_named_ambiguous 3
naming.functions_named_funcsig 16
naming.functions_named_pretty 0
naming.references_resolved 12984
naming.source_prefix c:\gitlab-runner\builds\t1_difr8g\0\ug\urban_games\train_fever\src\
pe_timestamp 0x6AB69FE5
schema_version 1
sha256 de1daad3a13f3b7e9f79903361bb43769cf4f15e59271a263aefe1f075f23ef2
size 69711288
size_of_image 0x434c000
tool tpfre 0.1.0
section .text 0x1000 vsize=0x3662f9c raw=0x3662f9c r-x entropy=6.47
section .rdata 0x3664000 vsize=0x64f300 raw=0x64f300 r-- entropy=6.11
section .data 0x3cb4000 vsize=0x4548d8 raw=0x387600 rw- entropy=5.13
section .pdata 0x4109000 vsize=0x1983e4 raw=0x1983e4 r-- entropy=6.96
section _RDATA 0x42a2000 vsize=0x210 raw=0x210 r-- entropy=2.68
section .rsrc 0x42a3000 vsize=0x423d8 raw=0x423d8 r-- entropy=1.11
section .reloc 0x42e6000 vsize=0x2b3e4 raw=0x2b3e4 r-- entropy=5.51
section .bind 0x4312000 vsize=0x39248 raw=0x39248 r-x entropy=7.96
warning SteamStub (Steam DRM) section .bind present (entropy 7.96); the entry point is in it
warning code in .text (entropy 6.47) is not encrypted on disk: static analysis of it is valid
```

## the simulation step (`GameSim::Step\b`)
```
0x159390 GameSim::Step matched string(3)
```

## the game's step (`CGame::Step\b`)
```
0x11f3b0 CGame::Step matched string(2)
```

## the simulation loop (`CGame::RunGameSimLoop`)
```
0x89470 std::_Func_impl_no_alloc<class `private: void __cdecl CGame::RunGameSimLoop(void)'::`14'::<lambda_2>, void>::vf4 rtti inherited(1802)
0x8aea0 std::_Func_impl_no_alloc<class `private: void __cdecl CGame::RunGameSimLoop(void)'::`14'::<lambda_2>, void>::vf5 rtti folded(4140)
0x8aea0 std::_Func_impl_no_alloc<class `public: class JoiningFuture<void> __cdecl ThreadPool::Enqueue<void, class `private: void __cdecl CGame::RunGameSimLoop(void)'::`14'::<lambda_2> >(class `private: void __cdecl CGame::RunGameSimLoop(void)'::`14'::<lambda_2> &&, int, bool)'::`3'::<lambda_1>, void>::vf5 rtti folded(4140)
0x8b610 std::_Func_impl_no_alloc<class `public: class JoiningFuture<void> __cdecl ThreadPool::Enqueue<void, class `private: void __cdecl CGame::RunGameSimLoop(void)'::`14'::<lambda_2> >(class `private: void __cdecl CGame::RunGameSimLoop(void)'::`14'::<lambda_2> &&, int, bool)'::`3'::<lambda_1>, void>::vf1 rtti folded(2554)
0xefd90 std::_Func_impl_no_alloc<class `public: class JoiningFuture<void> __cdecl ThreadPool::Enqueue<void, class `private: void __cdecl CGame::RunGameSimLoop(void)'::`14'::<lambda_2> >(class `private: void __cdecl CGame::RunGameSimLoop(void)'::`14'::<lambda_2> &&, int, bool)'::`3'::<lambda_1>, void>::vf4 rtti inherited(227)
0x11e210 CGame::RunGameSimLoop matched string(3)
0x11fd60 std::_Func_impl_no_alloc<class `public: class JoiningFuture<void> __cdecl ThreadPool::Enqueue<void, class `private: void __cdecl CGame::RunGameSimLoop(void)'::`14'::<lambda_2> >(class `private: void __cdecl CGame::RunGameSimLoop(void)'::`14'::<lambda_2> &&, int, bool)'::`3'::<lambda_1>, void>::vf0 rtti unique
0x11fe70 std::_Func_impl_no_alloc<class `private: void __cdecl CGame::RunGameSimLoop(void)'::`14'::<lambda_2>, void>::vf0 rtti folded(2)
0x11fe70 std::_Func_impl_no_alloc<class `private: void __cdecl CGame::RunGameSimLoop(void)'::`14'::<lambda_2>, void>::vf1 rtti folded(2)
0x120080 std::_Func_impl_no_alloc<class `public: class JoiningFuture<void> __cdecl ThreadPool::Enqueue<void, class `private: void __cdecl CGame::RunGameSimLoop(void)'::`14'::<lambda_2> >(class `private: void __cdecl CGame::RunGameSimLoop(void)'::`14'::<lambda_2> &&, int, bool)'::`3'::<lambda_1>, void>::vf2 rtti inherited(144)
0x120390 std::_Func_impl_no_alloc<class `private: void __cdecl CGame::RunGameSimLoop(void)'::`14'::<lambda_2>, void>::vf2 rtti inherited(2)
0x120c20 std::_Func_impl_no_alloc<class `public: class JoiningFuture<void> __cdecl ThreadPool::Enqueue<void, class `private: void __cdecl CGame::RunGameSimLoop(void)'::`14'::<lambda_2> >(class `private: void __cdecl CGame::RunGameSimLoop(void)'::`14'::<lambda_2> &&, int, bool)'::`3'::<lambda_1>, void>::vf3 rtti unique
0x120c70 std::_Func_impl_no_alloc<class `private: void __cdecl CGame::RunGameSimLoop(void)'::`14'::<lambda_2>, void>::vf3 rtti unique
0x367be00 std::_Func_impl_no_alloc<class `public: class JoiningFuture<void> __cdecl ThreadPool::Enqueue<void, class `private: void __cdecl CGame::RunGameSimLoop(void)'::`14'::<lambda_2> >(class `private: void __cdecl CGame::RunGameSimLoop(void)'::`14'::<lambda_2> &&, int, bool)'::`3'::<lambda_1>, void>::vftable rtti vtable
0x367be38 std::_Func_impl_no_alloc<class `private: void __cdecl CGame::RunGameSimLoop(void)'::`14'::<lambda_2>, void>::vftable rtti vtable
```

## the command queue's add (`CommandList::Add`)
```
0x9d23c0 CommandList::Add::<lambda_7152b1228d6642b2ea6daae905652661>::operator matched string(1)
```

## the command factories (`make_cmd::`)
```
0x9ee1f0 make_cmd::SellVehicle matched string(1)
```

## game speed (`CGameTime::`)
```
0x55b50 CGameTime::vf10 rtti folded(1902)
0x55b50 CGameTime::vf11 rtti folded(1902)
0x55b50 CGameTime::vf3 rtti folded(1902)
0x55b50 CGameTime::vf6 rtti folded(1902)
0x55b50 CGameTime::vf7 rtti folded(1902)
0x63bf0 std::_Ref_count_obj2<struct CGameTime::CGameTimeData>::vf1 rtti inherited(383)
0x8b610 std::_Ref_count_obj2<struct CGameTime::CGameTimeData>::vf3 rtti folded(2554)
0x2a83c0 std::_Ref_count_obj2<struct CGameTime::CGameTimeData>::vf2 rtti unique
0x2a8750 CGameTime::vf0 rtti unique
0x2a88f0 CGameTime::vf1 rtti unique
0x2a89b0 CGameTime::vf7 matched calls
0x2a89b0 CGameTime::vf8 rtti unique
0x2a8a50 CGameTime::ComponentChanged matched calls
0x2a8a50 CGameTime::vf9 rtti unique
0x2a8c50 CGameTime::vf3 matched string(2)
0x2a8c50 CGameTime::vf4 rtti unique
0x2a8d90 CGameTime::EntityToBeRemoved matched file-order
0x2a8d90 CGameTime::vf5 rtti unique
0x2a9130 CGameTime::GetDate matched string(1)
0x2a9430 CGameTime::GetDayNumber matched string(1)
0x2a9570 CGameTime::vf1 matched file-order
0x2a9570 CGameTime::vf2 rtti unique
0x2a96a0 CGameTime::OnIntervalStep matched string(1)
0x2a9a40 std::_Ref_count_obj2<struct CGameTime::CGameTimeData>::vf0 rtti unique
0x3684780 CGameTime::vftable rtti vtable
```

## starting a savegame (`CMenuUI::StartSavegame`)
```
0x89470 std::_Func_impl_no_alloc<class `public: __cdecl `public: __cdecl `public: __cdecl `public: bool __cdecl UI::CMenuUI::StartSavegame(struct UI::LoadGameParams const &, struct SavegameInfo const &)'::`2'::<lambda_1>::operator()(void) const'::`2'::<lambda_2>::operator()(void) const'::`2'::<lambda_1>::operator()(struct Checked<struct CheckedException>) const'::`2'::<lambda_1>, void, class GameRes const &>::vf4 rtti inherited(1802)
0x89470 std::_Func_impl_no_alloc<class `public: void __cdecl UI::CMenuUI::StartSavegameFallbackToMainMenu(struct UI::LoadGameParams const &, class std::optional<struct SavegameInfo> const &)'::`5'::<lambda_1>, void>::vf4 rtti inherited(1802)
0x8aea0 std::_Func_impl_no_alloc<class `public: __cdecl `public: __cdecl `public: __cdecl `public: bool __cdecl UI::CMenuUI::StartSavegame(struct UI::LoadGameParams const &, struct SavegameInfo const &)'::`2'::<lambda_1>::operator()(void) const'::`2'::<lambda_2>::operator()(void) const'::`2'::<lambda_1>::operator()(struct Checked<struct CheckedException>) const'::`2'::<lambda_1>, void, class GameRes const &>::vf5 rtti folded(4140)
0x8aea0 std::_Func_impl_no_alloc<class `public: __cdecl `public: __cdecl `public: bool __cdecl UI::CMenuUI::StartSavegame(struct UI::LoadGameParams const &, struct SavegameInfo const &)'::`2'::<lambda_1>::operator()(void) const'::`2'::<lambda_2>::operator()(void) const'::`2'::<lambda_1>, class std::unique_ptr<class CGame, struct std::default_delete<class CGame> >, struct Checked<struct CheckedException> >::vf5 rtti folded(4140)
0x8aea0 std::_Func_impl_no_alloc<class `public: __cdecl `public: bool __cdecl UI::CMenuUI::StartSavegame(struct UI::LoadGameParams const &, struct SavegameInfo const &)'::`2'::<lambda_1>::operator()(void) const'::`2'::<lambda_1>, class lua::Loader, char const *>::vf5 rtti folded(4140)
0x8aea0 std::_Func_impl_no_alloc<class `public: __cdecl `public: bool __cdecl UI::CMenuUI::StartSavegame(struct UI::LoadGameParams const &, struct SavegameInfo const &)'::`2'::<lambda_1>::operator()(void) const'::`2'::<lambda_2>, void>::vf5 rtti folded(4140)
0x8aea0 std::_Func_impl_no_alloc<class `public: bool __cdecl UI::CMenuUI::StartSavegame(struct UI::LoadGameParams const &, struct SavegameInfo const &)'::`2'::<lambda_1>, void>::vf5 rtti folded(4140)
0x8aea0 std::_Func_impl_no_alloc<class `public: void __cdecl UI::CMenuUI::StartSavegameFallbackToMainMenu(struct UI::LoadGameParams const &, class std::optional<struct SavegameInfo> const &)'::`5'::<lambda_1>, void>::vf5 rtti folded(4140)
0x8b610 std::_Func_impl_no_alloc<class `public: __cdecl `public: __cdecl `public: bool __cdecl UI::CMenuUI::StartSavegame(struct UI::LoadGameParams const &, struct SavegameInfo const &)'::`2'::<lambda_1>::operator()(void) const'::`2'::<lambda_2>::operator()(void) const'::`2'::<lambda_1>, class std::unique_ptr<class CGame, struct std::default_delete<class CGame> >, struct Checked<struct CheckedException> >::vf1 rtti folded(2554)
0x8b610 std::_Func_impl_no_alloc<class `public: __cdecl `public: bool __cdecl UI::CMenuUI::StartSavegame(struct UI::LoadGameParams const &, struct SavegameInfo const &)'::`2'::<lambda_1>::operator()(void) const'::`2'::<lambda_2>, void>::vf1 rtti folded(2554)
0x8b610 std::_Func_impl_no_alloc<class `public: bool __cdecl UI::CMenuUI::StartSavegame(struct UI::LoadGameParams const &, struct SavegameInfo const &)'::`2'::<lambda_1>, void>::vf1 rtti folded(2554)
0x5486c0 std::_Func_impl_no_alloc<class `public: __cdecl `public: bool __cdecl UI::CMenuUI::StartSavegame(struct UI::LoadGameParams const &, struct SavegameInfo const &)'::`2'::<lambda_1>::operator()(void) const'::`2'::<lambda_1>, class lua::Loader, char const *>::vf4 rtti inherited(6)
0x6a8080 std::_Func_impl_no_alloc<class `public: __cdecl `public: __cdecl `public: __cdecl `public: bool __cdecl UI::CMenuUI::StartSavegame(struct UI::LoadGameParams const &, struct SavegameInfo const &)'::`2'::<lambda_1>::operator()(void) const'::`2'::<lambda_2>::operator()(void) const'::`2'::<lambda_1>::operator()(struct Checked<struct CheckedException>) const'::`2'::<lambda_1>, void, class GameRes const &>::vf0 rtti folded(2)
0x6a8080 std::_Func_impl_no_alloc<class `public: __cdecl `public: __cdecl `public: __cdecl `public: bool __cdecl UI::CMenuUI::StartSavegame(struct UI::LoadGameParams const &, struct SavegameInfo const &)'::`2'::<lambda_1>::operator()(void) const'::`2'::<lambda_2>::operator()(void) const'::`2'::<lambda_1>::operator()(struct Checked<struct CheckedException>) const'::`2'::<lambda_1>, void, class GameRes const &>::vf1 rtti folded(2)
0x6a8220 std::_Func_impl_no_alloc<class `public: __cdecl `public: bool __cdecl UI::CMenuUI::StartSavegame(struct UI::LoadGameParams const &, struct SavegameInfo const &)'::`2'::<lambda_1>::operator()(void) const'::`2'::<lambda_1>, class lua::Loader, char const *>::vf0 rtti unique
0x6a82d0 std::_Func_impl_no_alloc<class `public: __cdecl `public: __cdecl `public: bool __cdecl UI::CMenuUI::StartSavegame(struct UI::LoadGameParams const &, struct SavegameInfo const &)'::`2'::<lambda_1>::operator()(void) const'::`2'::<lambda_2>::operator()(void) const'::`2'::<lambda_1>, class std::unique_ptr<class CGame, struct std::default_delete<class CGame> >, struct Checked<struct CheckedException> >::vf0 rtti unique
0x6a8590 std::_Func_impl_no_alloc<class `public: bool __cdecl UI::CMenuUI::StartSavegame(struct UI::LoadGameParams const &, struct SavegameInfo const &)'::`2'::<lambda_1>, void>::vf0 rtti unique
0x6a8a50 std::_Func_impl_no_alloc<class `public: void __cdecl UI::CMenuUI::StartSavegameFallbackToMainMenu(struct UI::LoadGameParams const &, class std::optional<struct SavegameInfo> const &)'::`5'::<lambda_1>, void>::vf0 rtti folded(2)
0x6a8a50 std::_Func_impl_no_alloc<class `public: void __cdecl UI::CMenuUI::StartSavegameFallbackToMainMenu(struct UI::LoadGameParams const &, class std::optional<struct SavegameInfo> const &)'::`5'::<lambda_1>, void>::vf1 rtti folded(2)
0x6a8b40 std::_Func_impl_no_alloc<class `public: __cdecl `public: bool __cdecl UI::CMenuUI::StartSavegame(struct UI::LoadGameParams const &, struct SavegameInfo const &)'::`2'::<lambda_1>::operator()(void) const'::`2'::<lambda_2>, void>::vf0 rtti unique
0x6a91c0 std::_Func_impl_no_alloc<class `public: __cdecl `public: __cdecl `public: bool __cdecl UI::CMenuUI::StartSavegame(struct UI::LoadGameParams const &, struct SavegameInfo const &)'::`2'::<lambda_1>::operator()(void) const'::`2'::<lambda_2>::operator()(void) const'::`2'::<lambda_1>, class std::unique_ptr<class CGame, struct std::default_delete<class CGame> >, struct Checked<struct CheckedException> >::vf4 rtti unique
0x6a9280 std::_Func_impl_no_alloc<class `public: bool __cdecl UI::CMenuUI::StartSavegame(struct UI::LoadGameParams const &, struct SavegameInfo const &)'::`2'::<lambda_1>, void>::vf4 rtti unique
0x6a9300 std::_Func_impl_no_alloc<class `public: __cdecl `public: bool __cdecl UI::CMenuUI::StartSavegame(struct UI::LoadGameParams const &, struct SavegameInfo const &)'::`2'::<lambda_1>::operator()(void) const'::`2'::<lambda_2>, void>::vf4 rtti unique
0x6a9530 std::_Func_impl_no_alloc<class `public: __cdecl `public: __cdecl `public: __cdecl `public: bool __cdecl UI::CMenuUI::StartSavegame(struct UI::LoadGameParams const &, struct SavegameInfo const &)'::`2'::<lambda_1>::operator()(void) const'::`2'::<lambda_2>::operator()(void) const'::`2'::<lambda_1>::operator()(struct Checked<struct CheckedException>) const'::`2'::<lambda_1>, void, class GameRes const &>::vf2 rtti unique
0x6a9700 std::_Func_impl_no_alloc<class `public: __cdecl `public: bool __cdecl UI::CMenuUI::StartSavegame(struct UI::LoadGameParams const &, struct SavegameInfo const &)'::`2'::<lambda_1>::operator()(void) const'::`2'::<lambda_1>, class lua::Loader, char const *>::vf2 rtti inherited(2)
```

## menu pages (`CMenuUI::CreatePage`)
```
0x8aea0 std::_Func_impl_no_alloc<class `private: void __cdecl UI::CMenuUI::CreatePageSplash(struct lua::Table const &)'::`2'::<lambda_1>, void>::vf5 rtti folded(4140)
0x8aea0 std::_Func_impl_no_alloc<class `public: __cdecl `private: void __cdecl UI::CMenuUI::CreatePageSplash(struct lua::Table const &)'::`2'::<lambda_1>::operator()(void) const'::`2'::<lambda_1>, void>::vf5 rtti folded(4140)
0x8b610 std::_Func_impl_no_alloc<class `private: void __cdecl UI::CMenuUI::CreatePageSplash(struct lua::Table const &)'::`2'::<lambda_1>, void>::vf1 rtti folded(2554)
0x8b610 std::_Func_impl_no_alloc<class `public: __cdecl `private: void __cdecl UI::CMenuUI::CreatePageSplash(struct lua::Table const &)'::`2'::<lambda_1>::operator()(void) const'::`2'::<lambda_1>, void>::vf1 rtti folded(2554)
0x6a2ee0 UI::CMenuUI::CreatePage matched string(1)
0x6a80a0 std::_Func_impl_no_alloc<class `public: __cdecl `private: void __cdecl UI::CMenuUI::CreatePageSplash(struct lua::Table const &)'::`2'::<lambda_1>::operator()(void) const'::`2'::<lambda_1>, void>::vf0 rtti unique
0x6a8440 std::_Func_impl_no_alloc<class `private: void __cdecl UI::CMenuUI::CreatePageSplash(struct lua::Table const &)'::`2'::<lambda_1>, void>::vf0 rtti unique
0x6a9110 std::_Func_impl_no_alloc<class `private: void __cdecl UI::CMenuUI::CreatePageSplash(struct lua::Table const &)'::`2'::<lambda_1>, void>::vf4 rtti inherited(2)
0x6a9110 std::_Func_impl_no_alloc<class `public: __cdecl `private: void __cdecl UI::CMenuUI::CreatePageSplash(struct lua::Table const &)'::`2'::<lambda_1>::operator()(void) const'::`2'::<lambda_1>, void>::vf4 rtti inherited(2)
0x6a9560 std::_Func_impl_no_alloc<class `public: __cdecl `private: void __cdecl UI::CMenuUI::CreatePageSplash(struct lua::Table const &)'::`2'::<lambda_1>::operator()(void) const'::`2'::<lambda_1>, void>::vf2 rtti unique
0x6a9e80 std::_Func_impl_no_alloc<class `private: void __cdecl UI::CMenuUI::CreatePageSplash(struct lua::Table const &)'::`2'::<lambda_1>, void>::vf2 rtti unique
0x6ab160 std::_Func_impl_no_alloc<class `public: __cdecl `private: void __cdecl UI::CMenuUI::CreatePageSplash(struct lua::Table const &)'::`2'::<lambda_1>::operator()(void) const'::`2'::<lambda_1>, void>::vf3 rtti unique
0x6ab240 std::_Func_impl_no_alloc<class `private: void __cdecl UI::CMenuUI::CreatePageSplash(struct lua::Table const &)'::`2'::<lambda_1>, void>::vf3 rtti unique
0x36c9558 std::_Func_impl_no_alloc<class `public: __cdecl `private: void __cdecl UI::CMenuUI::CreatePageSplash(struct lua::Table const &)'::`2'::<lambda_1>::operator()(void) const'::`2'::<lambda_1>, void>::vftable rtti vtable
0x36c9590 std::_Func_impl_no_alloc<class `private: void __cdecl UI::CMenuUI::CreatePageSplash(struct lua::Table const &)'::`2'::<lambda_1>, void>::vftable rtti vtable
```

## saving (`(?i)save(game)?::|::Save\b|SaveGame`)
```
0x55b50 UI::SavegameDialog::vf48 rtti folded(1902)
0x55b50 UI::SavegameDialog::vf50 rtti folded(1902)
0x55b50 UI::SavegameDialog::vf51 rtti folded(1902)
0x55b50 UI::SavegameDialog::vf52 rtti folded(1902)
0x55b50 UI::SavegameDialog::vf53 rtti folded(1902)
0x55b50 UI::SavegameDialog::vf54 rtti folded(1902)
0x55b50 UI::SavegameDialog::vf55 rtti folded(1902)
0x55b50 UI::SavegameDialog::vf56 rtti folded(1902)
0x55b50 UI::SavegameList::vf48 rtti folded(1902)
0x55b50 UI::SavegameList::vf50 rtti folded(1902)
0x55b50 UI::SavegameList::vf51 rtti folded(1902)
0x55b50 UI::SavegameList::vf52 rtti folded(1902)
0x55b50 UI::SavegameList::vf53 rtti folded(1902)
0x55b50 UI::SavegameList::vf54 rtti folded(1902)
0x55b50 UI::SavegameList::vf55 rtti folded(1902)
0x55b50 UI::SavegameList::vf56 rtti folded(1902)
0x55b50 platform::StandardSaveGameBackend::vf13 rtti folded(1902)
0x55b50 platform::StandardSaveGameBackend::vf19 rtti folded(1902)
0x55b50 std::_Associated_state<class std::optional<struct std::pair<struct SavegameInfo, struct UI::LoadGameParams> > >::vf4 rtti folded(1902)
0x55b50 std::_Associated_state<struct scripting::app_script_util::SaveGameData>::vf4 rtti folded(1902)
0x55b50 std::_Packaged_state<class std::optional<struct std::pair<struct SavegameInfo, struct UI::LoadGameParams> > __cdecl (void)>::vf4 rtti folded(1902)
0x55b50 std::_Packaged_state<struct scripting::app_script_util::SaveGameData __cdecl (void)>::vf4 rtti folded(1902)
0x63bf0 std::_Ref_count_obj2<class UI::`anonymous namespace'::ResSavegameListDataProvider>::vf1 rtti inherited(383)
0x63bf0 std::_Ref_count_obj2<class std::packaged_task<class std::optional<struct std::pair<struct SavegameInfo, struct UI::LoadGameParams> > __cdecl (void)> >::vf1 rtti inherited(383)
0x63bf0 std::_Ref_count_obj2<class std::packaged_task<struct scripting::app_script_util::SaveGameData __cdecl (void)> >::vf1 rtti inherited(383)
```

## loading (`(?i)::Load(Game)?\b|LoadGame`)
```
0x55b50 std::_Associated_state<class std::optional<struct std::pair<struct SavegameInfo, struct UI::LoadGameParams> > >::vf4 rtti folded(1902)
0x55b50 std::_Associated_state<struct UI::CMenuUI::LoadGameResult>::vf4 rtti folded(1902)
0x55b50 std::_Associated_state<struct UI::CMenuUI::PreLoadGameData::AsyncResult>::vf4 rtti folded(1902)
0x55b50 std::_Packaged_state<class std::optional<struct std::pair<struct SavegameInfo, struct UI::LoadGameParams> > __cdecl (void)>::vf4 rtti folded(1902)
0x55b50 std::_Packaged_state<struct UI::CMenuUI::LoadGameResult __cdecl (void)>::vf4 rtti folded(1902)
0x55b50 std::_Packaged_state<struct UI::CMenuUI::PreLoadGameData::AsyncResult __cdecl (void)>::vf4 rtti folded(1902)
0x63bf0 std::_Ref_count_obj2<class std::packaged_task<class std::optional<struct std::pair<struct SavegameInfo, struct UI::LoadGameParams> > __cdecl (void)> >::vf1 rtti inherited(383)
0x63bf0 std::_Ref_count_obj2<class std::packaged_task<struct UI::CMenuUI::LoadGameResult __cdecl (void)> >::vf1 rtti inherited(383)
0x63bf0 std::_Ref_count_obj2<class std::packaged_task<struct UI::CMenuUI::PreLoadGameData::AsyncResult __cdecl (void)> >::vf1 rtti inherited(383)
0x89470 std::_Func_impl_no_alloc<class `public: __cdecl `public: __cdecl `public: __cdecl `public: bool __cdecl UI::CMenuUI::StartSavegame(struct UI::LoadGameParams const &, struct SavegameInfo const &)'::`2'::<lambda_1>::operator()(void) const'::`2'::<lambda_2>::operator()(void) const'::`2'::<lambda_1>::operator()(struct Checked<struct CheckedException>) const'::`2'::<lambda_1>, void, class GameRes const &>::vf4 rtti inherited(1802)
0x89470 std::_Func_impl_no_alloc<class `public: __cdecl `public: struct UI::react::Node const * __cdecl UI::react::ReactFramework::Load(int, int, int, bool, int, class std::function<void __cdecl (struct Checked<struct CheckedException>, class UI::react::TransformRegistry const &, struct UI::react::Storage4&, int, class std::optional<int>)>, bool, bool, bool, class std::function<void __cdecl (float)>)'::`3'::<lambda_1>::operator()(struct Checked<struct CheckedException>) const'::`12'::<lambda_1>, void>::vf4 rtti inherited(1802)
0x89470 std::_Func_impl_no_alloc<class `public: void __cdecl CGame::Load(bool, bool, bool, class std::function<struct GuiSaveData __cdecl (class GameRes &, class GameState &, class std::function<void __cdecl (class IProgressMonitor &)> const &, class IProgressMonitor &)> const &, class IProgressMonitor &)'::`2'::<lambda_1>, void, class IProgressMonitor &>::vf4 rtti inherited(1802)
0x89470 std::_Func_impl_no_alloc<class `public: void __cdecl UI::CMenuUI::StartSavegameFallbackToMainMenu(struct UI::LoadGameParams const &, class std::optional<struct SavegameInfo> const &)'::`5'::<lambda_1>, void>::vf4 rtti inherited(1802)
0x8a8f0 std::_Associated_state<class std::optional<struct std::pair<struct SavegameInfo, struct UI::LoadGameParams> > >::vf3 rtti folded(284)
0x8a8f0 std::_Associated_state<struct UI::CMenuUI::LoadGameResult>::vf3 rtti folded(284)
0x8a8f0 std::_Associated_state<struct UI::CMenuUI::PreLoadGameData::AsyncResult>::vf3 rtti folded(284)
0x8a8f0 std::_Packaged_state<class std::optional<struct std::pair<struct SavegameInfo, struct UI::LoadGameParams> > __cdecl (void)>::vf3 rtti folded(284)
0x8a8f0 std::_Packaged_state<struct UI::CMenuUI::LoadGameResult __cdecl (void)>::vf3 rtti folded(284)
0x8a8f0 std::_Packaged_state<struct UI::CMenuUI::PreLoadGameData::AsyncResult __cdecl (void)>::vf3 rtti folded(284)
0x8aea0 std::_Func_impl_no_alloc<class `class std::unique_ptr<class CGame, struct std::default_delete<class CGame> > __cdecl LoadGame(struct Checked<struct CheckedException>, class IBaseFileSystem const &, class res_util::ResourceCache const &, class GameLoadingStreamData &, struct GameConfigData const &, struct GameUserProfileContext const &, class ModRep const &, class std::function<void __cdecl (class GameRes const &)> const &, struct platform::SaveGameId const &, bool, bool, bool, bool, class ResName const &, class IProgressMonitor &)'::`4'::<lambda_1>, struct GuiSaveData, class GameRes &, class GameState &, class std::function<void __cdecl (class IProgressMonitor &)> const &, class IProgressMonitor &>::vf5 rtti folded(4140)
0x8aea0 std::_Func_impl_no_alloc<class `public: __cdecl `public: __cdecl UI::CGameUI::CGameUI(class UI::CMenuUI *, class CGame *, class UserProfile *, class std::unique_ptr<class UI::GameStateProvider, struct std::default_delete<class UI::GameStateProvider> >, class std::unique_ptr<class GameUIRes, struct std::default_delete<class GameUIRes> >, class engine::GLContext &, class std::unique_ptr<class PipelineManager, struct std::default_delete<class PipelineManager> >, class std::unique_ptr<class RenderPassProvider, struct std::default_delete<class RenderPassProvider> >, class std::unique_ptr<class MaterialUboManager, struct std::default_delete<class MaterialUboManager> >, class std::unique_ptr<class TechniqueProvider, struct std::default_delete<class TechniqueProvider> >, class std::unique_ptr<struct ModelData, struct std::default_delete<struct ModelData> >, class std::unique_ptr<class procedural::GeneratorRep, struct std::default_delete<class procedural::GeneratorRep> >, class std::unique_ptr<class DynModelData, struct std::default_delete<class DynModelData> >, class std::unique_ptr<class MeshVBOManager, struct std::default_delete<class MeshVBOManager> >, class std::unique_ptr<class RefCell<class VaoManager>, struct std::default_delete<class RefCell<class VaoManager> > >, class std::unique_ptr<class engine::ColorMap, struct std::default_delete<class engine::ColorMap> >, class std::unique_ptr<class CollisionShapeRep, struct std::default_delete<class CollisionShapeRep> >, class std::unique_ptr<class procedural::GeneratorCache, struct std::default_delete<class procedural::GeneratorCache> >, class std::unique_ptr<class ModelDescriptorManager, struct std::default_delete<class ModelDescriptorManager> >, class std::unique_ptr<class ModelManager, struct std::default_delete<class ModelManager> >, class std::unique_ptr<class terrain::RenderDataManager, struct std::default_delete<class terrain::RenderDataManager> >, class std::unique_ptr<class terrain::ViewTerrain, struct std::default_delete<class terrain::ViewTerrain> >, class std::unique_ptr<class CRenderer, struct std::default_delete<class CRenderer> >, class std::unique_ptr<class UI::TransformatorManager, struct std::default_delete<class UI::TransformatorManager> >, class std::unique_ptr<class UI::react::ScriptComponentRoot, struct std::default_delete<class UI::react::ScriptComponentRoot> >, class MusicPlayer *, class IProgressMonitor &, bool, class std::basic_string<char, struct std::char_traits<char>, class std::allocator<char> > const &)'::`2'::<lambda_53>::operator()(struct platform::SystemEvent const &) const'::`5'::<lambda_1>, class std::optional<struct std::pair<struct SavegameInfo, struct UI::LoadGameParams> > >::vf5 rtti folded(4140)
0x8aea0 std::_Func_impl_no_alloc<class `public: __cdecl `public: __cdecl UI::CMenuUI::CMenuUI(class IBaseFileSystem const &, class engine::GLContext &, bool)'::`2'::<lambda_9>::operator()(struct platform::SystemEvent const &) const'::`5'::<lambda_1>, class std::optional<struct std::pair<struct SavegameInfo, struct UI::LoadGameParams> > >::vf5 rtti folded(4140)
0x8aea0 std::_Func_impl_no_alloc<class `public: __cdecl `public: __cdecl `public: __cdecl `public: bool __cdecl UI::CMenuUI::StartSavegame(struct UI::LoadGameParams const &, struct SavegameInfo const &)'::`2'::<lambda_1>::operator()(void) const'::`2'::<lambda_2>::operator()(void) const'::`2'::<lambda_1>::operator()(struct Checked<struct CheckedException>) const'::`2'::<lambda_1>, void, class GameRes const &>::vf5 rtti folded(4140)
0x8aea0 std::_Func_impl_no_alloc<class `public: __cdecl `public: __cdecl `public: bool __cdecl UI::CMenuUI::StartSavegame(struct UI::LoadGameParams const &, struct SavegameInfo const &)'::`2'::<lambda_1>::operator()(void) const'::`2'::<lambda_2>::operator()(void) const'::`2'::<lambda_1>, class std::unique_ptr<class CGame, struct std::default_delete<class CGame> >, struct Checked<struct CheckedException> >::vf5 rtti folded(4140)
0x8aea0 std::_Func_impl_no_alloc<class `public: __cdecl `public: bool __cdecl UI::CMenuUI::StartSavegame(struct UI::LoadGameParams const &, struct SavegameInfo const &)'::`2'::<lambda_1>::operator()(void) const'::`2'::<lambda_1>, class lua::Loader, char const *>::vf5 rtti folded(4140)
```

## By source file (`__FILE__`)
### gamesim.cpp
```
file game\gamesim.cpp functions=68 direct=3
0x159390 ~GameSim::Step game\gamesim.cpp direct
0x159610 sub_159610 game\gamesim.cpp inferred
0x159970 ecs::ReplicaCompVec<struct ecs::component::AudioEmitter>::vf4 game\gamesim.cpp inferred
0x159a80 ecs::ReplicaCompVec<struct ecs::component::BoundingVolume>::vf4 game\gamesim.cpp inferred
0x159bc0 ecs::ReplicaCompVec<struct ecs::component::EmissionGrid>::vf4 game\gamesim.cpp inferred
0x159cd0 ecs::ReplicaCompVec<struct ecs::component::GameScript>::vf4 game\gamesim.cpp inferred
0x159e70 ecs::ReplicaCompVec<struct ecs::component::GameSpeed>::vf4 game\gamesim.cpp inferred
0x159f80 ecs::ReplicaCompVec<struct ecs::component::GameTime>::vf4 game\gamesim.cpp inferred
0x15a150 ecs::ReplicaCompVec<struct ecs::component::Terrain>::vf4 game\gamesim.cpp inferred
0x15a380 ecs::ReplicaCompVec<struct ecs::component::World>::vf4 game\gamesim.cpp inferred
0x15a490 sub_15a490 game\gamesim.cpp inferred
0x15a590 sub_15a590 game\gamesim.cpp inferred
0x15a6d0 sub_15a6d0 game\gamesim.cpp inferred
0x15a7d0 sub_15a7d0 game\gamesim.cpp inferred
0x15a8d0 sub_15a8d0 game\gamesim.cpp inferred
0x15aa10 sub_15aa10 game\gamesim.cpp inferred
0x15ab10 sub_15ab10 game\gamesim.cpp inferred
0x15ac10 sub_15ac10 game\gamesim.cpp inferred
0x15ad10 sub_15ad10 game\gamesim.cpp inferred
0x15ad40 sub_15ad40 game\gamesim.cpp inferred
0x15add0 sub_15add0 game\gamesim.cpp inferred
0x15aeb0 sub_15aeb0 game\gamesim.cpp inferred
0x15af80 sub_15af80 game\gamesim.cpp inferred
0x15b040 sub_15b040 game\gamesim.cpp inferred
0x15b100 sub_15b100 game\gamesim.cpp inferred
0x15b1e0 sub_15b1e0 game\gamesim.cpp inferred
0x15b290 sub_15b290 game\gamesim.cpp inferred
0x15b2b0 sub_15b2b0 game\gamesim.cpp inferred
0x15b2e0 sub_15b2e0 game\gamesim.cpp inferred
0x15b310 sub_15b310 game\gamesim.cpp inferred
0x15b330 sub_15b330 game\gamesim.cpp inferred
0x15b350 sub_15b350 game\gamesim.cpp inferred
0x15b380 sub_15b380 game\gamesim.cpp inferred
0x15b3a0 sub_15b3a0 game\gamesim.cpp inferred
0x15b3c0 sub_15b3c0 game\gamesim.cpp inferred
0x15b3e0 sub_15b3e0 game\gamesim.cpp inferred
0x15b410 std::_Func_impl_no_alloc<class `public: __cdecl `void __cdecl `anonymous namespace'::Init(class GameRes const &, class GameState &, class DataLogger &, class std::vector<class ScopedConnection, class std::allocator<class ScopedConnection> > &, class std::vector<class std::function<void __cdecl (void)>, class std::allocator<class std::function<void __cdecl (void)> > > &)'::`2'::<lambda_9>::operator()(__int64) const'::`11'::<lambda_1>, void>::vf0 game\gamesim.cpp inferred
0x15b480 sub_15b480 game\gamesim.cpp inferred
0x15b4a0 sub_15b4a0 game\gamesim.cpp inferred
0x15b4b0 sub_15b4b0 game\gamesim.cpp inferred
0x15b4d0 sub_15b4d0 game\gamesim.cpp inferred
0x15b500 sub_15b500 game\gamesim.cpp inferred
0x15b510 sub_15b510 game\gamesim.cpp inferred
0x15b540 sub_15b540 game\gamesim.cpp inferred
0x15b560 sub_15b560 game\gamesim.cpp inferred
0x15b590 sub_15b590 game\gamesim.cpp inferred
0x15b5b0 std::_Func_impl_no_alloc<class `void __cdecl `anonymous namespace'::Init(class GameRes const &, class GameState &, class DataLogger &, class std::vector<class ScopedConnection, class std::allocator<class ScopedConnection> > &, class std::vector<class std::function<void __cdecl (void)>, class std::allocator<class std::function<void __cdecl (void)> > > &)'::`11'::<lambda_8>, void, __int64>::vf0 game\gamesim.cpp inferred
0x15b600 std::_Func_impl_no_alloc<class `void __cdecl `anonymous namespace'::Init(class GameRes const &, class GameState &, class DataLogger &, class std::vector<class ScopedConnection, class std::allocator<class ScopedConnection> > &, class std::vector<class std::function<void __cdecl (void)>, class std::allocator<class std::function<void __cdecl (void)> > > &)'::`2'::<lambda_9>, void, __int64>::vf0 game\gamesim.cpp inferred
0x15b660 std::_Func_impl_no_alloc<class `public: __cdecl `void __cdecl `anonymous namespace'::Init(class GameRes const &, class GameState &, class Dat
```
### gametime.cpp
```
file game\gametime.cpp functions=18 direct=7
0x2a8a50 CGameTime::vf9 game\gametime.cpp direct
0x2a8c50 CGameTime::vf4 game\gametime.cpp direct
0x2a8d90 CGameTime::vf5 game\gametime.cpp direct
0x2a8dd0 sub_2a8dd0 game\gametime.cpp direct
0x2a9100 sub_2a9100 game\gametime.cpp inferred
0x2a9130 ~CGameTime::GetDate game\gametime.cpp direct
0x2a9300 sub_2a9300 game\gametime.cpp inferred
0x2a9430 ~CGameTime::GetDayNumber game\gametime.cpp direct
0x2a9570 CGameTime::vf2 game\gametime.cpp inferred
0x2a9580 sub_2a9580 game\gametime.cpp inferred
0x2a95a0 sub_2a95a0 game\gametime.cpp inferred
0x2a95c0 sub_2a95c0 game\gametime.cpp inferred
0x2a95e0 sub_2a95e0 game\gametime.cpp inferred
0x2a9610 sub_2a9610 game\gametime.cpp inferred
0x2a9630 sub_2a9630 game\gametime.cpp inferred
0x2a9660 sub_2a9660 game\gametime.cpp inferred
0x2a9680 sub_2a9680 game\gametime.cpp inferred
0x2a96a0 ~CGameTime::OnIntervalStep game\gametime.cpp direct
```
### commandlist.cpp
```
file game\command\commandlist.cpp functions=6 direct=2
0x9d23c0 ~CommandList::Add::<lambda_7152b1228d6642b2ea6daae905652661>::operator game\command\commandlist.cpp direct
0x9d2530 sub_9d2530 game\command\commandlist.cpp inferred
0x9d28a0 boost::signals2::detail::connection_body<struct std::pair<enum boost::signals2::detail::slot_meta_group, class boost::optional<int> >, class boost::signals2::slot<void __cdecl (class std::vector<struct Command, class std::allocator<struct Command> > const &), class boost::function<void __cdecl (class std::vector<struct Command, class std::allocator<struct Command> > const &)> >, class boost::signals2::mutex>::vf0 game\command\commandlist.cpp inferred
0x9d2980 boost::signals2::signal<void __cdecl (class std::vector<struct Command, class std::allocator<struct Command> > const &), class boost::signals2::optional_last_value<void>, int, struct std::less<int>, class boost::function<void __cdecl (class std::vector<struct Command, class std::allocator<struct Command> > const &)>, class boost::function<void __cdecl (class boost::signals2::connection const &, class std::vector<struct Command, class std::allocator<struct Command> > const &)>, class boost::signals2::mutex>::vf0 game\command\commandlist.cpp inferred
0x9d29c0 sub_9d29c0 game\command\commandlist.cpp inferred
0x9d2d20 ~CommandList::Swap game\command\commandlist.cpp direct
```
### make_command.cpp
```
file game\command\make_command.cpp functions=56 direct=26
0x9e97a0 sub_9e97a0 game\command\make_command.cpp direct
0x9e9a30 sub_9e9a30 game\command\make_command.cpp inferred
0x9e9a80 sub_9e9a80 game\command\make_command.cpp direct
0x9e9b80 sub_9e9b80 game\command\make_command.cpp direct
0x9e9da0 sub_9e9da0 game\command\make_command.cpp direct
0x9e9f10 sub_9e9f10 game\command\make_command.cpp inferred
0x9ea370 sub_9ea370 game\command\make_command.cpp direct
0x9ea4b0 sub_9ea4b0 game\command\make_command.cpp direct
0x9ea5f0 sub_9ea5f0 game\command\make_command.cpp direct
0x9ea800 sub_9ea800 game\command\make_command.cpp inferred
0x9ea910 sub_9ea910 game\command\make_command.cpp inferred
0x9eaac0 sub_9eaac0 game\command\make_command.cpp inferred
0x9eaaf0 sub_9eaaf0 game\command\make_command.cpp inferred
0x9eae20 sub_9eae20 game\command\make_command.cpp inferred
0x9eae50 sub_9eae50 game\command\make_command.cpp inferred
0x9eae90 sub_9eae90 game\command\make_command.cpp inferred
0x9eaec0 sub_9eaec0 game\command\make_command.cpp inferred
0x9eaef0 sub_9eaef0 game\command\make_command.cpp inferred
0x9eaf20 sub_9eaf20 game\command\make_command.cpp direct
0x9eb030 sub_9eb030 game\command\make_command.cpp direct
0x9eb140 sub_9eb140 game\command\make_command.cpp direct
0x9eb2c0 sub_9eb2c0 game\command\make_command.cpp inferred
0x9eb350 sub_9eb350 game\command\make_command.cpp direct
0x9eb650 sub_9eb650 game\command\make_command.cpp direct
0x9ebc60 sub_9ebc60 game\command\make_command.cpp direct
0x9ebd60 sub_9ebd60 game\command\make_command.cpp direct
0x9ec120 sub_9ec120 game\command\make_command.cpp inferred
0x9ec1e0 sub_9ec1e0 game\command\make_command.cpp inferred
0x9ec2f0 sub_9ec2f0 game\command\make_command.cpp inferred
0x9ec470 sub_9ec470 game\command\make_command.cpp inferred
0x9ec790 sub_9ec790 game\command\make_command.cpp inferred
0x9ec840 sub_9ec840 game\command\make_command.cpp inferred
0x9ec8f0 sub_9ec8f0 game\command\make_command.cpp inferred
0x9ecac0 sub_9ecac0 game\command\make_command.cpp direct
0x9ecbd0 sub_9ecbd0 game\command\make_command.cpp inferred
0x9ecd90 sub_9ecd90 game\command\make_command.cpp inferred
0x9ecf50 sub_9ecf50 game\command\make_command.cpp inferred
0x9ed0a0 sub_9ed0a0 game\command\make_command.cpp direct
0x9ed190 sub_9ed190 game\command\make_command.cpp inferred
0x9ed400 sub_9ed400 game\command\make_command.cpp inferred
0x9ed5d0 sub_9ed5d0 game\command\make_command.cpp inferred
0x9ed650 sub_9ed650 game\command\make_command.cpp direct
0x9ed750 sub_9ed750 game\command\make_command.cpp inferred
0x9ed7a0 sub_9ed7a0 game\command\make_command.cpp inferred
0x9ed860 sub_9ed860 game\command\make_command.cpp inferred
0x9ed930 sub_9ed930 game\command\make_command.cpp inferred
0x9edb00 sub_9edb00 game\command\make_command.cpp inferred
0x9edbe0 sub_9edbe0 game\command\make_command.cpp direct
0x9edeb0 sub_9edeb0 game\command\make_command.cpp direct
0x9ee130 sub_9ee130 game\command\make_command.cpp direct
0x9ee1f0 ~make_cmd::SellVehicle game\command\make_command.cpp direct
0x9ee2d0 sub_9ee2d0 game\command\make_command.cpp direct
0x9ee430 sub_9ee430 game\command\make_command.cpp direct
0x9ee530 sub_9ee530 game\command\make_command.cpp direct
0x9ee600 sub_9ee600 game\command\make_command.cpp direct
0x9ee6f0 sub_9ee6f0 game\command\make_command.cpp direct
```
### savegame
```
file framework\platform\standardsavegamebackend.cpp functions=21 direct=13
file game\ui\components\savegamedialog.cpp functions=52 direct=3
file game\ui\components\savegamelist.cpp functions=1 direct=1
0x2f7cd00 sub_2f7cd00 framework\platform\standardsavegamebackend.cpp direct
0x2f7cd90 sub_2f7cd90 framework\platform\standardsavegamebackend.cpp direct
0x2f7ce20 sub_2f7ce20 framework\platform\standardsavegamebackend.cpp direct
0x2f7ceb0 sub_2f7ceb0 framework\platform\standardsavegamebackend.cpp direct
0x2f7cf40 sub_2f7cf40 framework\platform\standardsavegamebackend.cpp inferred
0x2f7cfe0 sub_2f7cfe0 framework\platform\standardsavegamebackend.cpp direct
0x2f7d070 sub_2f7d070 framework\platform\standardsavegamebackend.cpp direct
0x2f7d100 sub_2f7d100 framework\platform\standardsavegamebackend.cpp inferred
0x2f7d1a0 sub_2f7d1a0 framework\platform\standardsavegamebackend.cpp direct
0x2f7e530 platform::StandardSaveGameBackend::vf14 framework\platform\standardsavegamebackend.cpp direct
0x2f7e6f0 platform::StandardSaveGameBackend::vf16 framework\platform\standardsavegamebackend.cpp direct
0x2f7f920 platform::StandardSaveGameBackend::vf7 framework\platform\standardsavegamebackend.cpp inferred
0x2f7f990 sub_2f7f990 framework\platform\standardsavegamebackend.cpp inferred
0x2f7f9d0 platform::StandardSaveGameBackend::vf6 framework\platform\standardsavegamebackend.cpp inferred
0x2f7fa50 platform::StandardSaveGameBackend::vf15 framework\platform\standardsavegamebackend.cpp direct
0x2f7fe30 platform::StandardSaveGameBackend::vf17 framework\platform\standardsavegamebackend.cpp direct
0x2f7ff90 platform::StandardSaveGameBackend::vf18 framework\platform\standardsavegamebackend.cpp direct
0x2f80140 platform::StandardSaveGameBackend::vf4 framework\platform\standardsavegamebackend.cpp inferred
0x2f807a0 platform::StandardSaveGameBackend::vf9 framework\platform\standardsavegamebackend.cpp inferred
0x2f80c20 platform::StandardSaveGameBackend::vf11 framework\platform\standardsavegamebackend.cpp inferred
0x2f80f00 platform::StandardSaveGameBackend::vf5 framework\platform\standardsavegamebackend.cpp direct
0x6b38a0 ~UI::SavegameDialog::SavegameDialog game\ui\components\savegamedialog.cpp direct
0x6b5490 sub_6b5490 game\ui\components\savegamedialog.cpp inferred
0x6b5500 sub_6b5500 game\ui\components\savegamedialog.cpp inferred
0x6b5590 sub_6b5590 game\ui\components\savegamedialog.cpp inferred
0x6b55c0 sub_6b55c0 game\ui\components\savegamedialog.cpp inferred
0x6b5680 sub_6b5680 game\ui\components\savegamedialog.cpp inferred
0x6b56b0 sub_6b56b0 game\ui\components\savegamedialog.cpp inferred
0x6b5710 sub_6b5710 game\ui\components\savegamedialog.cpp inferred
0x6b5970 sub_6b5970 game\ui\components\savegamedialog.cpp inferred
0x6b5b70 sub_6b5b70 game\ui\components\savegamedialog.cpp inferred
0x6b5c80 sub_6b5c80 game\ui\components\savegamedialog.cpp inferred
0x6b5ff0 boost::signals2::detail::connection_body<struct std::pair<enum boost::signals2::detail::slot_meta_group, class boost::optional<int> >, class boost::signals2::slot<void __cdecl (class std::basic_string<char, struct std::char_traits<char>, class std::allocator<char> > const &, bool), class boost::function<void __cdecl (class std::basic_string<char, struct std::char_traits<char>, class std::allocator<char> > const &, bool)> >, class boost::signals2::mutex>::vf0 game\ui\components\savegamedialog.cpp inferred
0x6b60d0 boost::signals2::signal<void __cdecl (class std::basic_string<char, struct std::char_traits<char>, class std::allocator<char> > const &, bool), class boost::signals2::optional_last_value<void>, int, struct std::less<int>, class boost::function<void __cdecl (class std::basic_string<char, struct std::char_traits<char>, class std::allocator<char> > const &, bool)>, class boost::function<void __cdecl (class boost::signals2::connection const &, class std::basic_string<char, struct std::char_traits<char>, class std::allocator<char> > const &, bool)>, class boost::signals2::mutex>::vf0 game\ui\c
```

Then, for each target found: `tpfre q DB sig <name> --toml` gives its [[target]] block; `tools/re/make_profile.py` writes the whole profile (DAY_ONE.md, release-day order 1).
Verdict: GO
