/* Test VHPI plugin: foreign functions and procedures registered with
 * vhpi_register_foreignf and called from tests/foreign/design.vhd. The execution
 * callbacks read the parameters through vhpi_iterator(vhpiParamDecls) and
 * vhpi_handle_by_index with vhpi_get_value, and return values with vhpi_put_value
 * on the call (the function result) or on out/inout parameters.
 * SPDX-License-Identifier: MIT OR Apache-2.0 */

#include <stdio.h>
#include <string.h>

#include "vhpi_user.h"

#ifdef _WIN32
#define PLUGIN_EXPORT __declspec(dllexport)
#else
#define PLUGIN_EXPORT
#endif

static vhpiHandleT param(const vhpiCbDataT *cb, int i) {
  return vhpi_handle_by_index(vhpiParamDecls, cb->obj, i);
}

static int get_int(const vhpiCbDataT *cb, int i) {
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiIntVal;
  if (vhpi_get_value(param(cb, i), &v)) vhpi_printf("get_int failed\n");
  return v.value.intg;
}

static void put_int(vhpiHandleT h, int x) {
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiIntVal;
  v.value.intg = x;
  if (vhpi_put_value(h, &v, vhpiDepositPropagate)) vhpi_printf("put_int failed\n");
}

static void add_exec(const vhpiCbDataT *cb) {
  /* The parameters, their names and modes, once. */
  static int shown;
  if (!shown++) {
    vhpiHandleT it = vhpi_iterator(vhpiParamDecls, cb->obj);
    vhpiHandleT p;
    vhpi_printf("add: call kind=%d name=%s\n", vhpi_get(vhpiKindP, cb->obj),
                (const char *)vhpi_get_str(vhpiNameP, cb->obj));
    while ((p = vhpi_scan(it)))
      vhpi_printf("add: param %s mode=%d kind=%d\n",
                  (const char *)vhpi_get_str(vhpiNameP, p), vhpi_get(vhpiModeP, p),
                  vhpi_get(vhpiKindP, p));
  }
  put_int(cb->obj, get_int(cb, 0) + get_int(cb, 1));
}

static void half_exec(const vhpiCbDataT *cb) {
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiRealVal;
  vhpi_get_value(param(cb, 0), &v);
  v.value.real /= 2;
  vhpi_put_value(cb->obj, &v, vhpiDepositPropagate);
}

static void later_exec(const vhpiCbDataT *cb) {
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiTimeVal;
  vhpi_get_value(param(cb, 0), &v);
  /* Times count in the resolution limit (femtoseconds by default). */
  long long t = ((long long)v.value.time.high << 32) | v.value.time.low;
  t += 1000000; /* 1 ns */
  v.value.time.high = (int)(t >> 32);
  v.value.time.low = (unsigned)t;
  vhpi_put_value(cb->obj, &v, vhpiDepositPropagate);
}

static void parity_exec(const vhpiCbDataT *cb) {
  vhpiEnumT bits[64];
  vhpiValueT v;
  int ones = 0, i;
  memset(&v, 0, sizeof v);
  v.format = vhpiLogicVecVal;
  v.bufSize = sizeof bits;
  v.value.enumvs = bits;
  if (vhpi_get_value(param(cb, 0), &v)) vhpi_printf("parity: get failed\n");
  for (i = 0; i < v.numElems; i++) ones += bits[i] == vhpi1;
  memset(&v, 0, sizeof v);
  v.format = vhpiLogicVal;
  v.value.enumv = (ones & 1) ? vhpi1 : vhpi0;
  vhpi_put_value(cb->obj, &v, vhpiDepositPropagate);
}

static void is_big_exec(const vhpiCbDataT *cb) {
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiEnumVal;
  v.value.enumv = get_int(cb, 0) > 10;
  vhpi_put_value(cb->obj, &v, vhpiDepositPropagate);
}

static void divmod_exec(const vhpiCbDataT *cb) {
  int a = get_int(cb, 0), b = get_int(cb, 1);
  put_int(param(cb, 2), a / b);
  put_int(param(cb, 3), a % b);
  /* Writing an in parameter is misuse. */
  {
    vhpiValueT v;
    vhpiErrorInfoT e;
    memset(&v, 0, sizeof v);
    v.format = vhpiIntVal;
    if (!vhpi_put_value(param(cb, 0), &v, vhpiDepositPropagate))
      vhpi_printf("divmod: in parameter written\n");
    if (vhpi_check_error(&e)) vhpi_printf("divmod: in parameter: %s\n", e.message);
  }
}

static void invert_exec(const vhpiCbDataT *cb) {
  vhpiEnumT bits[64];
  vhpiValueT v;
  int i;
  memset(&v, 0, sizeof v);
  v.format = vhpiLogicVecVal;
  v.bufSize = sizeof bits;
  v.value.enumvs = bits;
  vhpi_get_value(param(cb, 0), &v);
  for (i = 0; i < v.numElems; i++) bits[i] = bits[i] == vhpi1 ? vhpi0 : vhpi1;
  vhpi_put_value(param(cb, 0), &v, vhpiDepositPropagate);
}

static void bump_exec(const vhpiCbDataT *cb) { put_int(param(cb, 0), get_int(cb, 0) + 1); }

static void forgetful_exec(const vhpiCbDataT *cb) { (void)cb; }

/* The size of an array result, set to n with vhpiSizeConstraint (before, the call result
 * and after, once). */
static void size(const vhpiCbDataT *cb, int n) {
  static int shown;
  vhpiValueT v;
  vhpiErrorInfoT e;
  int before = vhpi_get(vhpiSizeP, cb->obj), rc;
  memset(&v, 0, sizeof v);
  v.format = vhpiObjTypeVal;
  v.numElems = n;
  rc = vhpi_put_value(cb->obj, &v, vhpiSizeConstraint);
  if (!shown++)
    vhpi_printf("size: before=%d composite=%d constraint rc=%d after=%d\n", before,
                vhpi_get(vhpiIsCompositeP, cb->obj), rc, vhpi_get(vhpiSizeP, cb->obj));
  vhpi_check_error(&e);
}

static void rev_exec(const vhpiCbDataT *cb) {
  vhpiEnumT in[64], out[64];
  vhpiValueT v;
  int i, n;
  memset(&v, 0, sizeof v);
  v.format = vhpiLogicVecVal;
  v.bufSize = sizeof in;
  v.value.enumvs = in;
  vhpi_get_value(param(cb, 0), &v);
  n = v.numElems;
  size(cb, n);
  for (i = 0; i < n; i++) out[i] = in[n - 1 - i];
  v.value.enumvs = out;
  if (vhpi_put_value(cb->obj, &v, vhpiDepositPropagate)) vhpi_printf("rev: put failed\n");
}

static void iota_exec(const vhpiCbDataT *cb) {
  vhpiIntT out[64];
  vhpiValueT v;
  int i, n = get_int(cb, 0);
  size(cb, n);
  memset(&v, 0, sizeof v);
  for (i = 0; i < n; i++) out[i] = 10 * i;
  v.format = vhpiIntVecVal;
  v.numElems = n;
  v.bufSize = n * sizeof(vhpiIntT);
  v.value.intgs = out;
  if (vhpi_put_value(cb->obj, &v, vhpiDepositPropagate)) vhpi_printf("iota: put failed\n");
}

static void word_exec(const vhpiCbDataT *cb) {
  char out[] = "abc";
  vhpiValueT v;
  size(cb, get_int(cb, 0));
  memset(&v, 0, sizeof v);
  v.format = vhpiStrVal;
  v.bufSize = sizeof out;
  v.value.str = (vhpiCharT *)out;
  if (vhpi_put_value(cb->obj, &v, vhpiDepositPropagate)) vhpi_printf("word: put failed\n");
}

/* A constrained result has its size; another size is misuse. */
static void nibble_exec(const vhpiCbDataT *cb) {
  static int shown;
  vhpiEnumT out[4];
  vhpiValueT v;
  vhpiErrorInfoT e;
  int i, x = get_int(cb, 0), n = vhpi_get(vhpiSizeP, cb->obj);
  memset(&v, 0, sizeof v);
  v.format = vhpiLogicVecVal;
  v.numElems = 5;
  if (!vhpi_put_value(cb->obj, &v, vhpiSizeConstraint)) vhpi_printf("nibble: resized\n");
  if (vhpi_check_error(&e) && !shown++) vhpi_printf("nibble: size=%d resize: %s\n", n, e.message);
  for (i = 0; i < 4; i++) out[i] = (x >> (3 - i)) & 1 ? vhpi1 : vhpi0;
  v.numElems = 4;
  v.bufSize = sizeof out;
  v.value.enumvs = out;
  if (vhpi_put_value(cb->obj, &v, vhpiDepositPropagate)) vhpi_printf("nibble: put failed\n");
}

/* An unconstrained result put without vhpiSizeConstraint is misuse. */
static void unsized_exec(const vhpiCbDataT *cb) {
  vhpiIntT out[1] = {1};
  vhpiValueT v;
  vhpiErrorInfoT e;
  memset(&v, 0, sizeof v);
  v.format = vhpiIntVecVal;
  v.numElems = 1;
  v.bufSize = sizeof out;
  v.value.intgs = out;
  if (!vhpi_put_value(cb->obj, &v, vhpiDepositPropagate)) vhpi_printf("unsized: put accepted\n");
  if (vhpi_check_error(&e)) vhpi_printf("unsized: %s\n", e.message);
}

static void elab(const vhpiCbDataT *cb) {
  vhpiForeignDataT d;
  if (vhpi_get_foreignf_info(cb->obj, &d))
    vhpi_printf("elab: no info\n");
  else
    vhpi_printf("elab: %s %s kind=%d\n", d.libraryName, d.modelName, (int)d.kind);
}

/* Record parameters: the fields from vhpiSelectedNames, by index and by iterator. */
static int field_int(vhpiHandleT rec, int i) {
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiIntVal;
  if (vhpi_get_value(vhpi_handle_by_index(vhpiSelectedNames, rec, i), &v))
    vhpi_printf("field_int failed\n");
  return v.value.intg;
}

static int ones(vhpiHandleT h) {
  vhpiEnumT bits[64];
  vhpiValueT v;
  int i, n = 0;
  memset(&v, 0, sizeof v);
  v.format = vhpiLogicVecVal;
  v.bufSize = sizeof bits;
  v.value.enumvs = bits;
  if (vhpi_get_value(h, &v)) vhpi_printf("ones: get failed\n");
  for (i = 0; i < v.numElems; i++) n += bits[i] == vhpi1;
  return n;
}

/* x + ok + the number of '1' in tag. */
static void norm_exec(const vhpiCbDataT *cb) {
  static int shown;
  vhpiHandleT p = param(cb, 0), it, f;
  vhpiValueT v;
  if (!shown++) {
    vhpi_printf("norm: p kind=%d composite=%d size=%d\n", vhpi_get(vhpiKindP, p),
                vhpi_get(vhpiIsCompositeP, p), vhpi_get(vhpiSizeP, p));
    it = vhpi_iterator(vhpiSelectedNames, p);
    while ((f = vhpi_scan(it)))
      vhpi_printf("norm: field %s kind=%d\n", (const char *)vhpi_get_str(vhpiNameP, f),
                  vhpi_get(vhpiKindP, f));
    /* A record has no value as a whole. */
    memset(&v, 0, sizeof v);
    v.format = vhpiObjTypeVal;
    if (!vhpi_get_value(p, &v)) vhpi_printf("norm: whole record read\n");
  }
  put_int(cb->obj, field_int(p, 0) + field_int(p, 1) +
                       ones(vhpi_handle_by_index(vhpiSelectedNames, p, 2)));
}

/* q.x = -p.x, q.ok = not p.ok, q.tag = not p.tag. */
static void flip_exec(const vhpiCbDataT *cb) {
  vhpiHandleT p = param(cb, 0), q = param(cb, 1);
  vhpiEnumT bits[4];
  vhpiValueT v;
  int i;
  put_int(vhpi_handle_by_index(vhpiSelectedNames, q, 0), -field_int(p, 0));
  memset(&v, 0, sizeof v);
  v.format = vhpiEnumVal;
  v.value.enumv = !field_int(p, 1);
  vhpi_put_value(vhpi_handle_by_index(vhpiSelectedNames, q, 1), &v, vhpiDepositPropagate);
  memset(&v, 0, sizeof v);
  v.format = vhpiLogicVecVal;
  v.bufSize = sizeof bits;
  v.value.enumvs = bits;
  vhpi_get_value(vhpi_handle_by_index(vhpiSelectedNames, p, 2), &v);
  for (i = 0; i < 4; i++) bits[i] = bits[i] == vhpi1 ? vhpi0 : vhpi1;
  vhpi_put_value(vhpi_handle_by_index(vhpiSelectedNames, q, 2), &v, vhpiDepositPropagate);
}

/* An access parameter is the address of the designated object's data (an integer is 64
 * bits): r gets the value, which is then incremented; -1 for null. */
static void peek_exec(const vhpiCbDataT *cb) {
  vhpiValueT v;
  long long *p;
  memset(&v, 0, sizeof v);
  v.format = vhpiObjTypeVal;
  if (vhpi_get_value(param(cb, 0), &v) || v.format != vhpiPtrVal) vhpi_printf("peek: no pointer\n");
  p = (long long *)v.value.ptr;
  put_int(param(cb, 1), p ? (int)(*p)++ : -1);
}

/* s in, o and k out: o gets a deposit of "1111", then not s after 1 ns (inertial) through
 * its driver; k gets the number of '1' in "1111" after 2 ns (transport). */
static void drive_exec(const vhpiCbDataT *cb) {
  vhpiHandleT s = param(cb, 0), o = param(cb, 1), k = param(cb, 2), d, it;
  vhpiEnumT bits[4];
  vhpiValueT v;
  vhpiTimeT t;
  vhpiErrorInfoT e;
  int i, n = ones(s);
  vhpi_printf("drive: s kind=%d mode=%d ones=%d\n", vhpi_get(vhpiKindP, s), vhpi_get(vhpiModeP, s), n);
  memset(&v, 0, sizeof v);
  v.format = vhpiLogicVecVal;
  v.numElems = 4;
  v.bufSize = sizeof bits;
  v.value.enumvs = bits;
  for (i = 0; i < 4; i++) bits[i] = vhpi1;
  if (vhpi_put_value(o, &v, vhpiDepositPropagate)) vhpi_printf("drive: deposit failed\n");
  vhpi_get_value(s, &v);
  for (i = 0; i < 4; i++) bits[i] = bits[i] == vhpi1 ? vhpi0 : vhpi1;
  it = vhpi_iterator(vhpiDrivers, o);
  d = vhpi_scan(it);
  vhpi_printf("drive: driver kind=%d\n", vhpi_get(vhpiKindP, d));
  vhpi_release_handle(it);
  t.high = 0;
  t.low = 1000000; /* 1 ns in fs */
  if (vhpi_schedule_transaction(d, &v, 1, &t, vhpiInertial, NULL)) vhpi_printf("drive: o failed\n");
  /* A transaction on the parameter itself, or on an in parameter, is misuse. */
  if (!vhpi_schedule_transaction(o, &v, 1, &t, vhpiInertial, NULL)) vhpi_printf("drive: o accepted\n");
  if (vhpi_check_error(&e)) vhpi_printf("drive: on the parameter: %s\n", e.message);
  if (vhpi_iterator(vhpiDrivers, s)) vhpi_printf("drive: s has a driver\n");
  vhpi_check_error(&e);
  it = vhpi_iterator(vhpiDrivers, k);
  d = vhpi_scan(it);
  vhpi_release_handle(it);
  memset(&v, 0, sizeof v);
  v.format = vhpiIntVal;
  v.value.intg = 4;
  t.low = 2000000;
  if (vhpi_schedule_transaction(d, &v, 1, &t, vhpiTransport, NULL)) vhpi_printf("drive: k failed\n");
}

static void reg(vhpiForeignKindT kind, char *name, void (*exec)(const vhpiCbDataT *)) {
  vhpiForeignDataT d;
  vhpiHandleT h;
  memset(&d, 0, sizeof d);
  d.kind = kind;
  d.libraryName = "plug";
  d.modelName = name;
  d.elabf = elab;
  d.execf = exec;
  h = vhpi_register_foreignf(&d);
  if (!h) vhpi_printf("register %s failed\n", name);
}

PLUGIN_EXPORT void foreign_models(void) {
  reg(vhpiFuncF, "add", add_exec);
  reg(vhpiFuncF, "half", half_exec);
  reg(vhpiFuncF, "later", later_exec);
  reg(vhpiFuncF, "parity", parity_exec);
  reg(vhpiFuncF, "is_big", is_big_exec);
  reg(vhpiProcF, "divmod", divmod_exec);
  reg(vhpiProcF, "invert", invert_exec);
  reg(vhpiProcF, "bump", bump_exec);
  reg(vhpiFuncF, "forgetful", forgetful_exec);
  reg(vhpiFuncF, "rev", rev_exec);
  reg(vhpiFuncF, "iota", iota_exec);
  reg(vhpiFuncF, "word", word_exec);
  reg(vhpiFuncF, "nibble", nibble_exec);
  reg(vhpiFuncF, "unsized", unsized_exec);
  reg(vhpiFuncF, "norm", norm_exec);
  reg(vhpiProcF, "flip", flip_exec);
  reg(vhpiProcF, "peek", peek_exec);
  reg(vhpiProcF, "drive", drive_exec);
}

/* yavs only: registration queries and misuse (other simulators crash or accept it). */
PLUGIN_EXPORT void foreign_extras(void) {
  vhpiForeignDataT d, back;
  vhpiHandleT h;
  vhpiErrorInfoT e;
  /* vhpi_get_foreignf_info returns the registration. */
  memset(&d, 0, sizeof d);
  d.kind = vhpiProcF;
  d.libraryName = "other";
  d.modelName = "info";
  d.execf = bump_exec;
  h = vhpi_register_foreignf(&d);
  memset(&back, 0, sizeof back);
  if (!h || vhpi_get_foreignf_info(h, &back))
    vhpi_printf("info: failed\n");
  else
    vhpi_printf("info: %s %s kind=%d elabf=%d execf=%d kindP=%d name=%s\n",
                back.libraryName, back.modelName, (int)back.kind, back.elabf != NULL,
                back.execf == bump_exec, vhpi_get(vhpiForeignKindP, h),
                (const char *)vhpi_get_str(vhpiNameP, h));
  /* Misuse: a duplicate, a missing execf, a NULL registration. */
  if (vhpi_register_foreignf(&d)) vhpi_printf("duplicate accepted\n");
  if (vhpi_check_error(&e)) vhpi_printf("duplicate: %s\n", e.message);
  d.modelName = "noexec";
  d.execf = NULL;
  if (vhpi_register_foreignf(&d)) vhpi_printf("noexec accepted\n");
  if (vhpi_check_error(&e)) vhpi_printf("noexec: %s\n", e.message);
  if (vhpi_register_foreignf(NULL)) vhpi_printf("NULL accepted\n");
}

PLUGIN_EXPORT void (*vhpi_startup_routines[])(void) = {foreign_models, NULL};
