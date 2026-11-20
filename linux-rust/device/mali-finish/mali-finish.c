/* mali-finish: a stand-in "libEGL.so.1" loaded only by the browser (see ../shabnam-browser).
 *
 * The fault it removes: inside the browser window, during scrolling, animations and page changes, single
 * frames showed blocks of old and new content side by side.
 *
 * Why it happens: Chromium hands each finished window picture (a dma-buf) to the compositor right after
 * glFlush(), which only *starts* the drawing. It counts on one of two things to keep the compositor from
 * showing a half-drawn picture: a "drawing done" signal from the driver that it can wait for
 * (EGL_ANDROID_native_fence_sync), or the kernel making the reader wait (dma-buf fences). The Mali blob here
 * (g2p0, wayland-gbm) has no such extension and the kernel is built without CONFIG_MALI_BIFROST_DMA_FENCE,
 * so nothing waits: Chromium never creates a single sync object (counted), and the compositor sometimes
 * samples the picture while the GPU is still filling it in. Rockchip patches Chromium's source for this
 * ("Create new fence when there's no in-fences"); with the stock Void Chromium the same is done from outside:
 * glFlush() ("start drawing") becomes glFinish() ("return when the drawing is done").
 *
 * Measured on the test page (../flicker-test, 2026-11-20, screenshots with broken frames / all screenshots):
 *   without this library  8 of 156 (5 %), about 44 frames a second
 *   with this library     0 of 114,       about 39 frames a second
 * No Chromium flag and no niri debug option removed the fault; software compositing did, at 10 frames a second.
 *
 * How it works: Chromium's GL layer (ANGLE, built into the chromium binary) opens "libEGL.so.1" by name and
 * asks its eglGetProcAddress for every GL function. This library is found first (its directory is put in
 * front of LD_LIBRARY_PATH by the wrapper), answers for glFlush, and passes every other question to the
 * real libmali.so.1. It depends on libmali.so.1, so all other EGL symbols are found through it unchanged.
 * SHABNAM_GL_FINISH=0 in the environment switches the change off, to see the fault again.
 *
 * Build on the Mac: ./build.sh (needs zig). Install: /usr/local/lib/shabnam-browser-gl/libEGL.so.1, mode 755.
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdlib.h>
#include <string.h>

typedef void (*fn_t)(void);
static fn_t (*real_get_proc)(const char *);
static void (*real_finish)(void);
static void (*real_flush)(void);
static int finish_on_flush = 1;

__attribute__((constructor)) static void init(void) {
  void *mali = dlopen("libmali.so.1", RTLD_NOW);
  if (!mali) abort();
  real_get_proc = (fn_t (*)(const char *))dlsym(mali, "eglGetProcAddress");
  real_finish = (void (*)(void))dlsym(mali, "glFinish");
  real_flush = (void (*)(void))dlsym(mali, "glFlush");
  if (!real_get_proc || !real_finish || !real_flush) abort();
  const char *e = getenv("SHABNAM_GL_FINISH");
  if (e && e[0] == '0') finish_on_flush = 0;
}

/* static, and handed out by address below: a reference to the exported name "glFlush" from inside this
 * library would be bound by the dynamic linker to libmali's own glFlush (it is earlier in the browser's
 * global symbol list, through libgbm), and the change would silently do nothing. */
static void flush_and_wait(void) {
  if (finish_on_flush) real_finish(); else real_flush();
}

__attribute__((visibility("default"))) void glFlush(void) { flush_and_wait(); }

__attribute__((visibility("default"))) fn_t eglGetProcAddress(const char *name) {
  if (name && strcmp(name, "glFlush") == 0) return (fn_t)flush_and_wait;
  return real_get_proc(name);
}
