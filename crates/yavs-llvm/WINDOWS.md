# yavs-llvm on Windows (x86_64-pc-windows-msvc)

How to get a pinned LLVM 21 for MSVC, how `llvm-sys` 211 finds it, what ORC does on COFF, and a
CI recipe. Each claim is marked **[verified]** (checked on 2026-10-18 against the release
artifacts, the LLVM 21.1.8 sources or the `llvm-sys` 211.1.1 sources, mostly under Wine on Linux)
or **[assumed]** (not yet run on a Windows machine). Since #127 the `llvm` CI job builds and tests
on `windows-latest`: section 5 lists what that run turned into fact; remaining **[assumed]** items
below are the ones it did not exercise.

## 1. Where LLVM 21 comes from

| Package | Contents | CRT | License | Verdict |
|---|---|---|---|---|
| Official `clang+llvm-21.1.8-x86_64-pc-windows-msvc.tar.xz` (942 MB) from <https://github.com/llvm/llvm-project/releases/tag/llvmorg-21.1.8>, signed (`.sig`) | `bin/llvm-config.exe`, 139 static `lib/LLVM*.lib`, `include/llvm-c/LLJIT.h`, `bin/LLVM-C.dll` + `lib/LLVM-C.lib` **[verified]** | static `/MT` (`CMAKE_MSVC_RUNTIME_LIBRARY MultiThreaded`, objects carry `FAILIFMISMATCH RuntimeLibrary=MT_StaticRelease`) **[verified]** | Apache-2.0 WITH LLVM-exception | **Use this.** sha256 `749d22f565fcd5718dbed06512572d0e5353b502c03fe1f7f17ee8b8aca21a47` **[verified]** |
| Official `LLVM-21.1.8-win64.exe` (NSIS installer) | toolchain for users, no static libs or `llvm-config` **[assumed]** (historically so) | | same | Not usable for building. |
| conda-forge `llvmdev 21.1.8` (`win-64/llvmdev-21.1.8-h830ff33_1.conda`, 116 MB) | `Library/bin/llvm-config.exe`, static libs **[verified]** | dynamic `/MD` (`MD_DynamicRelease`) **[verified]**, matches Rust's default | Apache-2.0 WITH LLVM-exception | Fallback. `llvm-config --system-libs` asks for `z.lib zstd.dll.lib xml2.lib`, which come from the conda packages `libzlib`, `zstd`, `libxml2`, and `zstd.dll` must be on `PATH` at run time **[verified]** for the names, **[assumed]** for the link. |
| vovkos/llvm-package-windows (GitHub releases) | `libcmt` and `msvcrt` variants | both | no license file on the repo **[verified]** | Only 21.1.1, single maintainer, unclear license: not used. |
| MSYS2 `mingw-w64-x86_64-llvm` | GNU ABI | | | Wrong ABI for an MSVC build of yavs. |

The plan's note that "prebuilt packages lack `llvm-config`" holds for the installer but not for
the official `.tar.xz`, which has shipped `llvm-config.exe` and the static libraries since the
release scripts started producing it **[verified]** for 21.1.8.

## 2. How llvm-sys finds it

- `llvm-sys` 211 **always needs `llvm-config`**. It looks in `$LLVM_SYS_211_PREFIX/bin` only when
  that variable is set, otherwise on `PATH`. Without one it sets `cfg(LLVM_SYS_NOT_FOUND)` and
  the build fails **[verified]** (`build.rs`, `locate_llvm_config`). There is no
  "without llvm-config" mode except `no-llvm-linking` + `disable-alltargets-init`, where we would
  write the link lines ourselves. Not needed with the official tarball.
- So: `LLVM_SYS_211_PREFIX=C:\llvm\clang+llvm-21.1.8-x86_64-pc-windows-msvc`.
- **Dynamic linking is refused on MSVC** ("Dynamic linking to LLVM is not currently supported on
  Windows", upstream llvm/llvm-project#109483). Our `prefer-dynamic` feature then falls back to
  static, so Windows links the static libs **[verified]** (`build.rs`, `get_link_libraries`).
  `llvm-config --shared-mode` of the official package says `static` **[verified]**.
  `LLVM-C.dll` exists but llvm-sys does not use it.
- **`libxml2s.lib` is missing.** `llvm-config --system-libs --link-static` of the official package
  prints `psapi.lib shell32.lib ole32.lib uuid.lib advapi32.lib ws2_32.lib ntdll.lib
  libxml2s.lib`, and `libxml2s.lib` is not in the archive **[verified]**, upstream
  llvm/llvm-project#172890 (open). LLVM only needs libxml2 in `LLVMWindowsManifest`, which the JIT
  never pulls in, so an **empty stub** satisfies the linker: `llvm-lib /llvmlibempty
  /out:<prefix>\lib\libxml2s.lib` (`llvm-lib.exe` is in the package) **[verified]** that the stub
  is created, **[assumed]** that the link then succeeds.
- **CRT:** the official libs are `/MT`. Rust's MSVC target defaults to the dynamic CRT, so build
  with `-C target-feature=+crt-static` (Windows only), or link errors such as LNK2038/LNK4098
  follow **[assumed]**, typical for mixing `/MT` and `/MD` objects. `cc` (used by llvm-sys for its
  small C wrapper) honours `crt-static`. The conda package is `/MD` and needs no flag.
- `llvm-config --cflags` adds no `-W` flags on MSVC and llvm-sys leaves them uncleaned there
  **[verified]**.
- The release was built with the DIA SDK and its CMake exports name
  `C:/Program Files (x86)/Microsoft Visual Studio/2019/Professional/DIA SDK/lib/amd64/diaguids.lib`
  **[verified]**. `llvm-config` does not print it, so llvm-sys never links it **[verified]**;
  only CMake consumers would trip on it.

## 3. ORC on COFF (what yavs-llvm actually runs)

- **LLJIT picks RuntimeDyld, not JITLink, for x86_64 and aarch64 COFF** in LLVM 21.1.8
  **[verified]** (`LLJIT.cpp`: `UseJITLink = !TT.isOSBinFormatCOFF()`; the default layer is
  `RTDyldObjectLinkingLayer` with a `SectionMemoryManager` per object, with
  `setOverrideObjectFlagsWithResponsibilityFlags` and `setAutoClaimResponsibilityForObjectSymbols`).
  Linux uses JITLink. The two tiers therefore link differently per OS; this is ORC, not our own
  loader, so design rule 5 holds.
- **The C API cannot select JITLink**: `OrcEE.h` only offers
  `LLVMOrcCreateRTDyldObjectLinkingLayerWithSectionMemoryManager` and
  `...WithMCJITMemoryManagerLikeCallbacks` **[verified]**. JITLink's COFF backend exists but is
  x86_64 only (`COFF.cpp`) **[verified]**. Using it needs a small C++ shim around
  `ObjectLinkingLayer`.
- **Known RuntimeDyld COFF problems:**
  - Sections must lie within 2 GB of each other and of the image base (REL32/ADDR32NB, notably
    `.pdata`). `SectionMemoryManager` does not guarantee it; failures are intermittent fatal
    errors under memory pressure: llvm/llvm-project#55386 (closed, "roll your own allocator"),
    #65641 (open) **[verified]** that the issues exist. **Mitigation if seen:** a Rust memory
    manager through `...WithMCJITMemoryManagerLikeCallbacks` that reserves one slab and hands out
    code, then read-only, then read-write slices from it.
  - Negative 32-bit addends not sign-extended in `RuntimeDyldCOFFX86_64`: #183123 (open, fires
    an assert, so it matters for assertion-enabled LLVM builds) **[verified]** the issue exists.
  - **No Win64 unwind registration** (`.pdata`/`.xdata` are not passed to `RtlAddFunctionTable`):
    #24607 and #163503 (both open) **[verified]**. Exceptions, `longjmp` and debugger stack walks
    cannot cross JIT frames. yavs does not need them: runtime errors are reported through a
    callback that returns normally, and JIT code returns a status (see `src/spike.rs`). A Rust
    panic inside a callback aborts (it is `extern "C"`), so callbacks must not panic.
- **What yavs avoids by design:** no `COFFPlatform`, no ORC runtime, no MSVC C++ runtime, no TLS
  and no static initialisers in JIT code. Those are where Windows ORC users hit most trouble
  (#213568 clang-repl, #145296 lli, #200554 CRT symbols resolved from `msvcrt.dll`, #77996 ORC
  tests) **[verified]** that the issues exist. Host functions reach JIT code only as absolute
  symbols (`Jit::new`), so nothing is resolved from the process or from DLL exports.
- **Still to check on the first Windows run [assumed]:**
  - Stack frames over 4 KB call `__chkstk`; provide it as an absolute symbol or keep large
    locals on the heap (plan rule 6 already does).
  - `memcpy`/`memset` libcalls from aggregate copies need absolute symbols on every OS.
  - Calls from JIT code to far absolute addresses go through RuntimeDyld stubs.

## 4. CI recipe (GitHub Actions, `windows-latest`)

```yaml
llvm-windows:
  runs-on: windows-latest
  env:
    LLVM_VER: 21.1.8
    LLVM_SHA256: 749d22f565fcd5718dbed06512572d0e5353b502c03fe1f7f17ee8b8aca21a47
    RUSTFLAGS: -C target-feature=+crt-static
    CARGO_BUILD_JOBS: 4
  steps:
    - uses: actions/checkout@v4
    - uses: actions/cache@v4
      id: llvm
      with:
        path: C:\llvm
        key: llvm-${{ env.LLVM_VER }}-x86_64-pc-windows-msvc
    - name: Fetch pinned LLVM
      if: steps.llvm.outputs.cache-hit != 'true'
      shell: pwsh
      run: |
        $n = "clang+llvm-$env:LLVM_VER-x86_64-pc-windows-msvc"
        curl.exe -sSfLo llvm.tar.xz "https://github.com/llvm/llvm-project/releases/download/llvmorg-$env:LLVM_VER/$n.tar.xz"
        if ((Get-FileHash llvm.tar.xz -Algorithm SHA256).Hash -ne $env:LLVM_SHA256) { throw "LLVM sha256 mismatch" }
        New-Item -ItemType Directory C:\llvm | Out-Null
        tar -xf llvm.tar.xz -C C:\llvm
        & "C:\llvm\$n\bin\llvm-lib.exe" /llvmlibempty "/out:C:\llvm\$n\lib\libxml2s.lib"
    - name: Point llvm-sys at it
      shell: pwsh
      run: echo "LLVM_SYS_211_PREFIX=C:\llvm\clang+llvm-$env:LLVM_VER-x86_64-pc-windows-msvc" >> $env:GITHUB_ENV
    - run: cargo test -p yavs-llvm --features llvm
    - run: cargo clippy -p yavs-llvm --features llvm --all-targets -- -D warnings
```

Notes: the extracted tree is 4.1 GB, mostly clang **[verified]** from the archive listing.
Keeping only `bin\llvm-config.exe`, `bin\llvm-lib.exe`, `include\` and `lib\LLVM*.lib` before
caching leaves 0.4 GB **[verified]** (sizes); `llvm-config` checks that every component library
it lists exists, so all `lib\LLVM*.lib` must stay **[verified]**. Windows `tar`
(bsdtar) reads `.tar.xz` **[assumed]** on the runner image. The `crt-static` flag in `RUSTFLAGS`
applies to the whole build; once confirmed it can move to `.cargo/config.toml` under
`[target.x86_64-pc-windows-msvc]`.

## 5. CI result (#127, `windows-latest`, LLVM 21.1.8 official tarball)

- **[verified]** `cargo clippy` and `cargo test -p yavs-llvm -p yavs --features llvm` pass,
  including `tests/jit.rs` and `tests/tier.rs`: RuntimeDyld COFF, absolute symbols, background
  compile and the tier work with no further changes.
- **[verified]** `LLVM_SYS_211_PREFIX` plus the static fallback of `prefer-dynamic` is enough;
  `-C target-feature=+crt-static` with the `/MT` libraries links; the empty `libxml2s.lib` stub
  satisfies the linker; Windows `tar` extracts the `.tar.xz`.
- **Differs from the recipe in section 4:** `llvm-sys` emits the system libs as `static=<name>`, so
  rustc itself must find `psapi.lib`, `shell32.lib`, `ole32.lib`, `uuid.lib`, `advapi32.lib`,
  `ws2_32.lib` and `ntdll.lib` on a `-L` path (error `could not find native static library
  psapi`). The job copies them from `C:\Program Files (x86)\Windows Kits\10\Lib\<ver>\um\x64`
  into `<prefix>\lib`. The cache holds a trimmed tree (`llvm-config`, `llvm-lib`, `include`,
  `lib\LLVM*.lib`, stub, SDK libs).
- Not run on Windows: the `--tier=llvm` difftest slice (the harness uses a POSIX shell template).
