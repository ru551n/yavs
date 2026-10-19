/* Test VHPI plugin: reads objects of every kind in every vhpi_get_value
 * format, first with no buffer (the call returns the size it needs) and then
 * with one. A signal is read at the start of simulation, from an after-delay
 * callback, and everything at the end. The output is compared with
 * tests/values/expected.txt.
 * SPDX-License-Identifier: MIT OR Apache-2.0 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "vhpi_user.h"

#ifdef _WIN32
#define PLUGIN_EXPORT __declspec(dllexport)
#else
#define PLUGIN_EXPORT
#endif

static char line[4096];
static size_t used;

#define put(...)                                                              \
  (used += (size_t)snprintf(line + used, sizeof line - used, __VA_ARGS__),    \
   used = used < sizeof line ? used : sizeof line - 1)

static void flush(void) {
  vhpi_printf("%s\n", line);
  used = 0;
  line[0] = 0;
}

/* The root instance, looked up once. */
static vhpiHandleT root(void) {
  static vhpiHandleT r;
  if (!r) r = vhpi_handle(vhpiRootInst, NULL);
  return r;
}

static const char *names[] = {
    "sl", "slv", "sv", "bv", "b", "bo", "i", "r", "t", "d", "st", "s",
    "ch", "rec", "rec.a", "rec.b", "rec.c", "mem", "mem(1,2)", "iv",
    "iv(1)", "rv", "slv(5)", "p", "g", "k", "cnt", NULL};

static const struct {
  int f;
  const char *n;
} fmts[] = {{vhpiObjTypeVal, "ObjType"},   {vhpiBinStrVal, "BinStr"},
            {vhpiOctStrVal, "OctStr"},     {vhpiDecStrVal, "DecStr"},
            {vhpiHexStrVal, "HexStr"},     {vhpiStrVal, "Str"},
            {vhpiCharVal, "Char"},         {vhpiEnumVal, "Enum"},
            {vhpiSmallEnumVal, "SmallEnum"}, {vhpiLogicVal, "Logic"},
            {vhpiIntVal, "Int"},           {vhpiLongIntVal, "LongInt"},
            {vhpiRealVal, "Real"},         {vhpiTimeVal, "Time"},
            {vhpiPhysVal, "Phys"},         {vhpiEnumVecVal, "EnumVec"},
            {vhpiSmallEnumVecVal, "SmallEnumVec"},
            {vhpiLogicVecVal, "LogicVec"}, {vhpiIntVecVal, "IntVec"},
            {vhpiRealVecVal, "RealVec"},   {vhpiTimeVecVal, "TimeVec"},
            {vhpiPtrVal, "Ptr"},           {0, NULL}};

static void show(const vhpiValueT *v) {
  int k;
  switch (v->format) {
  case vhpiBinStrVal: case vhpiOctStrVal: case vhpiDecStrVal:
  case vhpiHexStrVal: case vhpiStrVal:
    put(" \"%s\"", (const char *)v->value.str);
    break;
  case vhpiCharVal: put(" '%c'", v->value.ch); break;
  case vhpiEnumVal: case vhpiLogicVal: put(" %u", (unsigned)v->value.enumv); break;
  case vhpiSmallEnumVal: put(" %u", (unsigned)v->value.smallenumv); break;
  case vhpiIntVal: put(" %d", (int)v->value.intg); break;
  case vhpiLongIntVal: put(" %lld", (long long)v->value.longintg); break;
  case vhpiRealVal: put(" %g", v->value.real); break;
  case vhpiTimeVal:
    put(" %d:%u", (int)v->value.time.high, (unsigned)v->value.time.low);
    break;
  case vhpiPhysVal:
    put(" %d:%u", (int)v->value.phys.high, (unsigned)v->value.phys.low);
    break;
  case vhpiEnumVecVal: case vhpiLogicVecVal:
    for (k = 0; k < v->numElems; k++) put(" %u", (unsigned)v->value.enumvs[k]);
    break;
  case vhpiSmallEnumVecVal:
    for (k = 0; k < v->numElems; k++) put(" %u", (unsigned)v->value.smallenumvs[k]);
    break;
  case vhpiIntVecVal:
    for (k = 0; k < v->numElems; k++) put(" %d", (int)v->value.intgs[k]);
    break;
  case vhpiRealVecVal:
    for (k = 0; k < v->numElems; k++) put(" %g", v->value.reals[k]);
    break;
  case vhpiTimeVecVal:
    for (k = 0; k < v->numElems; k++)
      put(" %d:%u", (int)v->value.times[k].high, (unsigned)v->value.times[k].low);
    break;
  default: put(" ?"); break;
  }
}

/* One line: the first call's result, then the second call's with a buffer of
 * the size the first asked for. */
static void read_one(vhpiHandleT h, const char *name, int f, const char *fname) {
  vhpiValueT v;
  int rc;
  memset(&v, 0, sizeof v);
  v.format = f;
  rc = vhpi_get_value(h, &v);
  put("%s %s rc=%d", name, fname, rc);
  if (f == vhpiObjTypeVal) put(" fmt=%d", v.format);
  if (rc > 0) {
    void *buf = calloc(1, (size_t)rc);
    v.bufSize = (size_t)rc;
    v.value.ptr = buf;
    rc = vhpi_get_value(h, &v);
    put(" then rc=%d n=%d", rc, v.numElems);
    if (rc == 0) show(&v);
    free(buf);
  } else if (rc == 0) {
    show(&v);
  }
  if (rc < 0 && vhpi_check_error(NULL)) put(" error");
  flush();
}

static void read_cnt(const vhpiCbDataT *cb) {
  vhpiHandleT h = vhpi_handle_by_name("cnt", root());
  put("t=%u ", (unsigned)cb->time->low);
  read_one(h, "cnt", vhpiIntVal, "Int");
}

static void read_all(const vhpiCbDataT *cb) {
  int i, f;
  (void)cb;
  for (i = 0; names[i]; i++) {
    vhpiHandleT h = vhpi_handle_by_name(names[i], root());
    if (!h) {
      put("%s: no handle", names[i]);
      flush();
      continue;
    }
    for (f = 0; fmts[f].n; f++) read_one(h, names[i], fmts[f].f, fmts[f].n);
    vhpi_release_handle(h);
  }
  /* A NULL buffer pointer also asks for the size; a short one does too. */
  {
    vhpiHandleT h = vhpi_handle_by_name("s", root());
    char small[3];
    vhpiValueT v;
    memset(&v, 0, sizeof v);
    v.format = vhpiStrVal;
    v.bufSize = sizeof small;
    v.value.str = (vhpiCharT *)small;
    put("short buffer rc=%d", vhpi_get_value(h, &v));
    flush();
  }
}

static void start(const vhpiCbDataT *cb) {
  vhpiCbDataT d;
  vhpiTimeT t = {0, 2000000}; /* 2 ns in femtoseconds, the default resolution */
  read_cnt(cb);
  memset(&d, 0, sizeof d);
  d.reason = vhpiCbAfterDelay;
  d.cb_rtn = read_cnt;
  d.time = &t;
  vhpi_register_cb(&d, 0);
}

static void startup(void) {
  vhpiCbDataT cb;
  memset(&cb, 0, sizeof cb);
  cb.reason = vhpiCbStartOfSimulation;
  cb.cb_rtn = start;
  vhpi_register_cb(&cb, 0);
  cb.reason = vhpiCbEndOfSimulation;
  cb.cb_rtn = read_all;
  vhpi_register_cb(&cb, 0);
}

PLUGIN_EXPORT void (*vhpi_startup_routines[])(void) = {startup, NULL};
