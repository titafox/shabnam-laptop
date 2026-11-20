# Sourced by the build.sh files here. Sets up the cross-build of a Rust program on the Mac for the
# laptop (aarch64, glibc 2.41) with cargo-zigbuild. Never build on the device.
#   WORK    = a scratch directory for the source checkouts and the target directories (default below)
#   SYSROOT = a copy of the device's /usr tree, for the few programs that link a system library
: "${WORK:=${TMPDIR:-/tmp}/shabnam-apps}"
TARGET=aarch64-unknown-linux-gnu.2.41
TRIPLE=aarch64-unknown-linux-gnu
mkdir -p "$WORK/src" "$WORK/target" "$WORK/out"
if [ -n "$SYSROOT" ]; then
  export PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_SYSROOT_DIR="$SYSROOT" PKG_CONFIG_LIBDIR="$SYSROOT/usr/lib/pkgconfig"
fi

# checkout NAME URL COMMIT : $WORK/src/NAME at exactly COMMIT (a full hash), without local changes.
checkout() {
  d=$WORK/src/$1
  [ -d "$d/.git" ] || git init -q "$d" || return 1
  git -C "$d" fetch -q --depth 1 "$2" "$3" && git -C "$d" checkout -q -f FETCH_HEAD && git -C "$d" clean -q -fdx || return 1
  [ "$(git -C "$d" rev-parse HEAD)" = "$3" ] || { echo "$1: not at $3" >&2; return 1; }
}

# patched_crate NAME VERSION PATCH : copies the crate cargo already fetched (checksum-verified, from
# its registry cache) to $WORK/src/NAME-VERSION-shabnam, applies PATCH (-p1) and prints the directory.
# Use it with:  cargo ... --config "patch.crates-io.NAME.path='DIR'"   (after `cargo fetch --locked`;
# build with --offline instead of --locked, because a path patch changes that one Cargo.lock entry).
patched_crate() {
  src=$(ls -d "${CARGO_HOME:-$HOME/.cargo}"/registry/src/*/"$1-$2" 2>/dev/null | head -1)
  [ -d "$src" ] || { echo "$1-$2 is not in cargo's registry cache (run cargo fetch --locked first)" >&2; return 1; }
  dst=$WORK/src/$1-$2-shabnam
  rm -rf "$dst" && cp -R "$src" "$dst" && rm -f "$dst/.cargo-ok" && patch -s -p1 -d "$dst" < "$3" || return 1
  echo "$dst"
}
