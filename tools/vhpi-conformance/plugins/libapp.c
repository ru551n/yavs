/* Library and application foreign models (vhpiLibF, vhpiAppF): whether and when their
 * elaboration and execution functions run (against the phase callbacks), and what they get as
 * obj. A function model in the same library is called by the design, so a simulator that
 * loads the library through the design's foreign attribute loads it.
 * SPDX-License-Identifier: MIT OR Apache-2.0 */
#include "common.h"

static void report(const char *what, const vhpiCbDataT *cb) {
  vhpiForeignDataT d;
  int rc = cb->obj ? vhpi_get_foreignf_info(cb->obj, &d) : -1;
  T("%s obj=%d kind=%d info=%d\n", what, cb->obj != NULL, cb->obj ? vhpi_get(vhpiKindP, cb->obj) : -1, rc);
  now(what);
}

static void lib_elab(const vhpiCbDataT *cb) { report("lib elabf", cb); }
static void lib_exec(const vhpiCbDataT *cb) { report("lib execf", cb); }
static void app_elab(const vhpiCbDataT *cb) { report("app elabf", cb); }
static void app_exec(const vhpiCbDataT *cb) { report("app execf", cb); }

static void f_elab(const vhpiCbDataT *cb) { report("func elabf", cb); }
static void f_exec(const vhpiCbDataT *cb) {
  report("func execf", cb);
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiIntVal;
  v.value.intg = 7;
  vhpi_put_value(cb->obj, &v, vhpiDepositPropagate);
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

static void reg(int kind, const char *lib, const char *model, void (*e)(const struct vhpiCbDataS *), void (*x)(const struct vhpiCbDataS *), const char *tag) {
  vhpiForeignDataT d;
  memset(&d, 0, sizeof d);
  d.kind = kind;
  d.libraryName = (char *)lib;
  d.modelName = (char *)model;
  d.elabf = e;
  d.execf = x;
  vhpiHandleT h = vhpi_register_foreignf(&d);
  T("register %s %d err=%d\n", tag, h != NULL, err());
}

static void init(void) {
  T("startup\n");
  reg(vhpiLibF, "libapp", "lib", lib_elab, lib_exec, "lib");
  reg(vhpiAppF, "libapp", "app", app_elab, app_exec, "app");
  reg(vhpiFuncF, "libapp", "f", f_elab, f_exec, "func");
  on(vhpiCbStartOfAnalysis, "start of analysis");
  on(vhpiCbEndOfAnalysis, "end of analysis");
  on(vhpiCbStartOfElaboration, "start of elaboration");
  on(vhpiCbEndOfElaboration, "end of elaboration");
  on(vhpiCbStartOfInitialization, "start of initialization");
  on(vhpiCbEndOfInitialization, "end of initialization");
  on(vhpiCbStartOfSimulation, "start of simulation");
  on(vhpiCbEndOfSimulation, "end of simulation");
}

PLUGIN_INIT(init)
