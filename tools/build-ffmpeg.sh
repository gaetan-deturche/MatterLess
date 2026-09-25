#!/usr/bin/env bash
# Builds the ffmpeg the video player loads: the same source as the bindings
# were generated from, with only what the player uses in it. Cross-compiled
# for 64-bit Windows from Linux; run by .github/workflows/ffmpeg.yml.
#
#   bash tools/build-ffmpeg.sh <out>
#
# needs: gcc-mingw-w64-x86-64 nasm meson ninja-build pkg-config curl git
#
# <out>/bin holds the five DLLs, <out>/LICENSE.txt ffmpeg's licence and
# <out>/dav1d-LICENSE.txt dav1d's: the layout tools/fetch-ffmpeg.ps1 leaves.
#
# LGPL 2.1 or later: nothing GPL, nothing non-free. dav1d (BSD) is linked into
# avcodec, because ffmpeg's own AV1 decoder only works on a GPU and a poster is
# made on the CPU.

set -euo pipefail

FFMPEG_COMMIT=a5923073bf
DAV1D_TAG=1.5.1
HOST=x86_64-w64-mingw32

# What the player, the posters and the drag previews meet in practice: phone
# and screen recordings, webm, and the odd old camera file.
DEMUXERS=(mov matroska avi mpegts mpegps ogg mp3 wav flac aac)
DECODERS=(
    h264 hevc vp8 vp9 av1 libdav1d mpeg4 mpeg1video mpeg2video mjpeg
    aac aac_latm mp3 mp3float opus vorbis flac ac3 eac3 alac
    pcm_s16le pcm_s16be pcm_s24le pcm_s24be pcm_f32le pcm_u8
)
PARSERS=(h264 hevc vp8 vp9 av1 mpeg4video mpegvideo mjpeg aac aac_latm mpegaudio opus vorbis flac ac3)
# The `2` ones are the D3D11 surfaces the player asks for; the others come
# with them.
HWACCELS=(
    h264_d3d11va h264_d3d11va2 hevc_d3d11va hevc_d3d11va2
    vp9_d3d11va vp9_d3d11va2 av1_d3d11va av1_d3d11va2
    mpeg2_d3d11va mpeg2_d3d11va2
)
PROTOCOLS=(file)

out=$(realpath -m "${1:?usage: build-ffmpeg.sh <out>}")
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
prefix=$work/prefix
jobs=$(nproc)

joined() { local IFS=,; echo "$*"; }

echo "== dav1d $DAV1D_TAG"
git clone --quiet --depth 1 --branch "$DAV1D_TAG" https://code.videolan.org/videolan/dav1d.git "$work/dav1d"
cat > "$work/cross.meson" <<EOF
[binaries]
c = '$HOST-gcc'
ar = '$HOST-ar'
strip = '$HOST-strip'
windres = '$HOST-windres'
[host_machine]
system = 'windows'
cpu_family = 'x86_64'
cpu = 'x86_64'
endian = 'little'
EOF
meson setup "$work/dav1d/build" "$work/dav1d" \
    --cross-file="$work/cross.meson" --prefix="$prefix" --libdir=lib \
    --buildtype=release --default-library=static \
    -Denable_tools=false -Denable_tests=false -Denable_examples=false
ninja -C "$work/dav1d/build" install

echo "== ffmpeg $FFMPEG_COMMIT"
mkdir "$work/ffmpeg"
curl -fsSL "https://github.com/FFmpeg/FFmpeg/archive/$FFMPEG_COMMIT.tar.gz" |
    tar -xz --strip-components=1 -C "$work/ffmpeg"
cd "$work/ffmpeg"
PKG_CONFIG_LIBDIR="$prefix/lib/pkgconfig" ./configure \
    --prefix="$prefix" \
    --enable-cross-compile --target-os=mingw32 --arch=x86_64 --cross-prefix="$HOST-" \
    --pkg-config=pkg-config --pkg-config-flags=--static \
    --enable-shared --disable-static \
    --disable-programs --disable-doc --disable-debug \
    --disable-avdevice --disable-avfilter --disable-network \
    --disable-autodetect --enable-w32threads --enable-d3d11va --enable-libdav1d \
    --disable-everything \
    --enable-demuxer="$(joined "${DEMUXERS[@]}")" \
    --enable-decoder="$(joined "${DECODERS[@]}")" \
    --enable-parser="$(joined "${PARSERS[@]}")" \
    --enable-hwaccel="$(joined "${HWACCELS[@]}")" \
    --enable-protocol="$(joined "${PROTOCOLS[@]}")" \
    --extra-ldflags=-static-libgcc

# configure only warns about a name it does not know, and a component whose
# dependencies are missing is quietly left out: check each one made it in.
missing=0
want() {
    local kind=$1; shift
    for name in "$@"; do
        if ! grep -q "^#define CONFIG_${name^^}_${kind} 1" config_components.h; then
            echo "not built: $name ${kind,,}"
            missing=1
        fi
    done
}
want DEMUXER "${DEMUXERS[@]}"
want DECODER "${DECODERS[@]}"
want PARSER "${PARSERS[@]}"
want HWACCEL "${HWACCELS[@]}"
want PROTOCOL "${PROTOCOLS[@]}"
grep -q '^#define CONFIG_D3D11VA 1' config.h || { echo "not built: d3d11va"; missing=1; }
[ "$missing" = 0 ]

make -j"$jobs"
make install

rm -rf "$out"
mkdir -p "$out/bin"
for dll in avutil-61 swresample-7 swscale-10 avcodec-63 avformat-63; do
    "$HOST-strip" --strip-unneeded -o "$out/bin/$dll.dll" "$prefix/bin/$dll.dll"
done
cp COPYING.LGPLv2.1 "$out/LICENSE.txt"
cp "$work/dav1d/COPYING" "$out/dav1d-LICENSE.txt"

# A DLL the build leaned on without saying so would be missing on every
# reader's machine: only Windows' own may be imported, and each other.
bad=$("$HOST-objdump" -p "$out"/bin/*.dll | sed -n 's/^\s*DLL Name: //p' | sort -u |
    grep -viE '^(kernel32|user32|advapi32|bcrypt|ole32|shell32|msvcrt|ucrtbase|api-ms-win-.*)\.dll$|^(avutil|swresample|swscale|avcodec|avformat)-[0-9]+\.dll$' || true)
if [ -n "$bad" ]; then
    echo "imports outside Windows: $bad"
    exit 1
fi

du -h "$out"/bin/*.dll
