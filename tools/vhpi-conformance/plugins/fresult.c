/* Foreign functions with array results: the size of an unconstrained result set with
 * vhpi_put_value(vhpiSizeConstraint) before the value, or by the value put alone, and a
 * constrained result. Each step prints its return code and error flag; the results are
 * read from the design's signals at 5 ns.
 * SPDX-License-Identifier: MIT OR Apache-2.0 */
#include "common.h"

static vhpiHandleT param(const vhpiCbDataT *cb, int i) {
  return vhpi_handle_by_index(vhpiParamDecls, cb->obj, i);
}

static int get_int(const vhpiCbDataT *cb, int i) {
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiIntVal;
  vhpi_get_value(param(cb, i), &v);
  return v.value.intg;
}

/* Size the result to n elements; prints the size before and after. */
static void size(const char *tag, const vhpiCbDataT *cb, vhpiFormatT f, int n) {
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = f;
  v.numElems = n;
  int before = vhpi_get(vhpiSizeP, cb->obj);
  int e0 = err();
  int rc = vhpi_put_value(cb->obj, &v, vhpiSizeConstraint);
  int e = err();
  T("%s size before=%d err=%d constraint rc=%d err=%d after=%d\n", tag, before, e0, rc, e,
    vhpi_get(vhpiSizeP, cb->obj));
}

static void put(const char *tag, const vhpiCbDataT *cb, vhpiValueT *v) {
  int rc = vhpi_put_value(cb->obj, v, vhpiDepositPropagate);
  T("%s put rc=%d err=%d\n", tag, rc, err());
}

static void rev_exec(const vhpiCbDataT *cb) {
  static vhpiEnumT in[64], out[64];
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiLogicVecVal;
  v.bufSize = sizeof in;
  v.value.enumvs = in;
  int rc = vhpi_get_value(param(cb, 0), &v);
  int n = v.numElems;
  T("rev get rc=%d n=%d\n", rc, n);
  size("rev", cb, vhpiLogicVecVal, n);
  for (int i = 0; i < n; i++) out[i] = in[n - 1 - i];
  v.value.enumvs = out;
  put("rev", cb, &v);
}

static void iota_exec(const vhpiCbDataT *cb) {
  static vhpiIntT out[64];
  int n = get_int(cb, 0);
  size("iota", cb, vhpiIntVecVal, n);
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  for (int i = 0; i < n; i++) out[i] = 10 * i;
  v.format = vhpiIntVecVal;
  v.numElems = n;
  v.bufSize = n * sizeof(vhpiIntT);
  v.value.intgs = out;
  put("iota", cb, &v);
}

static void word_exec(const vhpiCbDataT *cb) {
  static char out[8] = "abc";
  size("word", cb, vhpiStrVal, get_int(cb, 0));
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiStrVal;
  v.numElems = 3;
  v.bufSize = sizeof out;
  v.value.str = (vhpiCharT *)out;
  put("word", cb, &v);
}

static void nosize_exec(const vhpiCbDataT *cb) {
  static vhpiIntT out[3] = {7, 8, 9};
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiIntVecVal;
  v.numElems = 3;
  v.bufSize = sizeof out;
  v.value.intgs = out;
  int n = vhpi_get(vhpiSizeP, cb->obj);
  T("nosize size=%d err=%d\n", n, err());
  put("nosize", cb, &v);
}

static void fixed_exec(const vhpiCbDataT *cb) {
  static vhpiEnumT out[4] = {vhpi0, vhpi1, vhpi0, vhpi1};
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  int n = vhpi_get(vhpiSizeP, cb->obj);
  T("fixed size=%d err=%d\n", n, err());
  v.format = vhpiLogicVecVal;
  v.numElems = 4;
  v.bufSize = sizeof out;
  v.value.enumvs = out;
  put("fixed", cb, &v);
}

/* Each design has some of these signals. */
static void result(const char *name, vhpiFormatT f) {
  vhpiHandleT h = lookup(name);
  err();
  if (h) show(h, f, name);
}

static void results(const vhpiCbDataT *cb) {
  (void)cb;
  now("results");
  result("s_rev", vhpiBinStrVal);
  result("s_len", vhpiIntVal);
  result("s_left", vhpiIntVal);
  result("s_right", vhpiIntVal);
  result("s_iota", vhpiIntVecVal);
  result("s_word", vhpiStrVal);
  result("s_nosz", vhpiIntVecVal);
  result("s_fixed", vhpiBinStrVal);
}

static void start(const vhpiCbDataT *cb) {
  (void)cb;
  static vhpiTimeT t = {0, 5000000}; /* 5 ns in fs, scaled below */
  vhpiPhysT r = vhpi_get_phys(vhpiResolutionLimitP, NULL);
  long long res = ((long long)r.high << 32) | r.low;
  long long d = 5000000LL / (res ? res : 1);
  t.high = (uint32_t)(d >> 32);
  t.low = (uint32_t)d;
  vhpiCbDataT c;
  memset(&c, 0, sizeof c);
  c.reason = vhpiCbAfterDelay;
  c.cb_rtn = results;
  c.time = &t;
  vhpi_register_cb(&c, 0);
}

static void reg(vhpiForeignKindT kind, char *name, void (*exec)(const vhpiCbDataT *)) {
  vhpiForeignDataT d;
  memset(&d, 0, sizeof d);
  d.kind = kind;
  d.libraryName = "fresult";
  d.modelName = name;
  d.execf = exec;
  if (!vhpi_register_foreignf(&d)) T("register %s failed err=%d\n", name, err());
}

static void init(void) {
  reg(vhpiFuncF, "rev", rev_exec);
  reg(vhpiFuncF, "iota", iota_exec);
  reg(vhpiFuncF, "word", word_exec);
  reg(vhpiFuncF, "nosize", nosize_exec);
  reg(vhpiFuncF, "fixed", fixed_exec);
  vhpiCbDataT cb;
  memset(&cb, 0, sizeof cb);
  cb.reason = vhpiCbStartOfSimulation;
  cb.cb_rtn = start;
  vhpi_register_cb(&cb, 0);
}

PLUGIN_INIT(init)
