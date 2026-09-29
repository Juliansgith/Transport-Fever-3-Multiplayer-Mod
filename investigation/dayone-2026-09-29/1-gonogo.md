# Go or no-go: anti-tamper

Made by tools/dayone/dayone.py on 2026-09-29 18:34.

## `TransportFever3.exe`
- machine 0x8664, PE timestamp 1790353381, image base 0x140000000
  - `.text` 57,028,608 bytes, entropy 6.49 (code)
  - `.rdata` 6,616,064 bytes, entropy 5.91
  - `.data` 3,700,224 bytes, entropy 5.07
  - `.pdata` 1,672,192 bytes, entropy 6.96
  - `_RDATA` 1,024 bytes, entropy 1.73
  - `.rsrc` 271,360 bytes, entropy 1.11
  - `.reloc` 177,152 bytes, entropy 5.51
  - `.bind` 234,056 bytes, entropy 7.96 (code)
- imports: SHELL32.dll, OPENGL32.dll, PDXSDK.dll, alut.dll, OpenAL32.dll, SDL2.dll, ntdll.dll, ADVAPI32.dll, icuin61.dll, icuuc61.dll, WS2_32.dll, KERNEL32.dll, MSVCP140.dll, VCRUNTIME140.dll, VCRUNTIME140_1.dll, api-ms-win-crt-heap-l1-1-0.dll, api-ms-win-crt-runtime-l1-1-0.dll, api-ms-win-crt-math-l1-1-0.dll, api-ms-win-crt-stdio-l1-1-0.dll, api-ms-win-crt-filesystem-l1-1-0.dll, api-ms-win-crt-utility-l1-1-0.dll, api-ms-win-crt-time-l1-1-0.dll, api-ms-win-crt-locale-l1-1-0.dll, api-ms-win-crt-convert-l1-1-0.dll, api-ms-win-crt-string-l1-1-0.dll, api-ms-win-crt-environment-l1-1-0.dll, PSAPI.DLL, USER32.dll, steam_api64.dll, WINHTTP.dll, ole32.dll
- SteamStub (.bind): as TPF2 had; the hook loads into the unpacked process
- 3 TLS callback(s) (TPF2 had 3): if the hook fails to load, try the loader-first fallback (DAY_ONE.md section 5)
Verdict for TransportFever3.exe: GO

## The launcher's start (by hand; nothing here starts the game)
1. Steam running, start the game from the TPF3-MP launcher (Start Transport Fever 3) in a room.
2. While it runs: `python tools/dayone/dayone.py check-launch`.
3. In the game: signed in (your Steam name shows), the Workshop / Mod Hub works, and it did not close and restart.

Verdict: GO
