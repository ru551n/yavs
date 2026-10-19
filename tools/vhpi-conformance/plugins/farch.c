/* A foreign architecture: when its elaboration and execution functions run (against the
 * phase callbacks), the instance they get, vhpi_create of a process and of drivers for the
 * out ports, and vhpi_schedule_transaction from a value-change callback on the in port:
 * y <= not a after 2 ns (inertial), n <= count after 5 ns (transport). The testbench's
 * signals are traced with value-change callbacks.
 * SPDX-License-Identifier: MIT OR Apache-2.0 */
#include "common.h"

static vhpiHandleT drv_y, drv_n, port_a;
static int count;

static void tm(vhpiTimeT *t, long long fs) {
  vhpiPhysT r = vhpi_get_phys(vhpiResolutionLimitP, NULL);
  long long res = ((long long)r.high << 32) | r.low;
  long long d = fs / (res ? res : 1);
  t->high = (uint32_t)(d >> 32);
  t->low = (uint32_t)d;
}

static void phase(const vhpiCbDataT *cb) { now((const char *)cb->user_data); }

static void on(int reason, const char *tag) {
  vhpiCbDataT cb;
  memset(&cb, 0, sizeof cb);
  cb.reason = reason;
  cb.cb_rtn = phase;
  cb.user_data = (void *)tag;
  vhpi_register_cb(&cb, 0);
}

static void sched(const char *tag, vhpiHandleT d, vhpiValueT *v, long long fs, vhpiDelayModeT m) {
  vhpiTimeT t;
  tm(&t, fs);
  int rc = vhpi_schedule_transaction(d, v, 1, &t, m, NULL);
  T("schedule %s rc=%d err=%d\n", tag, rc, err());
}

static void a_changed(const vhpiCbDataT *cb) {
  static vhpiEnumT bits[4];
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiLogicVecVal;
  v.bufSize = sizeof bits;
  v.value.enumvs = bits;
  vhpi_get_value(port_a, &v);
  now("model a changed");
  for (int i = 0; i < 4; i++) bits[i] = bits[i] == vhpi1 ? vhpi0 : vhpi1;
  sched("y", drv_y, &v, 2000000, vhpiInertial);
  vhpiValueT w;
  memset(&w, 0, sizeof w);
  w.format = vhpiIntVal;
  w.value.intg = ++count;
  sched("n", drv_n, &w, 5000000, vhpiTransport);
  (void)cb;
}

static void trace_cb(const vhpiCbDataT *cb) {
  vhpiTimeT t;
  long cyc = 0;
  vhpi_get_time(&t, &cyc);
  show(cb->obj, strcmp((const char *)cb->user_data, "n") ? vhpiBinStrVal : vhpiIntVal,
       (const char *)cb->user_data);
  T("  at %lldfs\n", fs_of(&t));
}

static void watch(vhpiHandleT h, void (*f)(const vhpiCbDataT *), const char *tag) {
  static vhpiTimeT t;
  static vhpiValueT val;
  static char buf[64];
  vhpiCbDataT cb;
  memset(&cb, 0, sizeof cb);
  val.format = vhpiBinStrVal;
  val.bufSize = sizeof buf;
  val.value.str = (vhpiCharT *)buf;
  cb.value = &val;
  cb.reason = vhpiCbValueChange;
  cb.cb_rtn = f;
  cb.obj = h;
  cb.time = &t;
  cb.user_data = (void *)tag;
  vhpiHandleT h2 = vhpi_register_cb(&cb, vhpiReturnCb);
  T("watch %s %d err=%d\n", tag, h2 != NULL, err());
}

/* The elaboration function creates the process and the drivers: one reference allows drivers
 * only then. The process belongs to the architecture body (the instance's design unit); the
 * instance itself is tried first. */
static void elab(const vhpiCbDataT *cb) {
  vhpiHandleT inst = cb->obj;
  vhpiForeignDataT d;
  int rc = vhpi_get_foreignf_info(inst, &d);
  T("elabf kind=%d name=%s info rc=%d\n", vhpi_get(vhpiKindP, inst), S(vhpiNameP, inst), rc);
  vhpiHandleT p = vhpi_create(vhpiProcessStmtK, inst, NULL);
  T("create process on instance %d err=%d\n", p != NULL, err());
  vhpiHandleT du = vhpi_handle(vhpiDesignUnit, inst);
  T("design unit kind=%d\n", du ? vhpi_get(vhpiKindP, du) : -1);
  if (!p) {
    p = vhpi_create(vhpiProcessStmtK, du, NULL);
    T("create process on design unit %d err=%d\n", p != NULL, err());
  }
  T("process kind=%d\n", p ? vhpi_get(vhpiKindP, p) : -1);
  port_a = vhpi_handle_by_name("a", inst);
  vhpiHandleT y = vhpi_handle_by_name("y", inst), n = vhpi_handle_by_name("n", inst);
  T("ports a=%d y=%d n=%d err=%d\n", port_a != NULL, y != NULL, n != NULL, err());
  /* The signal, then the process; the other order is tried when that fails. */
  drv_y = vhpi_create(vhpiDriverK, y, p);
  T("create driver y %d err=%d kind=%d\n", drv_y != NULL, err(), drv_y ? vhpi_get(vhpiKindP, drv_y) : -1);
  if (!drv_y) {
    drv_y = vhpi_create(vhpiDriverK, p, y);
    T("create driver y (process first) %d err=%d\n", drv_y != NULL, err());
  }
  drv_n = vhpi_create(vhpiDriverK, n, p);
  if (!drv_n) drv_n = vhpi_create(vhpiDriverK, p, n);
  T("create driver n %d err=%d\n", drv_n != NULL, err());
}

/* The execution function watches the in port and drives y at once. */
static void exec(const vhpiCbDataT *cb) {
  T("execf kind=%d name=%s\n", vhpi_get(vhpiKindP, cb->obj), S(vhpiNameP, cb->obj));
  vhpiHandleT late = vhpi_create(vhpiProcessStmtK, vhpi_handle(vhpiDesignUnit, cb->obj), NULL);
  T("create process in execf %d err=%d\n", late != NULL, err());
  late = vhpi_create(vhpiDriverK, vhpi_handle_by_name("z", cb->obj), late);
  T("create driver in execf %d err=%d\n", late != NULL, err());
  watch(port_a, a_changed, "a");
  static vhpiEnumT ones[4] = {vhpi1, vhpi1, vhpi1, vhpi1};
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiLogicVecVal;
  v.numElems = 4;
  v.bufSize = sizeof ones;
  v.value.enumvs = ones;
  sched("y initial", drv_y, &v, 0, vhpiInertial);
}

static void start(const vhpiCbDataT *cb) {
  (void)cb;
  now("start of simulation");
  watch(lookup("y"), trace_cb, "y");
  watch(lookup("n"), trace_cb, "n");
}

static void init(void) {
  vhpiForeignDataT d;
  memset(&d, 0, sizeof d);
  d.kind = vhpiArchF;
  d.libraryName = "farch";
  d.modelName = "inv";
  d.elabf = elab;
  d.execf = exec;
  if (!vhpi_register_foreignf(&d)) T("register failed err=%d\n", err());
  vhpiCbDataT cb;
  memset(&cb, 0, sizeof cb);
  cb.reason = vhpiCbStartOfSimulation;
  cb.cb_rtn = start;
  vhpi_register_cb(&cb, 0);
  on(vhpiCbEndOfInitialization, "end of initialization");
}

PLUGIN_INIT(init)
