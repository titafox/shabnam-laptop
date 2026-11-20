/* va_export_egl — off-screen check of what Chromium's VA-API decoder needs from this machine.
 * 1. creates a 1920x1080 NV12 VA surface and exports it the way Chromium does
 *    (DRM PRIME 2, READ_ONLY | SEPARATE_LAYERS) and prints the descriptor;
 * 2. fills the buffer with a known colour;
 * 3. opens EGL without a window (GBM device, surfaceless or pbuffer), prints the extensions that matter,
 *    and imports the buffer as an EGLImage in every way Chromium might (NV12 two-plane with and without a
 *    modifier, R8 and GR88 per plane), samples each and reads the pixel back.
 * usage: va_export_egl [render node] [gbm node]            (no window is created)
 */
#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <fcntl.h>
#include <unistd.h>
#include <dlfcn.h>
#include <sys/mman.h>
#include <va/va.h>
#include <va/va_drm.h>
#include <va/va_drmcommon.h>
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <GLES2/gl2.h>
#include <GLES2/gl2ext.h>

#define FCC(a,b,c,d) ((uint32_t)(a)|((uint32_t)(b)<<8)|((uint32_t)(c)<<16)|((uint32_t)(d)<<24))
#define F_NV12 FCC('N','V','1','2')
#define F_R8   FCC('R','8',' ',' ')
#define F_GR88 FCC('G','R','8','8')

/* EGL / GLES entry points are taken from the Mali library at run time (no link-time dependency). */
#define FUNCS \
 X(EGLDisplay, eglGetDisplay, (EGLNativeDisplayType)) \
 X(EGLBoolean, eglInitialize, (EGLDisplay, EGLint*, EGLint*)) \
 X(const char*, eglQueryString, (EGLDisplay, EGLint)) \
 X(EGLBoolean, eglBindAPI, (EGLenum)) \
 X(EGLBoolean, eglChooseConfig, (EGLDisplay, const EGLint*, EGLConfig*, EGLint, EGLint*)) \
 X(EGLContext, eglCreateContext, (EGLDisplay, EGLConfig, EGLContext, const EGLint*)) \
 X(EGLSurface, eglCreatePbufferSurface, (EGLDisplay, EGLConfig, const EGLint*)) \
 X(EGLBoolean, eglMakeCurrent, (EGLDisplay, EGLSurface, EGLSurface, EGLContext)) \
 X(EGLint, eglGetError, (void)) \
 X(const GLubyte*, glGetString, (GLenum)) \
 X(GLenum, glGetError, (void)) \
 X(void, glGenTextures, (GLsizei, GLuint*)) \
 X(void, glBindTexture, (GLenum, GLuint)) \
 X(void, glTexParameteri, (GLenum, GLenum, GLint)) \
 X(void, glTexImage2D, (GLenum, GLint, GLint, GLsizei, GLsizei, GLint, GLenum, GLenum, const void*)) \
 X(void, glGenFramebuffers, (GLsizei, GLuint*)) \
 X(void, glBindFramebuffer, (GLenum, GLuint)) \
 X(void, glFramebufferTexture2D, (GLenum, GLenum, GLenum, GLuint, GLint)) \
 X(GLenum, glCheckFramebufferStatus, (GLenum)) \
 X(void, glViewport, (GLint, GLint, GLsizei, GLsizei)) \
 X(GLuint, glCreateShader, (GLenum)) \
 X(void, glShaderSource, (GLuint, GLsizei, const GLchar*const*, const GLint*)) \
 X(void, glCompileShader, (GLuint)) \
 X(void, glGetShaderiv, (GLuint, GLenum, GLint*)) \
 X(void, glGetShaderInfoLog, (GLuint, GLsizei, GLsizei*, GLchar*)) \
 X(GLuint, glCreateProgram, (void)) \
 X(void, glAttachShader, (GLuint, GLuint)) \
 X(void, glBindAttribLocation, (GLuint, GLuint, const GLchar*)) \
 X(void, glLinkProgram, (GLuint)) \
 X(void, glUseProgram, (GLuint)) \
 X(void, glVertexAttribPointer, (GLuint, GLint, GLenum, GLboolean, GLsizei, const void*)) \
 X(void, glEnableVertexAttribArray, (GLuint)) \
 X(void, glDrawArrays, (GLenum, GLint, GLsizei)) \
 X(void, glReadPixels, (GLint, GLint, GLsizei, GLsizei, GLenum, GLenum, void*)) \
 X(void, glClearColor, (GLfloat, GLfloat, GLfloat, GLfloat)) \
 X(void, glClear, (GLbitfield)) \
 X(void, glFinish, (void))
#define X(r,n,a) static r (*p_##n) a;
FUNCS
#undef X
static void *(*p_eglGetProcAddress)(const char *);
static EGLImageKHR (*p_eglCreateImageKHR)(EGLDisplay, EGLContext, EGLenum, EGLClientBuffer, const EGLint *);
static EGLBoolean (*p_eglDestroyImageKHR)(EGLDisplay, EGLImageKHR);
static void (*p_glEGLImageTargetTexture2DOES)(GLenum, void *);
static EGLBoolean (*p_eglQueryDmaBufFormatsEXT)(EGLDisplay, EGLint, EGLint *, EGLint *);
static EGLBoolean (*p_eglQueryDmaBufModifiersEXT)(EGLDisplay, EGLint, EGLint, EGLuint64KHR *, EGLBoolean *, EGLint *);
static void *(*p_gbm_create_device)(int);

static const char *fcc(uint32_t f) { static char b[4][5]; static int i; i = (i + 1) & 3; memcpy(b[i], &f, 4); b[i][4] = 0; return b[i]; }
static int has(const char *list, const char *e) {
  size_t n = strlen(e); const char *p = list;
  while (p && (p = strstr(p, e))) { if ((p == list || p[-1] == ' ') && (p[n] == ' ' || !p[n])) return 1; p += n; }
  return 0;
}
static void yn(const char *list, const char *e) { printf("  %-46s %s\n", e, has(list ? list : "", e) ? "yes" : "NO"); }

static GLuint prog(const char *fs) {
  static const char *vs = "attribute vec2 p; varying vec2 t; void main(){ t=p*0.5+0.5; gl_Position=vec4(p,0.,1.); }";
  GLuint v = p_glCreateShader(GL_VERTEX_SHADER), f = p_glCreateShader(GL_FRAGMENT_SHADER), pr; GLint ok = 0;
  p_glShaderSource(v, 1, &vs, NULL); p_glCompileShader(v);
  p_glShaderSource(f, 1, &fs, NULL); p_glCompileShader(f);
  p_glGetShaderiv(f, GL_COMPILE_STATUS, &ok);
  if (!ok) { char log[512] = ""; p_glGetShaderInfoLog(f, sizeof log, NULL, log); printf("    shader compile failed: %s\n", log); return 0; }
  pr = p_glCreateProgram(); p_glAttachShader(pr, v); p_glAttachShader(pr, f); p_glBindAttribLocation(pr, 0, "p"); p_glLinkProgram(pr);
  return pr;
}

/* imports one EGLImage, samples it at the centre, prints the pixel */
static void try_import(EGLDisplay dpy, const char *label, GLenum target, const char *fs, const EGLint *attr) {
  EGLImageKHR img = p_eglCreateImageKHR(dpy, EGL_NO_CONTEXT, EGL_LINUX_DMA_BUF_EXT, NULL, attr);
  if (img == EGL_NO_IMAGE_KHR) { printf("  %-44s eglCreateImage FAILED (EGL error 0x%x)\n", label, p_eglGetError()); return; }
  GLuint tex; p_glGenTextures(1, &tex); p_glBindTexture(target, tex);
  p_glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST); p_glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
  while (p_glGetError()) ;
  p_glEGLImageTargetTexture2DOES(target, img);
  GLenum e = p_glGetError();
  if (e) { printf("  %-44s image ok, glEGLImageTargetTexture2DOES FAILED (GL error 0x%x)\n", label, e); p_eglDestroyImageKHR(dpy, img); return; }
  GLuint pr = prog(fs);
  if (!pr) { printf("  %-44s image+texture ok, shader failed\n", label); return; }
  static const GLfloat q[] = { -1, -1, 1, -1, -1, 1, 1, 1 };
  p_glUseProgram(pr); p_glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE, 0, q); p_glEnableVertexAttribArray(0);
  p_glClearColor(0, 0, 1, 1); p_glClear(GL_COLOR_BUFFER_BIT);
  p_glDrawArrays(GL_TRIANGLE_STRIP, 0, 4); p_glFinish();
  unsigned char px[4] = { 0 }; p_glReadPixels(32, 32, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px);
  printf("  %-44s OK  sampled pixel = %u,%u,%u,%u  (GL error 0x%x)\n", label, px[0], px[1], px[2], px[3], p_glGetError());
  p_eglDestroyImageKHR(dpy, img);
}

int main(int argc, char **argv) {
  const char *node = argc > 1 ? argv[1] : "/dev/dri/renderD128";
  const char *gbmnode = argc > 2 ? argv[2] : "/dev/dri/card0";
  setvbuf(stdout, NULL, _IOLBF, 0);

  /* ---- 1. VA surface + export ---- */
  int fd = open(node, O_RDWR | O_CLOEXEC);
  if (fd < 0) { perror(node); return 1; }
  VADisplay va = vaGetDisplayDRM(fd); int maj, min;
  VAStatus st = vaInitialize(va, &maj, &min);
  if (st) { printf("vaInitialize: %s\n", vaErrorStr(st)); return 1; }
  printf("VA %d.%d  %s\n", maj, min, vaQueryVendorString(va));
  VASurfaceID sid;
  VASurfaceAttrib at = { .type = VASurfaceAttribPixelFormat, .flags = VA_SURFACE_ATTRIB_SETTABLE,
                         .value = { .type = VAGenericValueTypeInteger, .value.i = VA_FOURCC_NV12 } };
  st = vaCreateSurfaces(va, VA_RT_FORMAT_YUV420, 1920, 1080, &sid, 1, &at, 1);
  if (st) { printf("vaCreateSurfaces: %s\n", vaErrorStr(st)); return 1; }
  VADRMPRIMESurfaceDescriptor d;
  st = vaSyncSurface(va, sid);
  st = vaExportSurfaceHandle(va, sid, VA_SURFACE_ATTRIB_MEM_TYPE_DRM_PRIME_2,
                             VA_EXPORT_SURFACE_READ_ONLY | VA_EXPORT_SURFACE_SEPARATE_LAYERS, &d);
  if (st) { printf("vaExportSurfaceHandle: %s\n", vaErrorStr(st)); return 1; }
  printf("export (READ_ONLY|SEPARATE_LAYERS): fourcc=%s %ux%u objects=%u layers=%u\n", fcc(d.fourcc), d.width, d.height, d.num_objects, d.num_layers);
  for (unsigned i = 0; i < d.num_objects; i++)
    printf("  object %u: fd=%d size=%u modifier=0x%llx\n", i, d.objects[i].fd, d.objects[i].size, (unsigned long long)d.objects[i].drm_format_modifier);
  for (unsigned i = 0; i < d.num_layers; i++)
    for (unsigned p = 0; p < d.layers[i].num_planes; p++)
      printf("  layer %u: drm_format=%s plane %u: object=%u offset=%u pitch=%u\n", i, fcc(d.layers[i].drm_format), p,
             d.layers[i].object_index[p], d.layers[i].offset[p], d.layers[i].pitch[p]);
  if (d.num_layers != 2 || d.num_objects != 1) { printf("unexpected layout\n"); return 1; }
  int bfd = d.objects[0].fd; uint32_t pitch = d.layers[0].pitch[0], uvoff = d.layers[1].offset[0], size = d.objects[0].size;
  /* red in BT.601 limited range: Y=81 Cb=90 Cr=240 */
  uint8_t *m = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, bfd, 0);
  if (m == MAP_FAILED) { perror("mmap of the exported buffer"); }
  else { memset(m, 81, uvoff); for (uint32_t i = uvoff; i + 1 < size; i += 2) { m[i] = 90; m[i + 1] = 240; } munmap(m, size);
         printf("  buffer filled through mmap: Y=81 Cb=90 Cr=240 (red)\n"); }

  /* ---- 2. EGL without a window ---- */
  void *lib = dlopen("libmali.so.1", RTLD_NOW | RTLD_GLOBAL);
  if (!lib) { printf("dlopen libmali.so.1: %s\n", dlerror()); return 1; }
#define X(r,n,a) if (!(p_##n = dlsym(lib, #n))) { printf("missing %s\n", #n); return 1; }
  FUNCS
#undef X
  p_eglGetProcAddress = dlsym(lib, "eglGetProcAddress");
  p_gbm_create_device = dlsym(lib, "gbm_create_device");
  p_eglCreateImageKHR = p_eglGetProcAddress("eglCreateImageKHR");
  p_eglDestroyImageKHR = p_eglGetProcAddress("eglDestroyImageKHR");
  p_glEGLImageTargetTexture2DOES = p_eglGetProcAddress("glEGLImageTargetTexture2DOES");
  p_eglQueryDmaBufFormatsEXT = p_eglGetProcAddress("eglQueryDmaBufFormatsEXT");
  p_eglQueryDmaBufModifiersEXT = p_eglGetProcAddress("eglQueryDmaBufModifiersEXT");

  const char *cext = p_eglQueryString(EGL_NO_DISPLAY, EGL_EXTENSIONS);
  printf("EGL client extensions: %s\n", cext ? cext : "(none)");
  int gfd = open(gbmnode, O_RDWR | O_CLOEXEC);
  void *gbm = (gfd >= 0 && p_gbm_create_device) ? p_gbm_create_device(gfd) : NULL;
  EGLDisplay (*getplat)(EGLenum, void *, const EGLint *) = p_eglGetProcAddress("eglGetPlatformDisplayEXT");
  EGLDisplay dpy = (gbm && getplat) ? getplat(EGL_PLATFORM_GBM_KHR, gbm, NULL) : EGL_NO_DISPLAY;
  if (dpy == EGL_NO_DISPLAY && gbm) dpy = p_eglGetDisplay((EGLNativeDisplayType)gbm);
  printf("gbm: open(%s)=%d%s gbm_create_device=%s device=%p\n", gbmnode, gfd, gfd < 0 ? " (failed)" : "", p_gbm_create_device ? "found" : "missing", gbm);
  printf("EGL display: %s\n", dpy != EGL_NO_DISPLAY ? gbmnode : "GBM failed, trying EGL_DEFAULT_DISPLAY");
  if (dpy == EGL_NO_DISPLAY) dpy = p_eglGetDisplay(EGL_DEFAULT_DISPLAY);
  EGLint emaj, emin;
  if (!p_eglInitialize(dpy, &emaj, &emin)) { printf("eglInitialize failed 0x%x\n", p_eglGetError()); return 1; }
  const char *ext = p_eglQueryString(dpy, EGL_EXTENSIONS);
  printf("EGL %d.%d vendor=%s version=%s\nEGL display extensions: %s\n", emaj, emin, p_eglQueryString(dpy, EGL_VENDOR), p_eglQueryString(dpy, EGL_VERSION), ext);
  yn(ext, "EGL_KHR_image_base"); yn(ext, "EGL_EXT_image_dma_buf_import"); yn(ext, "EGL_EXT_image_dma_buf_import_modifiers");
  yn(ext, "EGL_KHR_surfaceless_context"); yn(ext, "EGL_KHR_fence_sync"); yn(ext, "EGL_ANDROID_native_fence_sync");
  yn(ext, "EGL_KHR_wait_sync"); yn(ext, "EGL_EXT_yuv_surface");

  p_eglBindAPI(EGL_OPENGL_ES_API);
  EGLConfig cfg = 0; EGLint n = 0;
  EGLint ca[] = { EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8, EGL_NONE };
  if (!p_eglChooseConfig(dpy, ca, &cfg, 1, &n) || n < 1) {
    EGLint cb[] = { EGL_RENDERABLE_TYPE, EGL_OPENGL_ES2_BIT, EGL_NONE };
    if (!p_eglChooseConfig(dpy, cb, &cfg, 1, &n) || n < 1) { printf("no EGL config\n"); return 1; }
  }
  EGLint ctxa[] = { EGL_CONTEXT_CLIENT_VERSION, 2, EGL_NONE };
  EGLContext ctx = p_eglCreateContext(dpy, cfg, EGL_NO_CONTEXT, ctxa);
  if (ctx == EGL_NO_CONTEXT) { printf("eglCreateContext failed 0x%x\n", p_eglGetError()); return 1; }
  EGLSurface surf = EGL_NO_SURFACE;
  if (!p_eglMakeCurrent(dpy, EGL_NO_SURFACE, EGL_NO_SURFACE, ctx)) {
    EGLint pa[] = { EGL_WIDTH, 64, EGL_HEIGHT, 64, EGL_NONE };
    surf = p_eglCreatePbufferSurface(dpy, cfg, pa);
    if (!p_eglMakeCurrent(dpy, surf, surf, ctx)) { printf("eglMakeCurrent failed 0x%x\n", p_eglGetError()); return 1; }
    printf("context: pbuffer\n");
  } else printf("context: surfaceless\n");
  const char *gle = (const char *)p_glGetString(GL_EXTENSIONS);
  printf("GL_RENDERER=%s\nGL_VERSION=%s\n", p_glGetString(GL_RENDERER), p_glGetString(GL_VERSION));
  yn(gle, "GL_OES_EGL_image"); yn(gle, "GL_OES_EGL_image_external"); yn(gle, "GL_OES_EGL_image_external_essl3");
  yn(gle, "GL_EXT_YUV_target"); yn(gle, "GL_EXT_texture_rg"); yn(gle, "GL_OES_surfaceless_context"); yn(gle, "GL_EXT_EGL_image_storage");

  if (has(ext, "EGL_EXT_image_dma_buf_import_modifiers") && p_eglQueryDmaBufFormatsEXT) {
    EGLint fmts[128], nf = 0; p_eglQueryDmaBufFormatsEXT(dpy, 128, fmts, &nf);
    printf("dmabuf formats (%d):", nf);
    for (int i = 0; i < nf; i++) printf(" %s", fcc((uint32_t)fmts[i]));
    printf("\n");
    uint32_t q[] = { F_NV12, F_R8, F_GR88 };
    for (int i = 0; i < 3; i++) {
      EGLuint64KHR mods[32]; EGLBoolean eo[32]; EGLint nm = 0;
      EGLBoolean ok = p_eglQueryDmaBufModifiersEXT(dpy, (EGLint)q[i], 32, mods, eo, &nm);
      printf("  modifiers for %s: %s count=%d", fcc(q[i]), ok ? "ok" : "query failed", nm);
      for (int k = 0; k < nm && k < 32; k++) printf(" 0x%llx%s", (unsigned long long)mods[k], eo[k] ? "(external only)" : "");
      printf("\n");
    }
  } else printf("no eglQueryDmaBufFormatsEXT: formats cannot be listed, modifiers cannot be passed\n");

  /* render target */
  GLuint fbt, fbo; p_glGenTextures(1, &fbt); p_glBindTexture(GL_TEXTURE_2D, fbt);
  p_glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 64, 64, 0, GL_RGBA, GL_UNSIGNED_BYTE, NULL);
  p_glGenFramebuffers(1, &fbo); p_glBindFramebuffer(GL_FRAMEBUFFER, fbo);
  p_glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, fbt, 0);
  if (p_glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) { printf("framebuffer incomplete\n"); return 1; }
  p_glViewport(0, 0, 64, 64);

  static const char *fs_ext = "#extension GL_OES_EGL_image_external : require\nprecision mediump float; varying vec2 t; uniform samplerExternalOES s; void main(){ gl_FragColor=texture2D(s,t); }";
  static const char *fs_2d  = "precision mediump float; varying vec2 t; uniform sampler2D s; void main(){ gl_FragColor=texture2D(s,t); }";
  printf("imports of the exported buffer (expect about 255,0,0 for NV12; 81 for R8; 90,240 for GR88):\n");
#define NV12_BASE EGL_WIDTH, 1920, EGL_HEIGHT, 1080, EGL_LINUX_DRM_FOURCC_EXT, (EGLint)F_NV12, \
    EGL_DMA_BUF_PLANE0_FD_EXT, bfd, EGL_DMA_BUF_PLANE0_OFFSET_EXT, 0, EGL_DMA_BUF_PLANE0_PITCH_EXT, (EGLint)pitch, \
    EGL_DMA_BUF_PLANE1_FD_EXT, bfd, EGL_DMA_BUF_PLANE1_OFFSET_EXT, (EGLint)uvoff, EGL_DMA_BUF_PLANE1_PITCH_EXT, (EGLint)d.layers[1].pitch[0]
  { EGLint a[] = { NV12_BASE, EGL_NONE };
    try_import(dpy, "NV12 two planes, no modifier, external", GL_TEXTURE_EXTERNAL_OES, fs_ext, a);
    try_import(dpy, "NV12 two planes, no modifier, TEXTURE_2D", GL_TEXTURE_2D, fs_2d, a); }
  { EGLint a[] = { NV12_BASE, EGL_YUV_COLOR_SPACE_HINT_EXT, EGL_ITU_REC709_EXT, EGL_SAMPLE_RANGE_HINT_EXT, EGL_YUV_NARROW_RANGE_EXT, EGL_NONE };
    try_import(dpy, "NV12 + colour space hints, external", GL_TEXTURE_EXTERNAL_OES, fs_ext, a); }
  { EGLint a[] = { NV12_BASE, EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT, 0, EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT, 0,
                   EGL_DMA_BUF_PLANE1_MODIFIER_LO_EXT, 0, EGL_DMA_BUF_PLANE1_MODIFIER_HI_EXT, 0, EGL_NONE };
    try_import(dpy, "NV12 + modifier LINEAR (0), external", GL_TEXTURE_EXTERNAL_OES, fs_ext, a); }
  { EGLint a[] = { EGL_WIDTH, 1920, EGL_HEIGHT, 1080, EGL_LINUX_DRM_FOURCC_EXT, (EGLint)F_R8,
                   EGL_DMA_BUF_PLANE0_FD_EXT, bfd, EGL_DMA_BUF_PLANE0_OFFSET_EXT, 0, EGL_DMA_BUF_PLANE0_PITCH_EXT, (EGLint)pitch, EGL_NONE };
    try_import(dpy, "plane 0 as R8, TEXTURE_2D", GL_TEXTURE_2D, fs_2d, a);
    try_import(dpy, "plane 0 as R8, external", GL_TEXTURE_EXTERNAL_OES, fs_ext, a); }
  { EGLint a[] = { EGL_WIDTH, 960, EGL_HEIGHT, 540, EGL_LINUX_DRM_FOURCC_EXT, (EGLint)F_GR88,
                   EGL_DMA_BUF_PLANE0_FD_EXT, bfd, EGL_DMA_BUF_PLANE0_OFFSET_EXT, (EGLint)uvoff, EGL_DMA_BUF_PLANE0_PITCH_EXT, (EGLint)d.layers[1].pitch[0], EGL_NONE };
    try_import(dpy, "plane 1 as GR88, TEXTURE_2D", GL_TEXTURE_2D, fs_2d, a);
    try_import(dpy, "plane 1 as GR88, external", GL_TEXTURE_EXTERNAL_OES, fs_ext, a); }
  vaDestroySurfaces(va, &sid, 1); vaTerminate(va);
  return 0;
}
