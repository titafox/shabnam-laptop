/* shabnam-video — the Shabnam video player window.
 *
 * GStreamer does the demuxing and decoding (Rockchip's mppvideodec when the plugin is installed, software
 * decoders otherwise); this program is the part GStreamer has no usable piece for on this machine: a Wayland
 * window that hands the decoder's NV12 dmabuf straight to the compositor.
 *
 * Why not waylandsink: it only takes dmabufs whose format the compositor lists with an explicit modifier,
 * and the Mali driver lists none that way; what is left is a CPU copy through shared memory.
 *
 * Why RGB and not the decoder's own NV12: the compositor (niri on the Mali driver) accepts an NV12 dmabuf but
 * draws it black — it binds the image as an ordinary 2D texture, and this driver wants YUV images bound as
 * "external" textures (niri log: "glEGLImageTargetTexture2DOES: unsupported format"). So mppvideodec is told
 * to convert every frame to BGRx with the RGA 2D accelerator (hardware, dmabuf to dmabuf); that dmabuf is
 * attached as it is (zwp_linux_dmabuf_v1, modifier DRM_FORMAT_MOD_INVALID). No pixel is touched by the CPU.
 * SHABNAM_VIDEO_NV12=1 hands over the NV12 dmabuf directly, for the day the compositor can show it.
 * Frames from software decoders are converted to BGRx on the CPU and copied through wl_shm.
 *
 * Keys: Space/K pause, Left/Right seek 10 s (Shift: 60 s), Up/Down volume, M mute, F/F11 or double click
 * fullscreen, Esc leave fullscreen, Q quit. A click pauses.
 *
 * usage: shabnam-video [--audio URI] [--title T] [--user-agent UA] [--referer R] [--sw] [--stats]
 *                      [--headless] [--selftest STEPS] FILE-or-URI
 * --sw: software decoder; --stats: frame counts on stderr; --headless: decode and time the frames but open
 * no window and play no sound; --selftest: see on_selftest().
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>

#include <glib-unix.h>
#include <gst/gst.h>
#include <gst/app/gstappsink.h>
#include <gst/allocators/gstdmabuf.h>
#include <gst/video/video.h>
#include <wayland-client.h>

#include "xdg-shell-client.h"
#include "viewporter-client.h"
#include "linux-dmabuf-v1-client.h"
#include "presentation-time-client.h"

#define FOURCC(a, b, c, d) ((guint32) (a) | ((guint32) (b) << 8) | ((guint32) (c) << 16) | ((guint32) (d) << 24))
#define DRM_NV12 FOURCC ('N', 'V', '1', '2')
#define DRM_XRGB8888 FOURCC ('X', 'R', '2', '4')
#define MOD_INVALID_HI 0x00ffffffu
#define MOD_INVALID_LO 0xffffffffu

/* evdev key codes (wl_keyboard sends these; no keymap handling is needed for a handful of fixed keys) */
enum { K_ESC = 1, K_Q = 16, K_F = 33, K_K = 37, K_M = 50, K_SPACE = 57, K_F11 = 87,
  K_UP = 103, K_LEFT = 105, K_RIGHT = 106, K_DOWN = 108 };

#define BAR_HEIGHT 6
#define BAR_SHOW_MS 2500
#define SHM_POOL 4

typedef struct {
  struct wl_buffer *buffer;
  void *data;
  gsize size;
  int width, height;
  gboolean busy;
} ShmSlot;

typedef struct {
  struct wl_buffer *buffer;
  GstSample *sample;            /* keeps the decoder's dmabuf alive until the compositor lets go */
} Frame;

static struct {
  /* options */
  const char *audio_uri, *title, *user_agent, *referer;
  gboolean sw, stats, headless;

  GMainLoop *loop;
  int exit_code;

  /* wayland */
  struct wl_display *display;
  struct wl_compositor *compositor;
  struct wl_subcompositor *subcompositor;
  struct wl_shm *shm;
  struct wl_seat *seat;
  struct wl_keyboard *keyboard;
  struct wl_pointer *pointer;
  struct xdg_wm_base *wm_base;
  struct wp_viewporter *viewporter;
  struct zwp_linux_dmabuf_v1 *dmabuf;
  struct wp_presentation *presentation;

  struct wl_surface *bg, *video, *bar, *fill;
  struct wl_subsurface *video_sub, *bar_sub, *fill_sub;
  struct wp_viewport *bg_vp, *video_vp, *bar_vp, *fill_vp;
  struct xdg_surface *xdg_surface;
  struct xdg_toplevel *toplevel;
  struct wl_buffer *black, *dark, *teal;

  int win_w, win_h;             /* logical size of the window */
  int pending_w, pending_h;
  gboolean configured, fullscreen, video_mapped;
  int src_w, src_h;             /* visible part of the frame, in buffer pixels */
  int src_x, src_y;
  int par_n, par_d;
  gboolean layout_dirty;
  gboolean shift;
  guint32 last_click;
  guint bar_timer, bar_tick;
  gboolean bar_visible;

  ShmSlot shm_pool[SHM_POOL];

  /* playback */
  GstElement *pipeline, *appsink, *volume, *vconv, *aconv;
  gboolean have_video, have_audio;
  gboolean paused, buffering, muted;
  GHashTable *levels;           /* fill level of each network buffer, in percent */
  guint buffering_timer;
  gboolean dmabuf_frames;

  /* counters */
  guint64 committed, presented, discarded, shm_frames, dma_frames;
  gint64 first_commit_us, last_commit_us;
} app;

/* ---------------------------------------------------------------- shared-memory buffers */

static struct wl_buffer *
shm_buffer_new (int width, int height, guint32 format, void **data_out, gsize *size_out)
{
  gsize stride = (gsize) width * 4, size = stride * height;
  int fd = memfd_create ("shabnam-video", MFD_CLOEXEC);
  if (fd < 0 || ftruncate (fd, size) < 0) {
    if (fd >= 0)
      close (fd);
    return NULL;
  }
  void *data = mmap (NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
  if (data == MAP_FAILED) {
    close (fd);
    return NULL;
  }
  struct wl_shm_pool *pool = wl_shm_create_pool (app.shm, fd, size);
  struct wl_buffer *buffer = wl_shm_pool_create_buffer (pool, 0, width, height, stride, format);
  wl_shm_pool_destroy (pool);
  close (fd);
  if (data_out) {
    *data_out = data;
    *size_out = size;
  } else {
    munmap (data, size);
  }
  return buffer;
}

static struct wl_buffer *
solid_buffer (guint32 argb)
{
  void *data;
  gsize size;
  struct wl_buffer *b = shm_buffer_new (1, 1, WL_SHM_FORMAT_ARGB8888, &data, &size);
  if (b) {
    memcpy (data, &argb, 4);
    munmap (data, size);
  }
  return b;
}

static void
shm_slot_release (void *data, struct wl_buffer *buffer)
{
  ((ShmSlot *) data)->busy = FALSE;
}

static const struct wl_buffer_listener shm_slot_listener = { shm_slot_release };

static ShmSlot *
shm_slot_get (int width, int height)
{
  ShmSlot *free_slot = NULL;
  for (int i = 0; i < SHM_POOL; i++) {
    ShmSlot *s = &app.shm_pool[i];
    if (s->busy)
      continue;
    if (s->buffer && s->width == width && s->height == height)
      return s;
    if (!free_slot)
      free_slot = s;
  }
  if (!free_slot)
    return NULL;                /* the compositor still holds every buffer: skip this frame */
  if (free_slot->buffer) {
    wl_buffer_destroy (free_slot->buffer);
    munmap (free_slot->data, free_slot->size);
    free_slot->buffer = NULL;
  }
  free_slot->buffer = shm_buffer_new (width, height, WL_SHM_FORMAT_XRGB8888, &free_slot->data, &free_slot->size);
  free_slot->width = width;
  free_slot->height = height;
  if (!free_slot->buffer)
    return NULL;
  wl_buffer_add_listener (free_slot->buffer, &shm_slot_listener, free_slot);
  return free_slot;
}

/* ---------------------------------------------------------------- layout */

static void
layout (void)
{
  if (!app.configured || app.headless)
    return;
  wp_viewport_set_destination (app.bg_vp, app.win_w, app.win_h);
  if (app.src_w > 0 && app.src_h > 0) {
    /* Fit the picture into the window, keeping its shape; the rest stays black. */
    double aspect = ((double) app.src_w * app.par_n) / ((double) app.src_h * app.par_d);
    int w = app.win_w, h = (int) (app.win_w / aspect + 0.5);
    if (h > app.win_h) {
      h = app.win_h;
      w = (int) (app.win_h * aspect + 0.5);
    }
    w = MAX (w, 1);
    h = MAX (h, 1);
    wp_viewport_set_source (app.video_vp, wl_fixed_from_int (app.src_x), wl_fixed_from_int (app.src_y),
        wl_fixed_from_int (app.src_w), wl_fixed_from_int (app.src_h));
    wp_viewport_set_destination (app.video_vp, w, h);
    wl_subsurface_set_position (app.video_sub, (app.win_w - w) / 2, (app.win_h - h) / 2);
  }
  app.layout_dirty = FALSE;
}

static void
bar_update (void)
{
  if (!app.bar_visible || app.headless || !app.configured)
    return;
  gint64 pos = 0, dur = 0;
  double f = 0;
  if (gst_element_query_position (app.pipeline, GST_FORMAT_TIME, &pos) &&
      gst_element_query_duration (app.pipeline, GST_FORMAT_TIME, &dur) && dur > 0)
    f = CLAMP ((double) pos / dur, 0.0, 1.0);
  int y = app.win_h - BAR_HEIGHT;
  wl_subsurface_set_position (app.bar_sub, 0, y);
  wp_viewport_set_destination (app.bar_vp, app.win_w, BAR_HEIGHT);
  wl_surface_attach (app.bar, app.dark, 0, 0);
  wl_surface_damage_buffer (app.bar, 0, 0, 1, 1);
  wl_surface_commit (app.bar);
  int fw = (int) (app.win_w * f);
  wl_subsurface_set_position (app.fill_sub, 0, y);
  if (fw > 0) {
    wp_viewport_set_destination (app.fill_vp, fw, BAR_HEIGHT);
    wl_surface_attach (app.fill, app.teal, 0, 0);
    wl_surface_damage_buffer (app.fill, 0, 0, 1, 1);
  } else {
    wl_surface_attach (app.fill, NULL, 0, 0);
  }
  wl_surface_commit (app.fill);
  wl_surface_commit (app.bg);
  wl_display_flush (app.display);
}

static gboolean
bar_tick_cb (gpointer data)
{
  bar_update ();
  return G_SOURCE_CONTINUE;
}

static gboolean
bar_hide_cb (gpointer data)
{
  app.bar_timer = 0;
  if (app.paused)               /* while paused the bar stays, so the position can be read */
    return G_SOURCE_REMOVE;
  app.bar_visible = FALSE;
  if (app.bar_tick) {
    g_source_remove (app.bar_tick);
    app.bar_tick = 0;
  }
  wl_surface_attach (app.bar, NULL, 0, 0);
  wl_surface_commit (app.bar);
  wl_surface_attach (app.fill, NULL, 0, 0);
  wl_surface_commit (app.fill);
  wl_surface_commit (app.bg);
  wl_display_flush (app.display);
  return G_SOURCE_REMOVE;
}

static void
bar_show (void)
{
  if (app.headless || !app.configured)
    return;
  app.bar_visible = TRUE;
  if (app.bar_timer)
    g_source_remove (app.bar_timer);
  app.bar_timer = g_timeout_add (BAR_SHOW_MS, bar_hide_cb, NULL);
  if (!app.bar_tick)
    app.bar_tick = g_timeout_add (250, bar_tick_cb, NULL);
  bar_update ();
}

/* ---------------------------------------------------------------- frames */

static void
frame_free (Frame *f)
{
  if (f->sample)
    gst_sample_unref (f->sample);
  if (f->buffer)
    wl_buffer_destroy (f->buffer);
  g_free (f);
}

static void
buffer_release (void *data, struct wl_buffer *buffer)
{
  frame_free (data);
}

static const struct wl_buffer_listener buffer_listener = { buffer_release };

static void
feedback_sync_output (void *data, struct wp_presentation_feedback *fb, struct wl_output *output)
{
}

static void
feedback_presented (void *data, struct wp_presentation_feedback *fb, uint32_t tv_sec_hi, uint32_t tv_sec_lo,
    uint32_t tv_nsec, uint32_t refresh, uint32_t seq_hi, uint32_t seq_lo, uint32_t flags)
{
  app.presented++;
  wp_presentation_feedback_destroy (fb);
}

static void
feedback_discarded (void *data, struct wp_presentation_feedback *fb)
{
  app.discarded++;
  wp_presentation_feedback_destroy (fb);
}

static const struct wp_presentation_feedback_listener feedback_listener = {
  feedback_sync_output, feedback_presented, feedback_discarded
};

/* Runs on the main thread for every frame the sink has released for display. */
static gboolean
show_sample (gpointer data)
{
  GstSample *sample = data;
  GstBuffer *buf = gst_sample_get_buffer (sample);
  GstCaps *caps = gst_sample_get_caps (sample);
  GstVideoInfo info;
  Frame *frame = NULL;

  if (!buf || !caps || !gst_video_info_from_caps (&info, caps))
    goto drop;

  gint64 now = g_get_monotonic_time ();
  if (app.headless) {
    if (!app.committed)
      app.first_commit_us = now;
    app.last_commit_us = now;
    app.committed++;
    if (gst_is_dmabuf_memory (gst_buffer_peek_memory (buf, 0)))
      app.dma_frames++;
    else
      app.shm_frames++;
    goto drop;
  }
  if (!app.configured)
    goto drop;

  GstVideoMeta *meta = gst_buffer_get_video_meta (buf);
  GstVideoCropMeta *crop = gst_buffer_get_video_crop_meta (buf);
  int width = meta ? (int) meta->width : GST_VIDEO_INFO_WIDTH (&info);
  int height = meta ? (int) meta->height : GST_VIDEO_INFO_HEIGHT (&info);
  GstMemory *mem = gst_buffer_peek_memory (buf, 0);

  GstVideoFormat format = GST_VIDEO_INFO_FORMAT (&info);
  struct wl_buffer *wbuf = NULL;
  if (gst_is_dmabuf_memory (mem) && app.dmabuf && (format == GST_VIDEO_FORMAT_BGRx || format == GST_VIDEO_FORMAT_NV12)) {
    /* Zero copy: the planes live in one dmabuf; the offsets and strides come from the decoder. */
    int fd = gst_dmabuf_memory_get_fd (mem);
    int planes = format == GST_VIDEO_FORMAT_NV12 ? 2 : 1;
    struct zwp_linux_buffer_params_v1 *params = zwp_linux_dmabuf_v1_create_params (app.dmabuf);
    for (int i = 0; i < planes; i++)
      zwp_linux_buffer_params_v1_add (params, fd, i, mem->offset + (meta ? meta->offset[i] : info.offset[i]),
          meta ? meta->stride[i] : info.stride[i], MOD_INVALID_HI, MOD_INVALID_LO);
    frame = g_new0 (Frame, 1);
    frame->buffer = wbuf = zwp_linux_buffer_params_v1_create_immed (params, width, height,
        format == GST_VIDEO_FORMAT_NV12 ? DRM_NV12 : DRM_XRGB8888, 0);
    zwp_linux_buffer_params_v1_destroy (params);
    if (!wbuf)
      goto drop;
    frame->sample = gst_sample_ref (sample);
    wl_buffer_add_listener (wbuf, &buffer_listener, frame);
    app.dma_frames++;
  } else if (format == GST_VIDEO_FORMAT_BGRx) {
    GstMapInfo map;
    ShmSlot *slot = shm_slot_get (width, height);
    if (!slot || !gst_buffer_map (buf, &map, GST_MAP_READ))
      goto drop;
    gint stride = meta ? meta->stride[0] : info.stride[0];
    gsize off = meta ? meta->offset[0] : 0;
    for (int y = 0; y < height; y++)
      memcpy ((guint8 *) slot->data + (gsize) y * width * 4, map.data + off + (gsize) y * stride, (gsize) width * 4);
    gst_buffer_unmap (buf, &map);
    slot->busy = TRUE;
    wbuf = slot->buffer;
    app.shm_frames++;
  } else {
    static gboolean warned;
    if (!warned)
      g_printerr ("shabnam-video: cannot show %s frames\n", gst_video_format_to_string (format));
    warned = TRUE;
    goto drop;
  }
  frame = NULL;                 /* owned by the buffer's release event from here on */

  int sx = crop ? (int) crop->x : 0, sy = crop ? (int) crop->y : 0;
  int sw = crop ? (int) crop->width : GST_VIDEO_INFO_WIDTH (&info), sh = crop ? (int) crop->height : GST_VIDEO_INFO_HEIGHT (&info);
  sw = MIN (sw, width - sx);
  sh = MIN (sh, height - sy);
  int pn = GST_VIDEO_INFO_PAR_N (&info) ? GST_VIDEO_INFO_PAR_N (&info) : 1;
  int pd = GST_VIDEO_INFO_PAR_D (&info) ? GST_VIDEO_INFO_PAR_D (&info) : 1;
  if (sx != app.src_x || sy != app.src_y || sw != app.src_w || sh != app.src_h || pn != app.par_n || pd != app.par_d) {
    app.src_x = sx; app.src_y = sy; app.src_w = sw; app.src_h = sh; app.par_n = pn; app.par_d = pd;
    app.layout_dirty = TRUE;
  }
  gboolean relayout = app.layout_dirty || !app.video_mapped;
  if (relayout)
    layout ();

  wl_surface_attach (app.video, wbuf, 0, 0);
  wl_surface_damage_buffer (app.video, 0, 0, width, height);
  if (app.presentation && app.stats) {
    struct wp_presentation_feedback *fb = wp_presentation_feedback (app.presentation, app.video);
    wp_presentation_feedback_add_listener (fb, &feedback_listener, NULL);
  }
  wl_surface_commit (app.video);
  if (relayout) {
    wl_surface_commit (app.bg);   /* the subsurface position is applied with the parent */
    app.video_mapped = TRUE;
  }
  wl_display_flush (app.display);
  if (!app.committed)
    app.first_commit_us = now;
  app.last_commit_us = now;
  app.committed++;
  gst_sample_unref (sample);
  return G_SOURCE_REMOVE;

drop:
  if (frame)
    frame_free (frame);
  gst_sample_unref (sample);
  return G_SOURCE_REMOVE;
}

static GstFlowReturn
on_new_sample (GstAppSink *sink, gpointer data)
{
  GstSample *sample = gst_app_sink_pull_sample (sink);
  if (sample)
    g_main_context_invoke_full (NULL, G_PRIORITY_HIGH, show_sample, sample, NULL);
  return GST_FLOW_OK;
}

static GstFlowReturn
on_new_preroll (GstAppSink *sink, gpointer data)
{
  GstSample *sample = gst_app_sink_pull_preroll (sink);
  if (sample)
    g_main_context_invoke_full (NULL, G_PRIORITY_HIGH, show_sample, sample, NULL);
  return GST_FLOW_OK;
}

/* ---------------------------------------------------------------- controls */

static void
apply_state (void)
{
  gst_element_set_state (app.pipeline, (app.paused || app.buffering) ? GST_STATE_PAUSED : GST_STATE_PLAYING);
}

static void
toggle_pause (void)
{
  app.paused = !app.paused;
  apply_state ();
  bar_show ();
}

static void
seek_by (int seconds)
{
  gint64 pos, dur = -1;
  if (!gst_element_query_position (app.pipeline, GST_FORMAT_TIME, &pos))
    return;
  gst_element_query_duration (app.pipeline, GST_FORMAT_TIME, &dur);
  pos += (gint64) seconds * GST_SECOND;
  if (dur > 0 && pos > dur - GST_SECOND)
    pos = dur - GST_SECOND;
  if (pos < 0)
    pos = 0;
  /* Jump to a key frame, and to one on the far side of the target, so a step always moves. */
  gst_element_seek_simple (app.pipeline, GST_FORMAT_TIME, GST_SEEK_FLAG_FLUSH | GST_SEEK_FLAG_KEY_UNIT |
      (seconds > 0 ? GST_SEEK_FLAG_SNAP_AFTER : GST_SEEK_FLAG_SNAP_BEFORE), pos);
  bar_show ();
}

static void
volume_by (double step)
{
  double v;
  if (!app.volume)
    return;
  g_object_get (app.volume, "volume", &v, NULL);
  v = CLAMP (v + step, 0.0, 1.5);
  g_object_set (app.volume, "volume", v, NULL);
}

static void
toggle_fullscreen (void)
{
  if (!app.toplevel)
    return;
  if (app.fullscreen)
    xdg_toplevel_unset_fullscreen (app.toplevel);
  else
    xdg_toplevel_set_fullscreen (app.toplevel, NULL);
  wl_display_flush (app.display);
}

static void
quit (int code)
{
  app.exit_code = code;
  g_main_loop_quit (app.loop);
}

/* ---------------------------------------------------------------- input */

static void
kb_keymap (void *data, struct wl_keyboard *kb, uint32_t format, int32_t fd, uint32_t size)
{
  close (fd);
}

static void
kb_enter (void *data, struct wl_keyboard *kb, uint32_t serial, struct wl_surface *s, struct wl_array *keys)
{
}

static void
kb_leave (void *data, struct wl_keyboard *kb, uint32_t serial, struct wl_surface *s)
{
  app.shift = FALSE;
}

static void
kb_key (void *data, struct wl_keyboard *kb, uint32_t serial, uint32_t time, uint32_t key, uint32_t state)
{
  if (state != WL_KEYBOARD_KEY_STATE_PRESSED)
    return;
  switch (key) {
    case K_SPACE: case K_K: toggle_pause (); break;
    case K_LEFT: seek_by (app.shift ? -60 : -10); break;
    case K_RIGHT: seek_by (app.shift ? 60 : 10); break;
    case K_UP: volume_by (0.1); break;
    case K_DOWN: volume_by (-0.1); break;
    case K_M:
      app.muted = !app.muted;
      if (app.volume)
        g_object_set (app.volume, "mute", app.muted, NULL);
      break;
    case K_F: case K_F11: toggle_fullscreen (); break;
    case K_ESC:
      if (app.fullscreen)
        toggle_fullscreen ();
      break;
    case K_Q: quit (0); break;
  }
}

static void
kb_modifiers (void *data, struct wl_keyboard *kb, uint32_t serial, uint32_t depressed, uint32_t latched,
    uint32_t locked, uint32_t group)
{
  app.shift = (depressed & 1) != 0;     /* Shift is the first modifier in every xkb keymap */
}

static void
kb_repeat (void *data, struct wl_keyboard *kb, int32_t rate, int32_t delay)
{
}

static const struct wl_keyboard_listener keyboard_listener = {
  kb_keymap, kb_enter, kb_leave, kb_key, kb_modifiers, kb_repeat
};

static void
ptr_enter (void *data, struct wl_pointer *p, uint32_t serial, struct wl_surface *s, wl_fixed_t x, wl_fixed_t y)
{
}

static void
ptr_leave (void *data, struct wl_pointer *p, uint32_t serial, struct wl_surface *s)
{
}

static void
ptr_motion (void *data, struct wl_pointer *p, uint32_t time, wl_fixed_t x, wl_fixed_t y)
{
}

static void
ptr_button (void *data, struct wl_pointer *p, uint32_t serial, uint32_t time, uint32_t button, uint32_t state)
{
  if (button != 0x110 || state != WL_POINTER_BUTTON_STATE_PRESSED)      /* BTN_LEFT */
    return;
  if (app.last_click && time - app.last_click < 400) {
    toggle_pause ();            /* undo the pause of the first click */
    toggle_fullscreen ();
    app.last_click = 0;
  } else {
    toggle_pause ();
    app.last_click = time;
  }
}

static void
ptr_axis (void *data, struct wl_pointer *p, uint32_t time, uint32_t axis, wl_fixed_t value)
{
}

static void
ptr_frame (void *data, struct wl_pointer *p)
{
}

static void
ptr_axis_source (void *data, struct wl_pointer *p, uint32_t source)
{
}

static void
ptr_axis_stop (void *data, struct wl_pointer *p, uint32_t time, uint32_t axis)
{
}

static void
ptr_axis_discrete (void *data, struct wl_pointer *p, uint32_t axis, int32_t discrete)
{
}

static const struct wl_pointer_listener pointer_listener = {
  ptr_enter, ptr_leave, ptr_motion, ptr_button, ptr_axis, ptr_frame, ptr_axis_source, ptr_axis_stop,
  ptr_axis_discrete
};

static void
seat_capabilities (void *data, struct wl_seat *seat, uint32_t caps)
{
  if ((caps & WL_SEAT_CAPABILITY_KEYBOARD) && !app.keyboard) {
    app.keyboard = wl_seat_get_keyboard (seat);
    wl_keyboard_add_listener (app.keyboard, &keyboard_listener, NULL);
  }
  if ((caps & WL_SEAT_CAPABILITY_POINTER) && !app.pointer) {
    app.pointer = wl_seat_get_pointer (seat);
    wl_pointer_add_listener (app.pointer, &pointer_listener, NULL);
  }
}

static void
seat_name (void *data, struct wl_seat *seat, const char *name)
{
}

static const struct wl_seat_listener seat_listener = { seat_capabilities, seat_name };

/* ---------------------------------------------------------------- window */

static void
wm_ping (void *data, struct xdg_wm_base *wm, uint32_t serial)
{
  xdg_wm_base_pong (wm, serial);
}

static const struct xdg_wm_base_listener wm_listener = { wm_ping };

static void
xdg_surface_configure (void *data, struct xdg_surface *xs, uint32_t serial)
{
  xdg_surface_ack_configure (xs, serial);
  int w = app.pending_w > 0 ? app.pending_w : (app.win_w > 0 ? app.win_w : 960);
  int h = app.pending_h > 0 ? app.pending_h : (app.win_h > 0 ? app.win_h : 540);
  gboolean first = !app.configured;
  app.configured = TRUE;
  if (first || w != app.win_w || h != app.win_h) {
    app.win_w = w;
    app.win_h = h;
    layout ();
    if (app.video_mapped)
      wl_surface_commit (app.video);    /* new destination size for the picture */
    if (first) {
      wl_surface_attach (app.bg, app.black, 0, 0);
      wl_surface_damage_buffer (app.bg, 0, 0, 1, 1);
    }
    bar_update ();
  }
  wl_surface_commit (app.bg);
  wl_display_flush (app.display);
}

static const struct xdg_surface_listener xdg_surface_listener = { xdg_surface_configure };

static void
toplevel_configure (void *data, struct xdg_toplevel *t, int32_t w, int32_t h, struct wl_array *states)
{
  uint32_t *s;
  app.pending_w = w;
  app.pending_h = h;
  app.fullscreen = FALSE;
  wl_array_for_each (s, states)
    if (*s == XDG_TOPLEVEL_STATE_FULLSCREEN)
      app.fullscreen = TRUE;
}

static void
toplevel_close (void *data, struct xdg_toplevel *t)
{
  quit (0);
}

static void
toplevel_bounds (void *data, struct xdg_toplevel *t, int32_t w, int32_t h)
{
}

static void
toplevel_caps (void *data, struct xdg_toplevel *t, struct wl_array *caps)
{
}

static const struct xdg_toplevel_listener toplevel_listener = {
  toplevel_configure, toplevel_close, toplevel_bounds, toplevel_caps
};

static void
registry_global (void *data, struct wl_registry *r, uint32_t name, const char *iface, uint32_t version)
{
  if (!strcmp (iface, wl_compositor_interface.name))
    app.compositor = wl_registry_bind (r, name, &wl_compositor_interface, MIN (version, 4));
  else if (!strcmp (iface, wl_subcompositor_interface.name))
    app.subcompositor = wl_registry_bind (r, name, &wl_subcompositor_interface, 1);
  else if (!strcmp (iface, wl_shm_interface.name))
    app.shm = wl_registry_bind (r, name, &wl_shm_interface, 1);
  else if (!strcmp (iface, wl_seat_interface.name) && !app.seat) {
    app.seat = wl_registry_bind (r, name, &wl_seat_interface, MIN (version, 5));
    wl_seat_add_listener (app.seat, &seat_listener, NULL);
  } else if (!strcmp (iface, xdg_wm_base_interface.name)) {
    app.wm_base = wl_registry_bind (r, name, &xdg_wm_base_interface, MIN (version, 4));
    xdg_wm_base_add_listener (app.wm_base, &wm_listener, NULL);
  } else if (!strcmp (iface, wp_viewporter_interface.name))
    app.viewporter = wl_registry_bind (r, name, &wp_viewporter_interface, 1);
  else if (!strcmp (iface, zwp_linux_dmabuf_v1_interface.name) && version >= 2)
    app.dmabuf = wl_registry_bind (r, name, &zwp_linux_dmabuf_v1_interface, MIN (version, 3));
  else if (!strcmp (iface, wp_presentation_interface.name))
    app.presentation = wl_registry_bind (r, name, &wp_presentation_interface, 1);
}

static void
registry_remove (void *data, struct wl_registry *r, uint32_t name)
{
}

static const struct wl_registry_listener registry_listener = { registry_global, registry_remove };

static gboolean
wayland_readable (gint fd, GIOCondition cond, gpointer data)
{
  if ((cond & (G_IO_ERR | G_IO_HUP)) || wl_display_dispatch (app.display) < 0) {
    g_printerr ("shabnam-video: lost the display connection (%s)\n", g_strerror (errno));
    quit (1);
    return G_SOURCE_REMOVE;
  }
  wl_display_flush (app.display);
  return G_SOURCE_CONTINUE;
}

static struct wl_surface *
sub_surface (struct wl_subsurface **sub, struct wp_viewport **vp)
{
  struct wl_surface *s = wl_compositor_create_surface (app.compositor);
  struct wl_region *empty = wl_compositor_create_region (app.compositor);
  wl_surface_set_input_region (s, empty);       /* all input goes to the window surface */
  wl_region_destroy (empty);
  *sub = wl_subcompositor_get_subsurface (app.subcompositor, s, app.bg);
  wl_subsurface_set_desync (*sub);
  *vp = wp_viewporter_get_viewport (app.viewporter, s);
  return s;
}

static gboolean
window_open (const char *title)
{
  app.display = wl_display_connect (NULL);
  if (!app.display) {
    g_printerr ("shabnam-video: no Wayland display\n");
    return FALSE;
  }
  struct wl_registry *registry = wl_display_get_registry (app.display);
  wl_registry_add_listener (registry, &registry_listener, NULL);
  wl_display_roundtrip (app.display);
  if (!app.compositor || !app.subcompositor || !app.shm || !app.wm_base || !app.viewporter) {
    g_printerr ("shabnam-video: the compositor lacks a required interface\n");
    return FALSE;
  }
  app.black = solid_buffer (0xff000000);
  app.dark = solid_buffer (0xcc061a33);
  app.teal = solid_buffer (0xff12c8c4);

  app.bg = wl_compositor_create_surface (app.compositor);
  app.bg_vp = wp_viewporter_get_viewport (app.viewporter, app.bg);
  app.video = sub_surface (&app.video_sub, &app.video_vp);
  app.bar = sub_surface (&app.bar_sub, &app.bar_vp);
  app.fill = sub_surface (&app.fill_sub, &app.fill_vp);

  app.xdg_surface = xdg_wm_base_get_xdg_surface (app.wm_base, app.bg);
  xdg_surface_add_listener (app.xdg_surface, &xdg_surface_listener, NULL);
  app.toplevel = xdg_surface_get_toplevel (app.xdg_surface);
  xdg_toplevel_add_listener (app.toplevel, &toplevel_listener, NULL);
  xdg_toplevel_set_app_id (app.toplevel, "shabnam-player");
  xdg_toplevel_set_title (app.toplevel, title);
  wl_surface_commit (app.bg);
  wl_display_roundtrip (app.display);
  g_unix_fd_add (wl_display_get_fd (app.display), G_IO_IN | G_IO_ERR | G_IO_HUP, wayland_readable, NULL);
  return TRUE;
}

/* ---------------------------------------------------------------- pipeline */

static void
link_to (GstPad *pad, GstElement *first)
{
  GstPad *sink = gst_element_get_static_pad (first, "sink");
  if (gst_pad_link (pad, sink) != GST_PAD_LINK_OK)
    g_printerr ("shabnam-video: cannot link a decoded stream\n");
  gst_object_unref (sink);
}

static void
on_pad_added (GstElement *dec, GstPad *pad, gpointer data)
{
  GstCaps *caps = gst_pad_get_current_caps (pad);
  if (!caps)
    caps = gst_pad_query_caps (pad, NULL);
  const char *name = gst_structure_get_name (gst_caps_get_structure (caps, 0));
  if (g_str_has_prefix (name, "video/") && !app.have_video) {
    app.have_video = TRUE;
    link_to (pad, app.vconv);
  } else if (g_str_has_prefix (name, "audio/") && !app.have_audio && app.aconv) {
    app.have_audio = TRUE;
    link_to (pad, app.aconv);
  } else {
    /* a second sound track, subtitles: not shown, but they must go somewhere */
    GstElement *sink = gst_element_factory_make ("fakesink", NULL);
    g_object_set (sink, "async", FALSE, "sync", FALSE, NULL);
    gst_bin_add (GST_BIN (app.pipeline), sink);
    gst_element_sync_state_with_parent (sink);
    link_to (pad, sink);
  }
  gst_caps_unref (caps);
}

static void
on_source_setup (GstElement *dec, GstElement *source, gpointer data)
{
  GObjectClass *klass = G_OBJECT_GET_CLASS (source);
  if (app.user_agent && g_object_class_find_property (klass, "user-agent"))
    g_object_set (source, "user-agent", app.user_agent, NULL);
  if (app.referer && g_object_class_find_property (klass, "extra-headers")) {
    GstStructure *h = gst_structure_new ("headers", "Referer", G_TYPE_STRING, app.referer, NULL);
    g_object_set (source, "extra-headers", h, NULL);
    gst_structure_free (h);
  }
}

static GstElement *
add_source (const char *uri)
{
  GstElement *dec = gst_element_factory_make ("uridecodebin", NULL);
  g_object_set (dec, "uri", uri, NULL);
  g_signal_connect (dec, "pad-added", G_CALLBACK (on_pad_added), NULL);
  g_signal_connect (dec, "source-setup", G_CALLBACK (on_source_setup), NULL);
  gst_bin_add (GST_BIN (app.pipeline), dec);
  return dec;
}

static GstElement *
must_make (const char *factory)
{
  GstElement *e = gst_element_factory_make (factory, NULL);
  if (!e) {
    g_printerr ("shabnam-video: GStreamer element %s is missing\n", factory);
    exit (1);
  }
  return e;
}

static void
build_pipeline (const char *uri)
{
  app.pipeline = gst_pipeline_new ("player");

  /* picture: decoder -> (conversion only for software decoders) -> this program */
  GstElement *vq = must_make ("queue");
  app.vconv = must_make ("videoconvert");
  app.appsink = must_make ("appsink");
  GstCaps *caps = gst_caps_from_string ("video/x-raw(memory:DMABuf), format={ BGRx, NV12 }; video/x-raw, format=BGRx");
  GstAppSinkCallbacks cb = { .new_preroll = on_new_preroll, .new_sample = on_new_sample };
  g_object_set (app.appsink, "caps", caps, "max-buffers", 2, "qos", TRUE, "max-lateness", (gint64) 20 * GST_MSECOND,
      "enable-last-sample", FALSE, NULL);
  gst_caps_unref (caps);
  gst_app_sink_set_callbacks (GST_APP_SINK (app.appsink), &cb, NULL, NULL);
  g_object_set (vq, "max-size-buffers", 4, "max-size-bytes", 0, "max-size-time", (guint64) 0, NULL);
  gst_bin_add_many (GST_BIN (app.pipeline), app.vconv, vq, app.appsink, NULL);
  gst_element_link_many (app.vconv, vq, app.appsink, NULL);

  /* sound */
  gboolean silent = app.headless && !g_getenv ("SHABNAM_VIDEO_TEST_AUDIO");
  GstElement *asink = gst_element_factory_make (silent ? "fakesink" : "pulsesink", NULL);
  if (!asink)
    asink = gst_element_factory_make ("autoaudiosink", NULL);
  if (asink) {
    GstElement *aq = must_make ("queue"), *ares = must_make ("audioresample");
    app.aconv = must_make ("audioconvert");
    app.volume = must_make ("volume");
    /* The picture follows the system clock, not the sound server's: through pipewire-pulse that clock
     * crawls for the first seconds of a stream and can stay stopped after a seek made while paused, and the
     * picture would crawl or freeze with it. The sound is nudged to follow the picture instead. */
    if (g_object_class_find_property (G_OBJECT_GET_CLASS (asink), "provide-clock"))
      g_object_set (asink, "provide-clock", FALSE, NULL);
    if (silent)
      g_object_set (asink, "sync", TRUE, NULL);
    else if (app.headless)
      g_object_set (app.volume, "mute", TRUE, NULL);
    gst_bin_add_many (GST_BIN (app.pipeline), app.aconv, ares, app.volume, aq, asink, NULL);
    gst_element_link_many (app.aconv, ares, app.volume, aq, asink, NULL);
  }

  add_source (uri);
  if (app.audio_uri)
    add_source (app.audio_uri);
}

static void
print_stats (const char *why)
{
  GstStructure *st = NULL;
  guint64 rendered = 0, dropped = 0;
  g_object_get (app.appsink, "stats", &st, NULL);
  if (st) {
    gst_structure_get_uint64 (st, "rendered", &rendered);
    gst_structure_get_uint64 (st, "dropped", &dropped);
    gst_structure_free (st);
  }
  double secs = (app.last_commit_us - app.first_commit_us) / 1e6;
  g_printerr ("stats(%s): shown=%" G_GUINT64_FORMAT " (dmabuf=%" G_GUINT64_FORMAT " shm=%" G_GUINT64_FORMAT
      ") over %.2fs = %.2f fps; sink dropped=%" G_GUINT64_FORMAT " rendered=%" G_GUINT64_FORMAT
      "; compositor presented=%" G_GUINT64_FORMAT " discarded=%" G_GUINT64_FORMAT "\n",
      why, app.committed, app.dma_frames, app.shm_frames, secs, secs > 0 ? (app.committed - 1) / secs : 0.0,
      dropped, rendered, app.presented, app.discarded);
}

/* Picture and sound may come from two connections, each with its own buffer. Playback waits when one of
 * them has run dry and goes on when all are full again. The level of a buffer can drop to a few percent
 * for some milliseconds while a demuxer jumps around in the file; only a buffer that stays empty counts. */
static gboolean buffering_settled (gpointer data);

static void
buffering_check (gboolean settled)
{
  GHashTableIter it;
  gpointer level;
  gboolean low = FALSE, full = TRUE;
  g_hash_table_iter_init (&it, app.levels);
  while (g_hash_table_iter_next (&it, NULL, &level)) {
    low |= GPOINTER_TO_INT (level) < 10;
    full &= GPOINTER_TO_INT (level) >= 100;
  }
  if (!app.buffering && low) {
    if (!settled) {
      if (!app.buffering_timer)
        app.buffering_timer = g_timeout_add (400, buffering_settled, NULL);
      return;
    }
    app.buffering = TRUE;
  } else if (app.buffering && full) {
    app.buffering = FALSE;
  } else {
    return;
  }
  if (app.stats)
    g_printerr ("%s\n", app.buffering ? "buffering" : "buffered");
  apply_state ();
}

static gboolean
buffering_settled (gpointer data)
{
  app.buffering_timer = 0;
  buffering_check (TRUE);
  return G_SOURCE_REMOVE;
}

static gboolean
on_bus (GstBus *bus, GstMessage *msg, gpointer data)
{
  switch (GST_MESSAGE_TYPE (msg)) {
    case GST_MESSAGE_EOS:
      quit (0);
      break;
    case GST_MESSAGE_ERROR: {
      GError *err = NULL;
      gchar *dbg = NULL;
      gst_message_parse_error (msg, &err, &dbg);
      g_printerr ("shabnam-video: %s\n  %s\n", err->message, dbg ? dbg : "");
      g_clear_error (&err);
      g_free (dbg);
      quit (1);
      break;
    }
    case GST_MESSAGE_BUFFERING: {
      gint percent = 0;
      gst_message_parse_buffering (msg, &percent);
      if (g_getenv ("SHABNAM_VIDEO_TRACE"))
        g_printerr ("%.2f buffering %d%% (%s)\n", g_get_monotonic_time () / 1e6, percent, GST_OBJECT_NAME (GST_MESSAGE_SRC (msg)));
      if (!app.levels)
        app.levels = g_hash_table_new (NULL, NULL);
      g_hash_table_insert (app.levels, GST_MESSAGE_SRC (msg), GINT_TO_POINTER (percent));
      buffering_check (FALSE);
      break;
    }
    case GST_MESSAGE_ELEMENT:
      break;
    default:
      break;
  }
  return G_SOURCE_CONTINUE;
}

static gboolean
on_signal (gpointer data)
{
  quit (0);
  return G_SOURCE_CONTINUE;
}

/* --selftest STEPS: run the actions the keys would run, one every two seconds, without any input device.
 * STEPS is a string of letters: s seek forward, b seek back, p pause/resume, f fullscreen on/off, q quit. */
static gboolean
on_selftest (gpointer data)
{
  static const char *next;
  if (!next)
    next = data;
  gint64 pos = -1;
  gst_element_query_position (app.pipeline, GST_FORMAT_TIME, &pos);
  g_printerr ("selftest '%c' at %.1fs (fullscreen=%d window=%dx%d)\n", *next, pos / 1e9, app.fullscreen, app.win_w, app.win_h);
  switch (*next) {
    case 's': seek_by (10); break;
    case 'b': seek_by (-10); break;
    case 'p': toggle_pause (); break;
    case 'f': toggle_fullscreen (); break;
    case 'q': case 0: quit (0); return G_SOURCE_REMOVE;
  }
  next++;
  return G_SOURCE_CONTINUE;
}

static gboolean
on_stats_tick (gpointer data)
{
  print_stats ("tick");
  return G_SOURCE_CONTINUE;
}

int
main (int argc, char **argv)
{
  const char *target = NULL, *selftest = NULL;
  for (int i = 1; i < argc; i++) {
    const char *a = argv[i];
    if (!strcmp (a, "--audio") && i + 1 < argc) app.audio_uri = argv[++i];
    else if (!strcmp (a, "--title") && i + 1 < argc) app.title = argv[++i];
    else if (!strcmp (a, "--user-agent") && i + 1 < argc) app.user_agent = argv[++i];
    else if (!strcmp (a, "--referer") && i + 1 < argc) app.referer = argv[++i];
    else if (!strcmp (a, "--sw")) app.sw = TRUE;
    else if (!strcmp (a, "--stats")) app.stats = TRUE;
    else if (!strcmp (a, "--headless")) app.headless = TRUE;
    else if (!strcmp (a, "--selftest") && i + 1 < argc) selftest = argv[++i];
    else if (a[0] == '-' && a[1] == '-') {
      g_printerr ("usage: shabnam-video [--audio URI] [--title T] [--user-agent UA] [--referer R] [--sw] [--stats] [--headless] FILE-or-URI\n");
      return 2;
    } else target = a;
  }
  if (!target) {
    g_printerr ("shabnam-video: nothing to play\n");
    return 2;
  }

  /* The hardware decoder labels its frames as dmabuf, so nothing in between tries to touch the pixels. */
  g_setenv ("GST_MPP_DEC_DMA_FEATURE", "1", TRUE);
  if (!g_getenv ("SHABNAM_VIDEO_NV12"))
    g_setenv ("GST_MPP_VIDEODEC_DEFAULT_FORMAT", "BGRx", TRUE);
  if (app.sw)
    g_setenv ("GST_PLUGIN_FEATURE_RANK", "mppvideodec:NONE", TRUE);
  gst_init (NULL, NULL);

  gchar *uri = gst_uri_is_valid (target) ? g_strdup (target) : gst_filename_to_uri (target, NULL);
  if (!uri) {
    g_printerr ("shabnam-video: cannot open %s\n", target);
    return 1;
  }
  gchar *base = app.title ? g_strdup (app.title) : g_path_get_basename (target);

  app.loop = g_main_loop_new (NULL, FALSE);
  app.par_n = app.par_d = 1;
  if (!app.headless && !window_open (base))
    return 1;
  build_pipeline (uri);

  GstBus *bus = gst_element_get_bus (app.pipeline);
  gst_bus_add_watch (bus, on_bus, NULL);
  g_unix_signal_add (SIGINT, on_signal, NULL);
  g_unix_signal_add (SIGTERM, on_signal, NULL);
  if (app.stats)
    g_timeout_add_seconds (5, on_stats_tick, NULL);
  if (selftest)
    g_timeout_add_seconds (2, on_selftest, (gpointer) selftest);

  gst_element_set_state (app.pipeline, GST_STATE_PLAYING);
  g_main_loop_run (app.loop);

  if (app.stats)
    print_stats ("end");
  gst_element_set_state (app.pipeline, GST_STATE_NULL);
  gst_object_unref (bus);
  gst_object_unref (app.pipeline);
  if (app.display) {
    wl_display_roundtrip (app.display);
    wl_display_disconnect (app.display);
  }
  g_free (uri);
  g_free (base);
  return app.exit_code;
}
