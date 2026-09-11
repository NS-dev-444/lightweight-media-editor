# Dependency and License Audit

**Project:** Lightweight cross-platform video + audio editor
**Document status:** Phase 0 deliverable 4 of 5
**Date:** 2026-09-09
**Nothing in this document is legal advice.** Items marked `[COUNSEL]` require review by qualified counsel before first public distribution (spec §46 Rule 3, §10.7).

---

> ### Scope note — 2026-09-11 (corrected)
>
> **A copy IS going to another person.** The app is being built for someone else
> to use on their own Windows PC, which means the obligations in this document
> are **live, not dormant** — an earlier version of this note said otherwise and
> was wrong. LGPL's notice and relinking duties and the attribution in §7b all
> trigger the moment a copy reaches somebody else, whether or not money changes
> hands.
>
> **Proportionately:** one copy to one known person is the lowest-stakes form of
> distribution there is, and the parts that are expensive to retrofit are already
> done — the FFmpeg build is LGPL-clean and gated, the attributions are generated
> and verified, and the libraries ship as replaceable dylibs/DLLs. The AVC/HEVC
> royalties in §6 attach per copy; at this volume they are far below any
> threshold a pool would act on, but the obligation exists rather than not.
>
> O-5, O-6 and O-7 are therefore **open again**, at low urgency.
>
> **The audit stands anyway, and the compliance work stays in place.** Not out of
> caution for its own sake — because the expensive half of compliance is the half
> that must be designed in. An LGPL-clean FFmpeg cannot be retrofitted onto a
> build that linked libx264; attributions cannot be reconstructed for a
> dependency list nobody kept. Both are done, gated, and cost nothing to keep.
>
> If distribution is ever considered, this document is current and the open
> items (O-5, O-6, O-7) resume exactly as written.

## 1. Audit status legend

| Status | Meaning |
|---|---|
| ✅ **CLEARED** | Licence text read during Phase 0; obligations understood and listed. |
| 🔍 **VERIFY** | Licence believed to be as stated but the licence text has **not** been read in Phase 0. Must be verified before the dependency is added to the build. |
| ⛔ **DENIED** | Must not be used, at any point, in any configuration. |
| ⚖️ **COUNSEL** | Legal question, not an engineering question. |

**No dependency may be added to the build while it is marked 🔍.** The process is: read the licence, record the obligations here, change the status, then add it.

---

## 2. The two licence layers

This project has two independent legal layers and clearing one does not clear the other.

| Layer | What it covers | Governed by | Status |
|---|---|---|---|
| **Copyright** | Our right to use and distribute the *code* of our dependencies | LGPL/MIT/Apache/BSD etc. | Manageable with the discipline in this document |
| **Patents** | Our right to implement *codecs* — AVC, HEVC, AAC — in a commercial product | Via LA, Access Advance, and unpooled holders | ⚖️ **Open — see §6** |

Choosing an LGPL FFmpeg build addresses the first layer only. It has no effect whatsoever on the second.

---

## 3. FFmpeg — the controlling dependency

### 3.1 Evidence: what is on the development machine today

```
ffmpeg version 9.0.1
configuration: --prefix=/opt/homebrew/Cellar/ffmpeg/9.0.1 --enable-shared
  --enable-pthreads --enable-version3 --enable-ffplay --enable-gpl
  --enable-libsvtav1 --enable-libopus --enable-libx264 --enable-libmp3lame
  --enable-libdav1d --enable-libvmaf --enable-libvpx --enable-libx265
  --enable-openssl --enable-videotoolbox --enable-audiotoolbox --enable-neon
```

⛔ **This build must never be linked into the product.** It carries `--enable-gpl`, `--enable-version3`, `libx264` and `libx265`. It is fine — and useful — for producing test media and for VMAF measurement in Phase 1, because those artefacts are not distributed. It is on the default `PATH` of the development machine, which makes accidental linkage the most likely licensing failure on this project (`RISK_REGISTER.md` R-05).

### 3.2 The build we ship

✅ **CLEARED** — obligations read from [ffmpeg.org/legal.html](https://ffmpeg.org/legal.html).

**Licence:** LGPL v2.1 or later, *provided* no GPL or nonfree component is enabled. FFmpeg's own words: "FFmpeg incorporates several optional parts and optimizations that are covered by the GNU General Public License (GPL) version 2 or later. If those parts get used the GPL applies to all of FFmpeg."

**Configure policy:**

```sh
# REQUIRED — absent from every build, forever
#   --enable-gpl        → makes the whole thing GPL
#   --enable-nonfree    → makes the result NON-REDISTRIBUTABLE
#   --enable-version3   → pulls in LGPLv3 terms; avoid unless a cleared component needs it

./configure \
  --prefix="$OUT" \
  --enable-shared --disable-static \
  --disable-programs --disable-doc \
  --disable-encoder=libx264,libx265 \
  --enable-videotoolbox \        # macOS
  --enable-audiotoolbox \        # macOS
  --enable-nvenc --enable-d3d11va --enable-dxva2 \   # Windows
  --enable-libopus --enable-libdav1d --enable-libsvtav1
  # NOT: --enable-gpl, --enable-nonfree, --enable-version3,
  #      --enable-libx264, --enable-libx265, --enable-libfdk-aac
```

*(Illustrative, not final. The exact configure line is a Phase 1 deliverable and must be checked in and reproducible.)*

**Distribution obligations we must meet:**

| Obligation | How we satisfy it | Verified in |
|---|---|---|
| Link dynamically | Ship `.dylib`/`.dll` in the bundle; no static linking | CI gate |
| Ship FFmpeg source, modified or not | Publish our exact source tarball + build script on the download server | Release checklist |
| Same server as the binaries | Download page links both | Release checklist |
| Attribution: "uses code of FFmpeg licensed under the LGPLv2.1" | About box + EULA + third-party notices screen | Release checklist |
| Mention FFmpeg in about box and EULA | Same | Release checklist |
| Do not rename the libraries to obscure them | Ship as `libavcodec.*` etc., unrenamed | CI gate |
| If we patch FFmpeg, publish the diff | Patches live in-repo under `third_party/ffmpeg/patches/` | Release checklist |

### 3.3 FFmpeg components — allow / deny

| Component | Licence | Status | Notes |
|---|---|---|---|
| `libavcodec`, `libavformat`, `libavfilter`, `libswscale`, `libswresample`, `libavutil` | LGPL 2.1+ | ✅ CLEARED | The core. LGPL only if nothing GPL is enabled. |
| `libx264` | GPL | ⛔ DENIED | Would make the whole product GPL. |
| `libx265` | GPL | ⛔ DENIED | Same. |
| `libfdk-aac` | nonfree | ⛔ DENIED | `--enable-nonfree` makes the binary non-redistributable. |
| Anything requiring `--enable-nonfree` | nonfree | ⛔ DENIED | Non-redistributable, full stop. |
| `videotoolbox` / `audiotoolbox` (macOS) | Apple system frameworks | ✅ CLEARED | Our primary macOS encode path. |
| `nvenc` / `nvdec` | headers MIT | ✅ **CLEARED (copyright)** | Verified 2026-09-10 against our pinned FFmpeg 8.1.2 — see §3.3a. `nvenc_deps="ffnvcodec"` (MIT headers, build-time only) + `LoadLibrary` at runtime: the encoder is in the **user's driver**, nothing of NVIDIA's is redistributed. Structurally identical to VideoToolbox. |
| `amf` (AMD) | headers MIT | ✅ **CLEARED (copyright)** | Same shape: `amf_deps_any="libdl LoadLibrary"` — runtime load from the user's driver, nothing redistributed. |
| `qsv` (Intel, via oneVPL/libvpl) | 🔍 VERIFY | 🔍 | **The one that is genuinely different.** `qsv_deps="libmfx"` — it *links* a library rather than loading the driver's. That is a real redistribution question and it is still open. |
| `d3d11va` / `dxva2` | Windows system APIs | ✅ **CLEARED** | Neither in the GPL nor the nonfree list in our pinned version. Decode only. |
| `libdav1d` (AV1 decode) | BSD-2-Clause | 🔍 VERIFY | Expected permissive. |
| `libsvtav1` (AV1 encode) | BSD-3-Clause + AOMedia Patent Licence | 🔍 VERIFY | Expected permissive; note patent grant terms. |
| `libopus` | BSD-3-Clause | 🔍 VERIFY | Expected permissive. |
| Native FFmpeg AAC encoder | LGPL (code) | ✅ CLEARED (copyright) / ⚖️ COUNSEL (patents) | Copyright fine; AAC patents are a separate matter (§6). |
| `aac_at` (AudioToolbox AAC) | Apple system framework | ✅ CLEARED | Confirmed present in our build. Apple's AAC encoder is generally higher quality than FFmpeg's native one — prefer it on macOS. |

### 3.3a How the Windows encoders were verified — 2026-09-10

§3.3 asked for re-verification "against our pinned FFmpeg version". Done, by
reading FFmpeg 8.1.2's own `configure` rather than trusting a report:

```
                gpl_list  nonfree_list
  nvenc            0          0
  nvdec            0          0
  qsv              0          0
  amf              0          0
  d3d11va          0          0
  dxva2            0          0
  videotoolbox     0          0
  libx264          1          0     <- control
  libfdk_aac       0          1     <- control
```

The last two are controls: a method that finds nothing proves nothing, so it had
to place the two components we already know are GPL and nonfree.

**Three questions were tangled under one 🔍, and they have different answers:**

1. **Does enabling them make the build GPL or nonfree?** No — verified above.
2. **Do we redistribute anything of the vendor's?** No for **nvenc** and **amf**:
   MIT headers at build time, and the encoder itself is loaded from the user's
   own graphics driver at runtime. Yes, possibly, for **qsv**, which links
   `libmfx` — that one stays open.
3. **AVC/HEVC patents?** This is **the same question as macOS**, which already
   ships VideoToolbox with O-5 unresolved. The argument is identical on both:
   the encoder is the OS's or the driver's, not in our binary. It is one
   question, not two — and per the scope note it is dormant while nothing is
   distributed.

**Consequence:** Windows video export via nvenc/amf is a **build task**, not a
licensing blocker. It needs `nv-codec-headers` added to the Windows FFmpeg
build. An Intel-only machine would have no hardware encoder until qsv's
redistribution question is answered, and that gap should be stated rather than
papered over.

### 3.4 Verified in Phase 1: the MP3 encoding gap

A minimal LGPL FFmpeg build **cannot encode MP3 at all**. Measured against our own build:

- `libmp3lame` — absent unless explicitly enabled and built as a dependency
- `mp3_at` (AudioToolbox MP3 encoder) — **absent on modern macOS**, so there is no system fallback

§4 requires WAV→MP3, FLAC→MP3, M4A→MP3 and OGG→MP3. Those conversions are impossible without adding LAME.

**Resolution:** LAME is built from pinned, checksum-verified source by `third_party/ffmpeg/build.sh` and linked dynamically. Because LAME is LGPL rather than GPL, this satisfies §4 without compromising the licensing posture. MP3's patents have expired, so there is no patent layer here either.

This is the pattern every future external library follows (Opus, dav1d, SVT-AV1): pin, verify, read the licence, record it here, then enable.
| `libmp3lame` (LAME 3.100) | LGPL v2 **or later** | ✅ **CLEARED** | Verified in Phase 1 by reading `COPYING` ("GNU Library General Public License, Version 2") and the source headers in `libmp3lame/lame.c` ("either version 2 of the License, or (at your option) any later version"). The upgrade clause makes it compatible with FFmpeg's LGPL v2.1+. **It does not make the build GPL.** Required — see the MP3 finding below. Same LGPL obligations as FFmpeg: dynamic linking, source published, attribution. Our build applies one patch (removing the stale `lame_init_old` export) which must be published as a diff. |

---

## 4. Rust crate dependencies (proposed)

None of these are committed yet. All are 🔍 until their licence text is read and recorded.

| Crate | Purpose | Licence (believed) | Status |
|---|---|---|---|
| `rusty_ffmpeg` *or* `ffmpeg-next` | FFmpeg FFI | MIT / WTFPL respectively | 🔍 |
| `cbindgen` | C header generation (build-time only) | MPL-2.0 | 🔍 |
| `serde`, `serde_json` | Project serialisation | MIT/Apache-2.0 | 🔍 |
| `cpal` | Audio device I/O | Apache-2.0 | 🔍 |
| `rubato` | Resampling (only if not using swresample) | MIT | 🔍 |
| `blake3` *or* `xxhash` | Content hashing for cache keys | CC0/Apache-2.0 / BSD | 🔍 |
| `parking_lot` | Synchronisation | MIT/Apache-2.0 | 🔍 |
| `crossbeam` | Channels for the job scheduler | MIT/Apache-2.0 | 🔍 |
| `thiserror` | Error types | MIT/Apache-2.0 | 🔍 |
| `tracing` | Structured logging | MIT | 🔍 |

**Explicitly not adopted:** `symphonia`. It is a good crate, but AD-7 uses one decoding stack (FFmpeg) rather than two. Adding it would mean two implementations of audio decoding with different bugs and different format quirks.

**Build-time-only dependencies** (compilers, codegen, test tooling) still need their licences recorded, but they do not create distribution obligations because they are not shipped. That distinction must be recorded per dependency so nobody has to re-derive it.

---

## 5. Platform and application dependencies

| Dependency | Platform | Licence | Status | Notes |
|---|---|---|---|---|
| Swift / SwiftUI / AppKit / Metal / VideoToolbox / CoreAudio | macOS | Apple SDK terms | ✅ CLEARED | Standard platform frameworks. |
| .NET runtime / Windows App SDK / WinUI 3 | Windows | MIT + Microsoft terms | 🔍 | Verify redistribution terms for the runtime and the App SDK. |
| Direct3D / DXGI | Windows | Windows SDK | 🔍 | Expected clean. |
| Sparkle (macOS updates) | macOS | permissive (believed MIT-style with attribution) | 🔍 | Verify before adopting. |
| Windows updater | Windows | — | — | **DEFERRED** — see `ARCHITECTURE_DECISION.md` AD-8. |
| Rubber Band Library | both | GPL + paid commercial | ⛔ DENIED for V1 | Pitch shifting is cut from V1 (AD-7). If ever needed, a commercial licence must be purchased. |
| SoundTouch | both | LGPL 2.1 (believed) | 🔍 | Only relevant if pitch shifting returns in V1.1. |
| `libvmaf` | dev only | BSD+patent | 🔍 | **Measurement tooling only — never shipped.** |
| Sintel (frames) | dev only | **CC BY 3.0** | ✅ CLEARED | Test corpus. README read: "Creative Commons Attribution 3.0 license", attribution to the Blender Foundation / durian.blender.org. Measurement only — never shipped, never redistributed. |
| Tears of Steel (frames) | dev only | **CC BY 3.0** | ✅ CLEARED | Test corpus, Blender Foundation. Measurement only. |
| `whisper.cpp` (ggml) | both | **MIT** | ✅ **CLEARED** | LICENSE read: "MIT License / Copyright (c) 2023-2026 The ggml authors". Bundleable and redistributable commercially with attribution. Approved by O-16. **Now built and shipped — see §6b for the as-built audit, versions and pins.** |
| OpenAI Whisper — **model weights** | both | **MIT** | ✅ **CLEARED** | LICENSE read: "MIT License / Copyright (c) 2022 OpenAI". OpenAI releases code **and weights** under MIT, so the weights may be redistributed inside the product. Unusual and worth stating explicitly — most model weights are not MIT. |

### 5.1 Captions: no royalty, no service, no subscription

Local transcription is the rare feature that is clean on **every** layer this document tracks:

- **Copyright:** MIT on both the runtime and the weights. Attribution only.
- **Patents:** no pool asserts against speech recognition the way Via LA and Access Advance assert against AVC/HEVC.
- **Recurring cost:** none. Nothing is called at runtime; no API key, no account, no per-minute fee.

Compare with the codec layer (§6), where the product carries genuine open exposure. Captions add a headline feature and no new legal risk.

**Remaining decision (O-18, product):** model size versus §47's "lightweight". Bundling a multi-gigabyte model contradicts the product's central claim. Ship a small model, fetch larger ones on demand, or fetch on first use — a bundle-size/accuracy trade, needed before Phase 3.

---

## 6. ⚖️ Codec patent exposure

**This is the layer that is not solved by anything in §3–5.**

| Codec | Pool / administrator | Our use | Exposure |
|---|---|---|---|
| **H.264 / AVC** | Via LA | Decode (software + hardware); encode (hardware only) | ⚖️ Open. Via LA apportions royalties across the value chain with annual caps and thresholds. In 2026 it restructured *streaming* fees into tiers up to $4.5M/yr for the largest platforms — aimed at streaming services, not desktop software, but evidence of active repricing. |
| **HEVC / H.265** | Access Advance (which has now also acquired administration of Via LA's HEVC/VVC pools, as "VCL Advance") | Decode (software + hardware); encode (hardware only) | ⚖️ Open, and the highest-exposure item. Access Advance licenses HEVC decoders *and* encoders installed in devices **or software**, seeking one royalty per copy at first sale. Desktop software is explicitly in scope. |
| **AAC** | Via LA | Decode and encode | ⚖️ Open. FFmpeg's native AAC encoder is copyright-clean; the patents are separate. |
| **MP3** | expired | Encode/decode | ✅ Patents expired. |
| **AV1** | AOMedia (royalty-free); Sisvel pool (contested) | Optional encode/decode | ⚠️ Low. AOMedia Patent License 1.0 is royalty-free. Sisvel's pool covers non-AOMedia holders and to date licenses hardware; it has explicitly kept software options open. No successful suit against a major implementer is reported; Netflix and Google have declined to pay. |
| **VP9 / Opus / FLAC / WAV/PCM** | — | Decode; Opus/FLAC/WAV encode | ✅ Considered clear. |

### The open question that must be answered before distribution

> Does encoding via an OS/hardware-provided encoder (VideoToolbox, Media Foundation, NVENC) discharge the *application developer's* AVC/HEVC royalty obligation, on the basis that the platform or hardware vendor has already paid for that unit?

Phase 0 research found **no authoritative statement either way**. The proposition is widely relied upon in industry practice; it was not verifiable from primary sources during this phase, and §46 Rule 3 forbids assuming a dependency is safe to distribute commercially. ⚖️ **COUNSEL — required before first public distribution.**

### Documented fallback: Cisco OpenH264

If a *software* H.264 encoder becomes necessary, Cisco pays Via LA royalties for binaries **Cisco itself builds and distributes**, subject to conditions:

- the binary must be **downloaded separately to the end user's device** and **not pre-bundled** into third-party software;
- the end user must be able to control its use;
- the application must display "OpenH264 Video Codec provided by Cisco Systems, Inc."

These conditions shape installer and first-run design, so if this path is ever taken it must be decided before the installer is built, not after. OpenH264's quality and profile support are also limited relative to hardware or `libx264`.

---

## 6b. Transcription — whisper.cpp and the Whisper model

✅ **CLEARED.** Added 2026-09-10 when captions were built (O-16, O-18).

This is the only dependency in the product whose **data** is redistributed as
well as its code, so it is audited as two things.

| Component | Version | Licence | Verified how |
|---|---|---|---|
| `whisper.cpp` | v1.9.3, commit `371b5a75` | **MIT**, "Copyright (c) 2023-2026 The ggml authors" | Read from `LICENSE` in the tree we actually build; `third_party/whisper/build.sh` **fails the build** if that file stops saying "MIT License". |
| `ggml` | vendored in the above | MIT, same file | As above. |
| Whisper model weights (`ggml-base-q5_1.bin`) | — | **MIT**, "Copyright (c) 2022 OpenAI" | OpenAI released Whisper's code *and* weights under MIT. |

**Weights being MIT is the unusual part and the load-bearing one.** Most speech
models are released under terms that forbid redistribution, which would have
meant a first-run download and a licence click. MIT means the model ships inside
the bundle like any other resource.

**No royalty, no service, no account, no network.** There is no runtime
component to license and no per-copy cost — unlike AVC/HEVC in §6, which is the
comparison worth making explicitly.

### Attribution obligations

✅ **DONE, 2026-09-10.** Both notices ship, alongside FFmpeg's and LAME's LGPL
texts and every Rust crate's licence — see §7b.

OpenAI's licence text was fetched from `github.com/openai/whisper` and checked
against the claim above: "MIT License / Copyright (c) 2022 OpenAI". It is
committed at `third_party/licences/whisper-model-openai.txt`, because unlike the
others it is not produced by a build we run.

### Why this is not a patent question

Speech recognition is not a pooled-patent field in the way AVC and HEVC are
(§6). There is no equivalent of Via LA or Access Advance to license from, and
the model is a set of weights rather than an implementation of a standard.

### Supply chain

The model is fetched by `tools/fetch_models.sh` and **pinned by SHA256**
(`422f1ae4…a8898`), for the same reason the FFmpeg tarball is: a model is
downloaded over the network and then executed against the user's private media.
The build fails on a mismatch rather than proceeding with whatever the CDN
served.

The whisper.cpp pin is by **commit**, not tag — and the first attempt recorded
the annotated tag *object's* SHA instead of the commit it points to, which the
build script's own check caught immediately. That is the check working.

---

## 7b. Attribution — how it is discharged

✅ **DONE, 2026-09-10.**

Both licences we ship under require notices to travel with the software:

- **MIT** — the copyright and permission notices must accompany all copies.
- **LGPL §6** — the licence text, prominent notice that the libraries are used,
  and the user's ability to relink with their own build.

### It is generated, never maintained

`tools/gather_licences.py` builds the acknowledgements from **the artefacts that
actually ship**: the dylibs present in the bundle, the crates `cargo metadata`
reports in the normal dependency closure, and the licence texts carried in the
build prefixes.

A hand-kept NOTICE file is wrong the day after somebody adds a dependency, and
nobody notices because nothing checks it. This cannot drift — add a crate and it
appears; remove a library and it goes.

**Build-only dependencies are deliberately excluded.** cbindgen runs at build
time and none of its code reaches a user, so attributing it would be padding
rather than compliance.

### Two gates, because one is not enough

| Layer | Behaviour |
|---|---|
| `app/macos/build_app.sh` | **The build fails** if any shipping component has no licence text. Verified by hiding one: exit 1, "something ships unattributed". |
| `tools/check.sh` | Asserts the file is in the bundle and **names every dylib, the model, and every crate**. Verified to fail by the same means. |

The second exists for the stale case: a bundle built before a dependency was
added and then checked without rebuilding.

### What ships, and under what

| Component | Licence |
|---|---|
| FFmpeg 8.1.2 (7 libraries) | LGPL v2.1+ |
| LAME 3.100 | LGPL v2.1+ |
| whisper.cpp v1.9.3 + ggml | MIT |
| Whisper model weights | MIT (OpenAI) |
| 12 Rust crates | MIT / Apache-2.0 / Unlicense; `unicode-ident` also Unicode-3.0 |

The LGPL text appears once and is referenced by both LGPL components, which is
what the licence requires — not one copy per library.

### The relinking right, stated explicitly

LGPL §6 is satisfied by shipping FFmpeg and LAME as **separate dynamic
libraries** in `Contents/Frameworks`, which a user may replace with their own
build. The acknowledgements say so in those words, name the exact versions, and
point at both the upstream source and `third_party/ffmpeg/build.sh` — noting
that it applies no patches.

---

## 7. CI enforcement

Licence discipline that depends on engineers remembering it will fail. The following are **build-blocking gates**, to be implemented in Phase 1:

1. **FFmpeg configuration gate.** Read `avutil`'s configuration string from the *linked* library at build time. Fail on `--enable-gpl`, `--enable-nonfree`, `--enable-version3`, or any denied component name.
2. **FFmpeg provenance gate.** Fail if the linked FFmpeg is not the CI-produced artifact — no Homebrew, no system, no `PATH` resolution, in any configuration including local development.
3. **Dynamic-linkage gate.** Assert on both platforms that FFmpeg is dynamically linked and that the libraries carry their canonical names.
4. **Crate licence gate.** `cargo-deny` (or equivalent) with an explicit allow-list. Any new crate whose licence is not on the list fails the build.
5. **Notices freshness gate.** The generated third-party notices file must match the current dependency set, or the build fails.
6. **macOS signing gate.** Every Mach-O in the bundle — app, workers, every dylib — signed with our Team ID and hardened. Fail if `disable-library-validation` appears in any entitlements file.

Gate 6's last clause matters: because we build the FFmpeg dylibs ourselves and sign them with our own Team ID, Library Validation is satisfied and that entitlement is never needed. Its presence would indicate a wrong turn.

---

## 8. Release checklist (licence portion)

- [x] **Attribution for everything that ships** — generated, gated, and shown
      in the app under Editor ▸ Acknowledgements (§7b).

Before any public build ships:

- [ ] FFmpeg source tarball + build script published on the download server
- [ ] Any FFmpeg patches published as diffs
- [ ] Third-party notices file complete and current
- [ ] About box states FFmpeg usage and LGPL v2.1
- [ ] EULA mentions FFmpeg
- [ ] Every 🔍 in this document resolved to ✅ or ⛔
- [ ] `[COUNSEL]` items in §6 answered in writing
- [ ] AVC/HEVC royalty position documented and budgeted
- [ ] macOS: notarized, stapled, hardened, every binary signed with one Team ID
- [ ] Windows: installer signed; certificate valid well past the release date
- [ ] No `--enable-gpl` / `--enable-nonfree` in the shipped FFmpeg (verified from the *shipped* binary, not the build script)

---

## Sources

- [FFmpeg — License and Legal Considerations](https://ffmpeg.org/legal.html)
- [FFmpeg — LICENSE.md](https://github.com/FFmpeg/FFmpeg/blob/master/LICENSE.md)
- [Via LA — AVC/H.264 licence fees](https://www.via-la.com/licensing-programs/avc-h-264/)
- [Access Advance — HEVC Advance](https://accessadvance.com/licensing-programs/hevc-advance/) · [What we license](https://accessadvance.com/topic-what-do-we-license/)
- [Access Advance / Via LA HEVC-VVC acquisition](https://www.accessnewswire.com/newsroom/en/electronics-and-engineering/access-advance-and-via-licensing-alliance-announce-hevc%2Fvvc-program-acq-1117638)
- [OpenH264 — binary licence](https://www.openh264.org/BINARY_LICENSE.txt) · [FAQ](https://www.openh264.org/faq.html)
- [Sisvel — AV1 licensing programme](https://www.sisvel.com/licensing-programmes/audio-and-video-coding-decoding/video-coding-platform-av1/)
- [Apple Developer Forums — disable-library-validation](https://developer.apple.com/forums/thread/799497)
