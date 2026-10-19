/* Shared helpers of the conformance plugins. Every trace line starts with
 * "T " so the runner can drop the simulator's own output. Error text is
 * never printed (it differs by design), only whether an error was pending.
 * SPDX-License-Identifier: MIT OR Apache-2.0 */
#ifndef COMMON_H
#define COMMON_H

#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "vhpi_user.h"

/* Output is collected until the newline and printed with one vhpi_printf,
 * because a simulator may treat each call as a message of its own. */
static void L(const char *fmt, ...) {
  static char line[2048];
  static size_t len;
  va_list ap;
  va_start(ap, fmt);
  vsnprintf(line + len, sizeof line - len, fmt, ap);
  va_end(ap);
  len = strlen(line);
  if (len && line[len - 1] == '\n') {
    line[len - 1] = 0;
    vhpi_printf("%s\n", line);
    len = 0;
    line[0] = 0;
  }
}
#define T(...) (L("T "), L(__VA_ARGS__))

/* 1 when the previous VHPI call left an error (and clear it). */
static int err(void) {
  vhpiErrorInfoT i;
  return vhpi_check_error(&i) != 0;
}

/* vhpi_get_str result copied into a ring, so several can sit in one call. */
static const char *S(vhpiStrPropertyT p, vhpiHandleT h) {
  static char ring[8][256];
  static int n;
  char *b = ring[n++ & 7];
  const vhpiCharT *s = vhpi_get_str(p, h);
  if (!s) return "(null)";
  snprintf(b, 256, "%s", (const char *)s);
  return b;
}

/* ":a:b" is a full name; anything else is relative to the design root. */
static vhpiHandleT lookup(const char *name) {
  if (name[0] == ':') return vhpi_handle_by_name(name, NULL);
  vhpiHandleT root = vhpi_handle(vhpiRootInst, NULL);
  vhpiHandleT h = vhpi_handle_by_name(name, root);
  if (h) return h;
  /* A simulator whose root is a wrapper above the design: retry in its first region. */
  vhpiHandleT it = vhpi_iterator(vhpiInternalRegions, root);
  vhpiHandleT top = it ? vhpi_scan(it) : NULL;
  if (top) {
    h = vhpi_handle_by_name(name, top);
    if (h) {
      vhpiErrorInfoT e;
      vhpi_check_error(&e);
    }
  }
  return h;
}

/* Time in fs, from the callback time or vhpi_get_time. */
static long long fs_of(const vhpiTimeT *t) {
  vhpiPhysT r = vhpi_get_phys(vhpiResolutionLimitP, NULL);
  return (((long long)t->high << 32) | t->low) * (((long long)r.high << 32) | r.low);
}

static void now(const char *tag) {
  vhpiTimeT t;
  long cyc = 0;
  vhpi_get_time(&t, &cyc);
  T("%s time=%lldfs cycles=%ld\n", tag, fs_of(&t), cyc);
}

/* One value read in format f; prints rc, the error flag and the payload. */
static void show(vhpiHandleT h, vhpiFormatT f, const char *fname) {
  static char buf[512];
  static vhpiEnumT ev[64];
  static vhpiIntT iv[64];
  static vhpiRealT rv[64];
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  memset(buf, 0, sizeof buf);
  v.format = f;
  v.bufSize = sizeof buf;
  v.value.str = (vhpiCharT *)buf;
  if (f == vhpiLogicVecVal || f == vhpiEnumVecVal) {
    v.bufSize = sizeof ev;
    v.value.enumvs = ev;
  } else if (f == vhpiIntVecVal) {
    v.bufSize = sizeof iv;
    v.value.intgs = iv;
  } else if (f == vhpiRealVecVal) {
    v.bufSize = sizeof rv;
    v.value.reals = rv;
  }
  int rc = vhpi_get_value(h, &v);
  int e = err();
  T("  %s rc=%d err=%d fmt=%d n=%d", fname, rc, e, (int)v.format, (int)v.numElems);
  if (!rc && !e) switch (v.format) {
    case vhpiBinStrVal: case vhpiOctStrVal: case vhpiHexStrVal:
    case vhpiDecStrVal: case vhpiStrVal:
      L(" '%s'", buf); break;
    case vhpiCharVal: L(" '%c'", v.value.ch); break;
    case vhpiEnumVal: case vhpiLogicVal: L(" %u", v.value.enumv); break;
    case vhpiSmallEnumVal: L(" %u", v.value.smallenumv); break;
    case vhpiIntVal: L(" %d", v.value.intg); break;
    case vhpiRealVal: L(" %g", v.value.real); break;
    case vhpiPhysVal: L(" %d/%u", v.value.phys.high, v.value.phys.low); break;
    case vhpiTimeVal: L(" %d/%u", v.value.time.high, v.value.time.low); break;
    case vhpiLogicVecVal: case vhpiEnumVecVal:
      for (int i = 0; i < v.numElems && i < 64; i++) L(" %u", ev[i]);
      break;
    case vhpiIntVecVal:
      for (int i = 0; i < v.numElems && i < 64; i++) L(" %d", iv[i]);
      break;
    case vhpiRealVecVal:
      for (int i = 0; i < v.numElems && i < 64; i++) L(" %g", rv[i]);
      break;
    default: break;
  }
  L("\n");
}

#define SHOW(h, f) show(h, f, #f)

static void all_formats(vhpiHandleT h) {
  SHOW(h, vhpiBinStrVal); SHOW(h, vhpiOctStrVal); SHOW(h, vhpiHexStrVal);
  SHOW(h, vhpiDecStrVal); SHOW(h, vhpiStrVal);    SHOW(h, vhpiCharVal);
  SHOW(h, vhpiEnumVal);   SHOW(h, vhpiSmallEnumVal); SHOW(h, vhpiLogicVal);
  SHOW(h, vhpiIntVal);    SHOW(h, vhpiRealVal);   SHOW(h, vhpiPhysVal);
  SHOW(h, vhpiTimeVal);   SHOW(h, vhpiEnumVecVal); SHOW(h, vhpiLogicVecVal);
  SHOW(h, vhpiIntVecVal); SHOW(h, vhpiRealVecVal);
  SHOW(h, vhpiObjTypeVal);
}

/* PLUGIN_START(fn) registers fn to run at the start of simulation (the
 * design is elaborated by then); PLUGIN_INIT(fn) runs fn at load. */
static void (*plugin_body)(void);
static void plugin_cb(const vhpiCbDataT *cb) { (void)cb; plugin_body(); }
static void plugin_register(void) {
  vhpiCbDataT cb;
  memset(&cb, 0, sizeof cb);
  cb.reason = vhpiCbStartOfSimulation;
  cb.cb_rtn = plugin_cb;
  vhpi_register_cb(&cb, 0);
}
#ifdef _WIN32
#define PLUGIN_EXPORT __declspec(dllexport)
#else
#define PLUGIN_EXPORT
#endif
#define PLUGIN_INIT(fn) \
  PLUGIN_EXPORT void (*vhpi_startup_routines[])(void) = {fn, NULL}; \
  PLUGIN_EXPORT void conf_boot(void) { fn(); }
#define PLUGIN_START(fn) \
  static void plugin_init(void) { plugin_body = fn; plugin_register(); } \
  PLUGIN_INIT(plugin_init)

#endif
