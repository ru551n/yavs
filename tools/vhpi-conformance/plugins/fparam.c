/* Foreign subprograms with record, access and signal parameters: the parameter kinds, how a
 * record's fields and an access value's designated object are reached and written, and how an
 * out signal parameter is driven (vhpi_put_value, vhpi_schedule_transaction on the parameter,
 * on a driver from vhpiDrivers). The design's result signals are traced with value-change
 * callbacks.
 * SPDX-License-Identifier: MIT OR Apache-2.0 */
#include "common.h"

static vhpiHandleT param(const vhpiCbDataT *cb, int i) {
  return vhpi_handle_by_index(vhpiParamDecls, cb->obj, i);
}

static void describe(const char *tag, vhpiHandleT p) {
  T("%s kind=%d mode=%d name=%s composite=%d err=%d\n", tag, vhpi_get(vhpiKindP, p),
    vhpi_get(vhpiModeP, p), S(vhpiNameP, p), vhpi_get(vhpiIsCompositeP, p), err());
}

static int get_int(vhpiHandleT h) {
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiIntVal;
  int rc = vhpi_get_value(h, &v);
  return rc ? -1000 + err() : v.value.intg;
}

static void put_int(const char *tag, vhpiHandleT h, int x) {
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiIntVal;
  v.value.intg = x;
  int rc = vhpi_put_value(h, &v, vhpiDepositPropagate);
  T("%s put %d rc=%d err=%d\n", tag, x, rc, err());
}

static void put_bin(const char *tag, vhpiHandleT h, const char *bits) {
  static char buf[16];
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  snprintf(buf, sizeof buf, "%s", bits);
  v.format = vhpiBinStrVal;
  v.numElems = strlen(buf);
  v.bufSize = sizeof buf;
  v.value.str = (vhpiCharT *)buf;
  int rc = vhpi_put_value(h, &v, vhpiDepositPropagate);
  T("%s put '%s' rc=%d err=%d\n", tag, bits, rc, err());
}

/* The fields of a record parameter, by iterator and by index. */
static void fields(const char *tag, vhpiHandleT r) {
  vhpiHandleT it = vhpi_iterator(vhpiSelectedNames, r), f;
  T("%s iterator %d err=%d\n", tag, it != NULL, err());
  while (it && (f = vhpi_scan(it))) {
    T("%s field kind=%d name=%s\n", tag, vhpi_get(vhpiKindP, f), S(vhpiNameP, f));
    show(f, vhpiBinStrVal, "bin");
  }
  vhpiHandleT i = vhpi_handle_by_index(vhpiSelectedNames, r, 0);
  T("%s by index 0 %d err=%d value=%d\n", tag, i != NULL, err(), i ? get_int(i) : -1);
  vhpiHandleT n = vhpi_handle_by_name("v", r);
  T("%s by name v %d err=%d\n", tag, n != NULL, err());
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiObjTypeVal;
  int rc = vhpi_get_value(r, &v);
  T("%s whole rc=%d err=%d fmt=%d\n", tag, rc, err(), (int)v.format);
}

static void recp_exec(const vhpiCbDataT *cb) {
  vhpiHandleT r = param(cb, 0), o = param(cb, 1);
  describe("recp r", r);
  describe("recp o", o);
  fields("recp r", r);
  vhpiHandleT oi = vhpi_handle_by_index(vhpiSelectedNames, o, 0);
  vhpiHandleT ol = vhpi_handle_by_index(vhpiSelectedNames, o, 1);
  vhpiHandleT ov = vhpi_handle_by_index(vhpiSelectedNames, o, 2);
  T("recp o fields %d %d %d err=%d\n", oi != NULL, ol != NULL, ov != NULL, err());
  if (oi) put_int("recp o.i", oi, get_int(vhpi_handle_by_index(vhpiSelectedNames, r, 0)) + 1);
  if (ol) put_bin("recp o.l", ol, "0");
  if (ov) put_bin("recp o.v", ov, "1001");
  /* The fields of o from the iterator. */
  vhpiHandleT it = vhpi_iterator(vhpiSelectedNames, o), f;
  int k = 0;
  while (it && (f = vhpi_scan(it))) {
    if (k == 0) put_int("recp o.i (iterator)", f, 43);
    if (k == 1) put_bin("recp o.l (iterator)", f, "1");
    if (k == 2) put_bin("recp o.v (iterator)", f, "0110");
    k++;
  }
}

static void recf_exec(const vhpiCbDataT *cb) {
  vhpiHandleT r = param(cb, 0);
  describe("recf r", r);
  vhpiHandleT i = vhpi_handle_by_index(vhpiSelectedNames, r, 0);
  put_int("recf result", cb->obj, i ? get_int(i) * 2 : -2);
}

static void access(const char *tag, vhpiHandleT p) {
  describe(tag, p);
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiObjTypeVal;
  int rc = vhpi_get_value(p, &v);
  T("%s value rc=%d err=%d fmt=%d null=%d\n", tag, rc, err(), (int)v.format,
    rc == 0 && v.format == vhpiPtrVal ? v.value.ptr == NULL : -1);
  vhpiHandleT d = vhpi_handle(vhpiDerefObj, p);
  T("%s deref %d err=%d kind=%d\n", tag, d != NULL, err(), d ? vhpi_get(vhpiKindP, d) : -1);
  if (d) T("%s deref value %d\n", tag, get_int(d));
}

/* Doubles the designated integer into r and increments it; r is -2 for null. */
static void accp_exec(const vhpiCbDataT *cb) {
  vhpiHandleT p = param(cb, 0);
  access("accp p", p);
  vhpiHandleT d = vhpi_handle(vhpiDerefObj, p);
  err();
  int x = d ? get_int(d) : 0;
  put_int("accp r", param(cb, 1), d ? x * 2 : -2);
  if (d) put_int("accp deref", d, x + 1);
}

/* The number of '1' elements of a signal parameter of mode in. */
static void ones_exec(const vhpiCbDataT *cb) {
  vhpiHandleT s = param(cb, 0);
  describe("ones s", s);
  static vhpiEnumT bits[16];
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiLogicVecVal;
  v.bufSize = sizeof bits;
  v.value.enumvs = bits;
  int rc = vhpi_get_value(s, &v), n = 0;
  T("ones get rc=%d err=%d n=%d\n", rc, err(), (int)v.numElems);
  for (int i = 0; i < v.numElems; i++) n += bits[i] == vhpi1;
  put_int("ones r", param(cb, 1), n);
}

static void tm(vhpiTimeT *t, long long fs) {
  vhpiPhysT r = vhpi_get_phys(vhpiResolutionLimitP, NULL);
  long long res = ((long long)r.high << 32) | r.low;
  long long d = fs / (res ? res : 1);
  t->high = (uint32_t)(d >> 32);
  t->low = (uint32_t)d;
}

static int calls;

static void sigp_exec(const vhpiCbDataT *cb) {
  vhpiHandleT s = param(cb, 0), o = param(cb, 1), k = param(cb, 2);
  now("sigp");
  calls++;
  describe("sigp s", s);
  describe("sigp o", o);
  show(s, vhpiBinStrVal, "s");
  show(o, vhpiBinStrVal, "o");
  if (calls == 1) {
    /* Deposit into the out signal parameter. */
    put_bin("sigp o", o, "1010");
    /* A transaction on the parameter itself, 2 ns from now. */
    vhpiValueT v;
    memset(&v, 0, sizeof v);
    v.format = vhpiIntVal;
    v.value.intg = 7;
    vhpiTimeT t;
    tm(&t, 2000000);
    int rc = vhpi_schedule_transaction(k, &v, 1, &t, vhpiInertial, NULL);
    T("sigp k schedule rc=%d err=%d\n", rc, err());
  } else {
    /* A transaction on a driver of the parameter. */
    vhpiHandleT it = vhpi_iterator(vhpiDrivers, o);
    vhpiHandleT d = it ? vhpi_scan(it) : NULL;
    T("sigp o drivers %d driver %d err=%d\n", it != NULL, d != NULL, err());
    static char bits[] = "0011";
    vhpiValueT v;
    memset(&v, 0, sizeof v);
    v.format = vhpiBinStrVal;
    v.numElems = 4;
    v.bufSize = sizeof bits;
    v.value.str = (vhpiCharT *)bits;
    vhpiTimeT t;
    tm(&t, 1000000);
    int rc = vhpi_schedule_transaction(d ? d : o, &v, 1, &t, vhpiTransport, NULL);
    T("sigp o schedule rc=%d err=%d\n", rc, err());
  }
}

static void trace_cb(const vhpiCbDataT *cb) {
  vhpiTimeT t;
  long cyc = 0;
  vhpi_get_time(&t, &cyc);
  const char *tag = (const char *)cb->user_data;
  show(cb->obj, tag[strlen(tag) - 1] == 'v' || !strcmp(tag, "o") ? vhpiBinStrVal : vhpiIntVal, tag);
  T("  at %lldfs\n", fs_of(&t));
}

static void watch(const char *name) {
  static vhpiTimeT t;
  static vhpiValueT val;
  static char buf[64];
  vhpiHandleT h = lookup(name);
  err();
  if (!h) return;
  vhpiCbDataT cb;
  memset(&cb, 0, sizeof cb);
  val.format = vhpiBinStrVal;
  val.bufSize = sizeof buf;
  val.value.str = (vhpiCharT *)buf;
  cb.reason = vhpiCbValueChange;
  cb.cb_rtn = trace_cb;
  cb.obj = h;
  cb.time = &t;
  cb.value = &val;
  cb.user_data = (void *)name;
  vhpi_register_cb(&cb, 0);
}

static void start(const vhpiCbDataT *cb) {
  (void)cb;
  const char *names[] = {"s_o.i", "s_o.l", "s_o.v", "s_f", "s_v", "s_n", "o", "k"};
  for (unsigned i = 0; i < sizeof names / sizeof *names; i++) watch(names[i]);
}

static void reg(vhpiForeignKindT kind, char *name, void (*exec)(const vhpiCbDataT *)) {
  vhpiForeignDataT d;
  memset(&d, 0, sizeof d);
  d.kind = kind;
  d.libraryName = "fparam";
  d.modelName = name;
  d.execf = exec;
  if (!vhpi_register_foreignf(&d)) T("register %s failed err=%d\n", name, err());
}

static void init(void) {
  reg(vhpiProcF, "recp", recp_exec);
  reg(vhpiFuncF, "recf", recf_exec);
  reg(vhpiProcF, "accp", accp_exec);
  reg(vhpiProcF, "sigp", sigp_exec);
  reg(vhpiProcF, "ones", ones_exec);
  vhpiCbDataT cb;
  memset(&cb, 0, sizeof cb);
  cb.reason = vhpiCbStartOfSimulation;
  cb.cb_rtn = start;
  vhpi_register_cb(&cb, 0);
}

PLUGIN_INIT(init)
