# The oldest macOS the product supports. ONE definition, sourced by every build.
#
# This exists because the three builds drifted apart silently. `Info.plist` said
# 14.0 while the Swift binary, the FFmpeg dylibs and the whisper static libs
# were all built for whatever the developer's machine happened to be running —
# 26.0. Launch Services would have allowed the app onto a macOS 14 machine and
# dyld would then have refused the binaries.
#
# Nothing in the build warns about this: each piece is individually correct, and
# only the combination is wrong. `tools/check.sh` asserts they all agree.
#
# **The floor is 14.0 because of `CADisplayLink`**, which the preview's playback
# clock uses and which came to macOS in 14.0. Lowering it means finding another
# clock; raising it means abandoning users for no reason yet identified.
export MACOS_DEPLOYMENT_TARGET="14.0"
export MACOSX_DEPLOYMENT_TARGET="$MACOS_DEPLOYMENT_TARGET"
