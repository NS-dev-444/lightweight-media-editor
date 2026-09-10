# Lightweight Cross-Platform Video + Audio Editor
## Master Product, Architecture, and Development Specification

**Working name:** TBD  
**Project type:** Native desktop media editor  
**Platforms:** macOS + Windows  
**Primary goal:** A fast, lightweight, privacy-first video/audio editor and media converter that focuses on must-have features instead of competing with professional suites on feature count.

---

# 1. PRODUCT VISION

Build a desktop application for people who want to quickly:

- Edit video
- Edit music/audio
- Convert video
- Convert audio
- Extract audio from video
- Combine media
- Resize/compress media
- Export high-quality 4K, 2K, and 1080p content

The application must feel fast, uncluttered, and understandable.

## Product philosophy

> Open → edit → convert → export.

Do not turn this into another Premiere Pro, Final Cut Pro, or DaVinci Resolve.

The product wins through:

1. Simplicity
2. Speed
3. Lightweight resource usage
4. Native operating-system integration
5. Excellent 4K performance
6. Strong audio support
7. Reliable conversion
8. Local/offline operation by default

---

# 2. CORE PRODUCT PILLARS

## Pillar A — Video Editor

Must support:

- MP4
- MOV
- MKV
- WebM
- Common H.264/H.265/HEVC media
- Common audio codecs used inside video containers

Core editing:

- Import
- Drag/drop
- Trim
- Split
- Cut
- Delete
- Reorder clips
- Crop
- Resize
- Rotate
- Flip
- Speed adjustment
- Freeze frame
- Basic transitions
- Text overlays
- Image/logo overlays
- Volume adjustment
- Fade in/out
- Mute
- Detach/extract audio
- Replace audio
- Multiple video tracks where practical
- Multiple audio tracks
- Undo/redo

Do NOT initially build advanced compositing, 3D editing, professional color grading, motion tracking, or multicamera workflows.

---

# 3. AUDIO EDITOR

Audio must be a first-class feature, not merely audio attached to video.

Support:

- MP3
- WAV
- FLAC
- M4A/AAC
- OGG
- Common PCM formats

Core operations:

- Waveform display
- Play/pause
- Scrubbing
- Trim
- Split
- Cut
- Delete
- Join/merge
- Fade in
- Fade out
- Volume
- Normalize
- Silence removal
- Basic EQ
- Basic gain control
- Speed adjustment
- Pitch adjustment where technically reliable
- Convert formats
- Export formats
- Extract audio from video

The audio editor should work independently of the video editor.

---

# 4. MEDIA CONVERTER

Create a dedicated conversion workflow.

## Video conversion

Examples:

- MOV → MP4
- MKV → MP4
- WebM → MP4
- MP4 → WebM
- MP4 → MOV

## Audio conversion

Examples:

- WAV → MP3
- FLAC → MP3
- M4A → MP3
- MP3 → WAV
- OGG → MP3
- M4A → WAV

## Required converter features

- Drag/drop
- Multiple files
- Batch conversion
- Output folder selection
- Presets
- Progress
- Cancel
- Retry
- Error reporting
- Preserve metadata when practical
- Optional overwrite protection

---

# 5. RESOLUTION SUPPORT

The application must fully support:

| Resolution | Dimensions |
|---|---:|
| 4K UHD | 3840×2160 |
| 2K/QHD | 2560×1440 |
| 1080p/FHD | 1920×1080 |
| 720p | 1280×720 |
| SD | 854×480 |

Also support custom dimensions.

## Export presets

Provide simple presets:

- 4K — Best Quality
- 4K — Smaller File
- 2K — Best Quality
- 1080p — Best Quality
- 1080p — Smaller File
- 720p — Smaller File
- Custom

The user should not need to understand codecs or bitrate to produce a good result.

Advanced settings can expose:

- Codec
- Bitrate
- Frame rate
- GOP settings where appropriate
- Audio codec
- Audio bitrate
- Sample rate
- Container

---

# 6. 4K PERFORMANCE REQUIREMENT

4K is a core requirement.

The application must NOT simply decode full-resolution source video into huge CPU-memory buffers for every UI operation.

Use an optimized preview architecture.

Concept:

```text
Original 4K Media
       |
       v
Media Decoder
       |
       +------> Proxy / Preview Representation
       |
       v
Timeline
       |
       v
GPU Preview
       |
       v
Final Render
       |
       v
Original Source Media
       |
       v
4K / 2K / 1080p Export
```

Preview quality can dynamically change depending on:

- Hardware
- Resolution
- FPS
- Number of effects
- Number of tracks
- Available memory
- Current playback performance

Final export must use original-quality source media whenever possible.

---

# 7. NATIVE MACOS STRATEGY

The macOS application should be genuinely native.

Preferred stack:

- Swift
- SwiftUI for application UI where appropriate
- AppKit where SwiftUI does not provide sufficient control/performance
- Metal for GPU rendering
- AVFoundation where useful
- Core Video
- VideoToolbox
- Native macOS file/document APIs

Apple's VideoToolbox provides low-level hardware-accelerated video encoding and decoding, including compression/decompression sessions and pixel-buffer processing. Use it as a major part of the macOS media path rather than treating macOS as a generic Windows-like platform.

Reference:
Apple VideoToolbox documentation:
https://developer.apple.com/documentation/videotoolbox

## macOS goals

- Native menus
- Native keyboard shortcuts
- Native drag/drop
- Retina/HiDPI support
- Native full-screen
- Native window behavior
- Apple Silicon optimization
- Intel Mac compatibility only if deliberately supported
- Native save/open dialogs
- macOS sandbox compatibility
- Proper signing and notarization
- Efficient memory use

The Mac version should feel like a Mac application, not a web application wrapped in a desktop window.

---

# 8. NATIVE WINDOWS STRATEGY

Windows should also have a native desktop experience.

Potential UI technologies:

- WinUI 3 / C#
- C++ where performance-sensitive integration requires it

The exact Windows UI technology must be validated during Phase 0 before implementation.

Windows media acceleration should use appropriate native/hardware paths where available.

Potential hardware paths include:

- Direct3D/D3D12 video
- Media Foundation
- NVIDIA acceleration
- Intel acceleration
- AMD acceleration

Microsoft documents D3D12 video encoding as a low-level hardware-accelerated path that can be used by higher-level media APIs.

Reference:
Microsoft D3D12 Video Encoding documentation:
https://learn.microsoft.com/en-us/windows-hardware/drivers/display/video-encoding-d3d12

---

# 9. SHARED CORE ARCHITECTURE

Do NOT duplicate the entire media engine for each platform.

Preferred architecture:

```text
                 APPLICATION
                     |
          +----------+----------+
          |                     |
       macOS                  Windows
          |                     |
     Swift/SwiftUI          Native Windows UI
          |                     |
          +----------+----------+
                     |
               Shared Core
                     |
                   Rust
                     |
        +------------+------------+
        |            |            |
    Timeline      Media       Project
     Engine       Engine       Engine
        |            |
        +------------+
                     |
                 FFmpeg
                     |
        Decode / Encode / Convert
```

The exact architecture must be validated before implementation.

Rust is preferred for the shared core because it can provide:

- Cross-platform native code
- Memory safety
- Strong concurrency
- Good performance
- Clear separation between UI and processing
- Reusable core across macOS and Windows

---

# 10. FFmpeg STRATEGY

FFmpeg should be evaluated as the primary codec/conversion backend.

However, licensing MUST be treated as an architecture requirement, not something discovered after development.

FFmpeg's official documentation states that most of FFmpeg is LGPL v2.1+, while optional components can be GPL. Using GPL components changes the licensing implications.

For a proprietary/commercial application, the project must establish an explicit FFmpeg build/license strategy before distributing binaries.

Reference:
https://ffmpeg.org/legal.html

The implementation team must:

1. Identify every FFmpeg component used.
2. Record its license.
3. Decide whether LGPL-only configuration is sufficient.
4. Avoid accidentally enabling GPL/nonfree components.
5. Document the exact FFmpeg build.
6. Maintain third-party notices.
7. Have the final distribution reviewed for license compliance.

Do not assume "FFmpeg is LGPL" means every FFmpeg build/configuration has the same licensing implications.

---

# 11. HARDWARE ACCELERATION

Hardware acceleration should be automatic whenever available.

## macOS

Investigate:

- VideoToolbox
- Metal
- Apple Silicon media engines
- AVFoundation/CoreVideo integration

## Windows

Investigate:

- NVIDIA hardware acceleration
- Intel Quick Sync
- AMD hardware acceleration
- Direct3D/D3D12
- Media Foundation

The application should have an abstraction:

```text
HardwareAccelerationManager

detect()
capabilities()
selectDecoder()
selectEncoder()
selectRenderPath()
fallbackToCPU()
```

Never assume a specific GPU exists.

---

# 12. SMART EXPORT

Create a simple export experience.

Default:

```text
EXPORT

Resolution
[ 4K UHD 3840×2160 ]

Quality
[ Recommended ]

Format
[ MP4 ]

Audio
[ Original ]

              [ EXPORT ]
```

Advanced:

```text
Codec
Bitrate
Frame rate
Audio codec
Audio bitrate
Sample rate
Keyframe settings
Hardware acceleration
```

The default mode should hide unnecessary complexity.

---

# 13. PROJECT MODEL

A project should not destroy the original media.

The project should store references to source files plus edit instructions.

Concept:

```text
Project
|
+-- Metadata
+-- Timeline
|   +-- Video Tracks
|   +-- Audio Tracks
|   +-- Clips
|   +-- Transitions
|   +-- Text
|   +-- Effects
|
+-- Source References
|
+-- Export Settings
```

Edits should be non-destructive.

Example:

If the user trims a 20-minute video down to 5 minutes, do NOT modify the original source file.

---

# 14. MEDIA CACHE / PROXY SYSTEM

The application should have a managed cache.

Possible structure:

```text
Project
  |
  +-- Original media
  |
  +-- Cache
       +-- thumbnails
       +-- waveforms
       +-- proxies
       +-- render cache
```

Requirements:

- Cache can be rebuilt.
- Cache can be cleared.
- Cache location can be changed.
- Cache should never be treated as the source of truth.
- Corrupt cache should not corrupt the project.

---

# 15. TIMELINE

The timeline is the heart of the application.

Must support:

- Zoom
- Horizontal scrolling
- Playhead
- Timecode
- Snap
- Clip selection
- Multi-select
- Drag/reorder
- Split at playhead
- Trim handles
- Ripple delete where practical
- Track mute
- Track solo
- Track lock
- Audio waveform
- Video thumbnails
- Undo/redo

Keep the UI visually simple.

---

# 16. PREVIEW WINDOW

Preview should support:

- Fit
- 100%
- Fullscreen
- Play/pause
- Frame step
- Current time
- Duration
- Volume
- Mute

Performance target:

Playback should remain responsive even when the project contains demanding 4K footage, with graceful preview-quality reduction when necessary.

Do not sacrifice timeline responsiveness simply to maintain maximum preview quality.

---

# 17. AUDIO WAVEFORM SYSTEM

Waveforms should be generated asynchronously.

Do not block the UI while processing audio.

Pipeline:

```text
Audio File
   |
Decoder
   |
Waveform Analyzer
   |
Cached Waveform
   |
Timeline
```

Waveform cache should be reused between launches.

---

# 18. BASIC EFFECTS

Initial video effects:

- Brightness
- Contrast
- Saturation
- Exposure where practical
- Blur
- Sharpen
- Grayscale
- Basic color temperature/tint if stable

Initial audio effects:

- Gain
- Normalize
- Fade
- Basic EQ
- Compressor only if implementation is reliable
- Noise reduction only if it can be implemented without making the application bloated

Do not build a giant effects marketplace.

---

# 19. TEXT

Basic text overlay only.

Support:

- Text
- Font
- Size
- Weight
- Alignment
- Position
- Rotation
- Opacity
- Simple background
- Basic animation such as fade

Do not initially build a full motion graphics system.

---

# 20. IMAGE OVERLAYS

Support:

- PNG
- JPEG
- WebP where practical

Features:

- Position
- Scale
- Rotation
- Opacity

PNG transparency must work correctly.

---

# 21. CONVERSION QUEUE

Create a conversion queue.

Example:

```text
CONVERSION QUEUE

video1.mov    4K → 1080p     ███████░░░ 72%
video2.mkv    MKV → MP4      Waiting
song.flac     FLAC → MP3     Waiting
song2.wav     WAV → M4A      Waiting
```

Requirements:

- Pause where technically safe
- Cancel
- Retry
- Remove
- Open output
- Show errors
- Batch processing

---

# 22. FILE HANDLING

Support drag-and-drop throughout the application.

Examples:

- Drop media into project
- Drop media onto timeline
- Drop files into converter
- Drop audio onto video
- Drop images onto timeline

Validate files before processing.

Never crash because of an invalid or partially corrupted media file.

---

# 23. ERROR HANDLING

Errors must be understandable.

Bad:

> FFmpeg error 234

Better:

> This video could not be decoded. The file may be corrupted or use a codec that is not supported on this system.

Provide:

- Human-readable explanation
- Technical details behind an expandable section
- Retry where appropriate
- Suggested action

---

# 24. AUTOSAVE

Projects should autosave.

Requirements:

- Crash-safe project saving
- Recovery after unexpected termination
- Manual save
- Save As
- Recent projects

Autosave must not block the UI.

---

# 25. PRIVACY

Default behavior:

- No cloud upload
- No account required
- No telemetry required for core functionality
- No automatic media uploads
- No remote processing

If telemetry is ever added:

- Explicit opt-in
- Clear explanation
- No media contents
- No personal media uploaded

---

# 26. PERFORMANCE PRINCIPLES

Performance is a product feature.

Never:

- Block the main UI thread with media processing
- Load an entire 4K file into memory unnecessarily
- Generate all thumbnails synchronously
- Generate full waveforms synchronously
- Re-render everything when one timeline item changes
- Re-encode source files unnecessarily

Use:

- Async processing
- Background workers
- GPU acceleration
- Incremental rendering
- Caching
- Proxies
- Lazy thumbnail generation
- Lazy waveform generation

---

# 27. MEMORY MANAGEMENT

Large files are normal.

The application must handle:

- 4K video
- Long videos
- Large WAV files
- Multiple clips
- Multiple audio tracks

without continuously growing memory usage.

Set measurable memory budgets during testing.

---

# 28. UNDO/REDO

Every user-visible editing operation should be undoable.

Examples:

- Trim
- Split
- Delete
- Move
- Crop
- Rotate
- Volume
- Text changes
- Effects
- Track changes

Undo/redo must not duplicate massive media files.

Use command/state changes rather than destructive media copies.

---

# 29. KEYBOARD SHORTCUTS

At minimum:

- Space — play/pause
- Cmd/Ctrl+Z — undo
- Cmd/Ctrl+Shift+Z — redo
- Delete/Backspace — delete
- S — split
- I — set in point where appropriate
- O — set out point where appropriate
- Arrow keys — frame/time navigation
- Cmd/Ctrl+S — save
- Cmd/Ctrl+Shift+S — Save As

Exact shortcuts must be validated against native platform conventions.

---

# 30. UI STRUCTURE

Suggested main application:

```text
┌─────────────────────────────────────────────────────────┐
│ Media   Edit   View   Project   Export                  │
├───────────────┬─────────────────────────┬───────────────┤
│               │                         │               │
│ Media         │        Preview          │ Inspector     │
│ Library       │                         │               │
│               │                         │               │
│ + Import      │                         │ Properties    │
│               │                         │               │
├───────────────┴─────────────────────────┴───────────────┤
│                                                         │
│                    TIMELINE                             │
│                                                         │
│ V1 ────[Clip]────[Clip]──────────────                  │
│ V2 ──────────────[Clip]──────────────                  │
│ A1 ────[Audio]───[Audio]────────────                  │
│ A2 ──────────────[Music]────────────                  │
│                                                         │
└─────────────────────────────────────────────────────────┘
```

Do not overload the interface.

---

# 31. APPLICATION MODES

Recommended top-level modes:

## EDIT

Video and audio timeline.

## AUDIO

Dedicated waveform/audio editor.

## CONVERT

Batch media converter.

## TOOLS

Simple utilities:

- Extract audio
- Extract frames
- Resize
- Compress
- Merge videos
- Merge audio
- Change format
- Change resolution

This structure keeps the product useful without making the editor itself complicated.

---

# 32. MUST-HAVE VS LATER

## MUST HAVE — VERSION 1

- Native macOS application
- Native Windows application
- 4K
- 2K
- 1080p
- 720p
- Video import
- Audio import
- Timeline
- Trim
- Split
- Cut
- Delete
- Reorder
- Basic text
- Basic image overlay
- Crop
- Rotate
- Speed
- Volume
- Fade
- Audio waveform
- Basic audio editing
- Audio conversion
- Video conversion
- Batch conversion
- Export presets
- Hardware acceleration
- Project save/load
- Autosave/recovery
- Undo/redo
- Drag/drop

## VERSION 1.1+

Potential additions:

- More effects
- More audio effects
- Proxy controls
- Better subtitle support
- More export codecs
- Better HDR workflows
- More advanced transitions
- Better color controls

## DO NOT BUILD INITIALLY

- AI video generation
- AI avatars
- Cloud editing
- Collaboration
- Stock media
- 3D
- Full motion graphics
- Professional color grading
- Multicam
- Plugin marketplace
- Social network
- Subscription-required processing
- Giant effects library

---

# 33. PHASE 0 — RESEARCH BEFORE CODE

Do NOT start by building the UI.

First produce:

`docs/PHASE_0_ARCHITECTURE_RESEARCH.md`

Research and compare:

1. SwiftUI vs AppKit for the Mac UI
2. Rust integration strategy
3. FFmpeg integration
4. AVFoundation integration
5. VideoToolbox integration
6. Metal rendering
7. Windows UI framework options
8. Windows hardware acceleration
9. NVIDIA/Intel/AMD paths
10. Audio processing libraries
11. Timeline architecture
12. Proxy architecture
13. Cache architecture
14. Project file format
15. macOS sandbox requirements
16. macOS signing/notarization
17. Windows packaging/signing
18. FFmpeg licensing
19. Codec licensing/patent considerations
20. Memory/performance architecture

The research must identify risks before code is written.

---

# 34. PHASE 0 DECISION GATE

At the end of Phase 0, create:

`docs/ARCHITECTURE_DECISION.md`

It must answer:

- What language?
- What UI framework?
- What media backend?
- What rendering backend?
- How will 4K work?
- How will hardware acceleration work?
- How will audio work?
- How will Windows differ from macOS?
- How will projects be stored?
- How will cache/proxies work?
- How will FFmpeg be distributed?
- What are the licensing risks?
- What are the biggest technical risks?

Do not proceed until these decisions are internally consistent.

---

# 35. PHASE 1 — TECHNICAL PROTOTYPE

Build only technical proof-of-concepts.

No polished UI.

Validate:

- Open 4K video
- Decode
- Display
- Seek
- Play
- Stop
- Extract audio
- Decode audio
- Generate waveform
- Encode 1080p
- Encode 2K
- Encode 4K
- Detect hardware acceleration
- Perform a basic conversion

Measure:

- CPU
- GPU
- RAM
- Decode FPS
- Preview FPS
- Export speed
- Stability

Create:

`docs/PHASE_1_RESULTS.md`

---

# 36. PHASE 2 — PROJECT/TIMELINE ENGINE

Build:

- Project model
- Media assets
- Timeline
- Clips
- Tracks
- Timecode
- Trim
- Split
- Move
- Delete
- Undo/redo
- Save/load
- Autosave

No advanced effects yet.

---

# 37. PHASE 3 — MAC NATIVE APPLICATION

Build the polished macOS application.

Prioritize:

- Native UI
- Smooth timeline
- Metal preview
- VideoToolbox
- Apple Silicon optimization
- Native drag/drop
- Native menus
- Keyboard shortcuts
- Save/recovery
- 4K playback

Test on multiple Mac hardware classes if available.

---

# 38. PHASE 4 — AUDIO

Build:

- Waveforms
- Audio timeline
- Trim
- Split
- Fade
- Gain
- Normalize
- EQ
- Conversion
- Extraction from video
- Audio export

Audio must remain responsive while video processing occurs.

---

# 39. PHASE 5 — WINDOWS

Port the shared core.

Build Windows-specific:

- Native UI
- File handling
- Hardware acceleration
- Packaging
- Installation
- GPU paths

Do not compromise the architecture merely to force identical implementation details between platforms.

The user experience should be consistent, while platform implementation can differ.

---

# 40. PHASE 6 — CONVERTER + TOOLS

Build the dedicated converter and utility tools.

Focus on reliability and batch processing.

---

# 41. PHASE 7 — PERFORMANCE

Create benchmark projects:

### Test A
1080p H.264

### Test B
2K H.264

### Test C
4K H.264

### Test D
4K HEVC

### Test E
4K 60 FPS

### Test F
Long-form 4K video

### Test G
Large WAV

### Test H
Multiple audio tracks

### Test I
Multiple simultaneous conversions

Record:

- Startup time
- Import time
- Thumbnail generation
- Waveform generation
- Scrubbing latency
- Playback FPS
- Export time
- Peak RAM
- CPU usage
- GPU usage
- Crash rate

---

# 42. PHASE 8 — FAILURE TESTING

Test:

- Corrupted video
- Corrupted audio
- Missing media
- Disconnected drive
- Very large files
- Very long filenames
- Unicode filenames
- Spaces in paths
- Read-only folders
- Full disk
- Interrupted export
- App termination during save
- App termination during export
- Sleep/wake
- Multiple monitors
- High DPI
- Retina
- Unsupported codec
- Unsupported resolution
- Unsupported frame rate

The app must fail gracefully.

---

# 43. PHASE 9 — POLISH

Only after functionality is stable:

- UI refinement
- Animations
- Icons
- Empty states
- Error messages
- Accessibility
- Keyboard navigation
- Help
- Preferences
- Performance tuning

Do not polish broken functionality.

---

# 44. PHASE 10 — RELEASE AUDIT

Before release, perform a complete audit.

Check:

### Functionality
Every advertised feature works.

### Performance
4K remains usable.

### Stability
No known reproducible crashes.

### Data safety
Original files are never accidentally destroyed.

### Licensing
All third-party dependencies are documented and compliant.

### Packaging
macOS signing/notarization works.

Windows installer works.

### Privacy
No unexpected network activity.

### UX
A new user can edit a basic video without documentation.

---

# 45. TESTING RULE

Every feature must have:

1. Implementation
2. Unit test where practical
3. Integration test where practical
4. Manual UI test
5. Failure test
6. Performance consideration

Do not mark a feature complete merely because the happy path works.

---

# 46. AGENT DEVELOPMENT RULES

If Claude/Codex is implementing this project:

## Rule 1
Do not build features that are not approved.

## Rule 2
Do not skip architectural validation.

## Rule 3
Do not assume a library is safe to distribute commercially.

## Rule 4
Do not hide compiler warnings.

## Rule 5
Do not leave dead code.

## Rule 6
Do not create fake/stub functionality and call it complete.

## Rule 7
Do not disable tests merely to make the build pass.

## Rule 8
Do not optimize prematurely, but measure performance continuously.

## Rule 9
Never destroy source media.

## Rule 10
Never invent support for a codec or format that has not been tested.

## Rule 11
When a feature cannot be implemented reliably, document the limitation instead of pretending it works.

## Rule 12
At the end of every phase, produce a written validation report.

---

# 47. QUALITY BAR

The application should feel:

- Fast
- Clean
- Native
- Stable
- Predictable
- Simple

It should NOT feel:

- Bloated
- Slow
- Confusing
- Web-based
- Over-engineered
- Feature-heavy

---

# 48. PRODUCT SUCCESS CRITERIA

A successful V1 should allow a user to do this:

```text
Open App
   ↓
Import 4K Video
   ↓
Drag onto Timeline
   ↓
Trim unwanted section
   ↓
Split clip
   ↓
Add another clip
   ↓
Add music
   ↓
Adjust music volume
   ↓
Add simple text
   ↓
Preview
   ↓
Export 4K / 2K / 1080p
   ↓
Done
```

And independently:

```text
Open App
   ↓
Audio
   ↓
Import FLAC/WAV/MP3
   ↓
Edit waveform
   ↓
Trim/Fade/Normalize/EQ
   ↓
Export MP3/WAV/M4A/FLAC
```

And:

```text
Open App
   ↓
Convert
   ↓
Drop 20 files
   ↓
Choose output format
   ↓
Convert
   ↓
Done
```

If these three workflows are excellent, V1 is successful.

---

# 49. FINAL ARCHITECTURAL PRINCIPLE

The most important architectural principle is:

> Share the media intelligence and business logic. Keep platform-specific UI and hardware integration native.

Therefore:

```text
                  SHARED
              Rust Core
                  |
       +----------+----------+
       |                     |
    macOS                  Windows
       |                     |
    Swift                  Native
   SwiftUI                Windows UI
       |                     |
    Metal                 Direct3D
VideoToolbox           Windows GPU APIs
       |                     |
       +----------+----------+
                  |
              Media Layer
                  |
                FFmpeg
```

This gives the project:

- Cross-platform reuse
- Native performance
- Native UX
- Hardware acceleration
- Maintainability
- A realistic path to a lightweight application

---

# 50. FIRST INSTRUCTION TO THE DEVELOPMENT AGENT

Before writing application code:

> Read this entire specification.
>
> Do not build anything yet.
>
> Start Phase 0.
>
> Research the architecture required to build this application as a lightweight native macOS + Windows video/audio editor.
>
> Pay particular attention to:
>
> - SwiftUI/AppKit
> - Rust
> - FFmpeg
> - VideoToolbox
> - Metal
> - Windows native UI options
> - Windows hardware acceleration
> - Audio architecture
> - 4K/2K/1080p playback and export
> - Timeline architecture
> - Proxy/cache architecture
> - Project format
> - macOS distribution
> - Windows distribution
> - FFmpeg licensing
> - Codec licensing
>
> Do not make architectural decisions based on assumptions.
>
> Produce:
>
> 1. `docs/PHASE_0_ARCHITECTURE_RESEARCH.md`
> 2. `docs/ARCHITECTURE_DECISION.md`
> 3. `docs/RISK_REGISTER.md`
> 4. `docs/DEPENDENCY_AND_LICENSE_AUDIT.md`
> 5. `docs/PHASE_0_VALIDATION.md`
>
> Identify anything in this specification that is technically weak, unnecessarily complex, contradictory, or risky.
>
> Challenge the specification where necessary.
>
> Do not blindly agree with it.
>
> Phase 0 is complete only when the architecture is justified, the major risks are understood, and the project has a defensible path to a fast 4K-capable native macOS and Windows application.
