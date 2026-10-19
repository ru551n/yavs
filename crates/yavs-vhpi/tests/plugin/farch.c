/* Test VHPI plugin: the foreign architecture "VHPI plug inv" of tests/farch/design.vhd.
 * elabf creates a process and drivers for the out ports (vhpi_create); execf watches the in
 * port and drives y <= not a after the generic delay (inertial) and n <= events after 3 ns
 * (transport) with vhpi_schedule_transaction.
 * SPDX-License-Identifier: MIT OR Apache-2.0 */

#include <stdio.h>
#include <string.h>

#include "vhpi_user.h"

#ifdef _WIN32
#define PLUGIN_EXPORT __declspec(dllexport)
#else
#define PLUGIN_EXPORT
#endif

/* One model instance. */
struct inst {
  vhpiHandleT a, y_drv, n_drv;
  int delay_ns, events;
};

static struct inst insts[4];
static int ninsts;

static vhpiTimeT ns(int n) {
  vhpiTimeT t;
  long long fs = (long long)n * 1000000;
  t.high = (int)(fs >> 32);
  t.low = (unsigned)fs;
  return t;
}

static int generic(vhpiHandleT inst, const char *name) {
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiIntVal;
  vhpi_get_value(vhpi_handle_by_name(name, inst), &v);
  return v.value.intg;
}

static void a_changed(const vhpiCbDataT *cb) {
  struct inst *m = cb->user_data;
  vhpiEnumT bits[4];
  vhpiValueT v;
  vhpiTimeT t;
  int i;
  memset(&v, 0, sizeof v);
  v.format = vhpiLogicVecVal;
  v.bufSize = sizeof bits;
  v.value.enumvs = bits;
  vhpi_get_value(m->a, &v);
  for (i = 0; i < 4; i++) bits[i] = bits[i] == vhpi1 ? vhpi0 : vhpi1;
  t = ns(m->delay_ns);
  if (vhpi_schedule_transaction(m->y_drv, &v, 1, &t, vhpiInertial, NULL)) vhpi_printf("y failed\n");
  memset(&v, 0, sizeof v);
  v.format = vhpiIntVal;
  v.value.intg = ++m->events;
  t = ns(3);
  if (vhpi_schedule_transaction(m->n_drv, &v, 1, &t, vhpiTransport, NULL)) vhpi_printf("n failed\n");
}

static void elab(const vhpiCbDataT *cb) {
  struct inst *m = &insts[ninsts++];
  vhpiHandleT inst = cb->obj, p, unit = vhpi_handle(vhpiDesignUnit, inst);
  vhpiForeignDataT d;
  vhpiErrorInfoT e;
  if (vhpi_get_foreignf_info(inst, &d)) vhpi_printf("elab: no info\n");
  vhpi_printf("elab: %s kind=%d model %s %s unit kind=%d\n",
              (const char *)vhpi_get_str(vhpiFullNameP, inst), vhpi_get(vhpiKindP, inst),
              d.libraryName, d.modelName, vhpi_get(vhpiKindP, unit));
  p = vhpi_create(vhpiProcessStmtK, unit, NULL);
  m->a = vhpi_handle_by_name("a", inst);
  m->y_drv = vhpi_create(vhpiDriverK, vhpi_handle_by_name("y", inst), p);
  m->n_drv = vhpi_create(vhpiDriverK, p, vhpi_handle_by_name("n", inst));
  m->delay_ns = generic(inst, "delay_ns");
  vhpi_printf("elab: process kind=%d drivers kind=%d %d delay=%d\n", vhpi_get(vhpiKindP, p),
              vhpi_get(vhpiKindP, m->y_drv), m->n_drv != NULL, m->delay_ns);
  /* An in port has no driver. */
  if (vhpi_create(vhpiDriverK, m->a, p)) vhpi_printf("elab: driver on an in port\n");
  if (vhpi_check_error(&e)) vhpi_printf("elab: %s\n", e.message);
}

static void exec(const vhpiCbDataT *cb) {
  struct inst *m = NULL;
  vhpiCbDataT w;
  vhpiEnumT ones[4] = {vhpi1, vhpi1, vhpi1, vhpi1};
  vhpiValueT v;
  vhpiTimeT t = ns(0);
  int i;
  for (i = 0; i < ninsts; i++)
    if (vhpi_compare_handles(vhpi_handle_by_name("a", cb->obj), insts[i].a)) m = &insts[i];
  if (!m) {
    vhpi_printf("exec: unknown instance\n");
    return;
  }
  memset(&w, 0, sizeof w);
  w.reason = vhpiCbValueChange;
  w.cb_rtn = a_changed;
  w.obj = m->a;
  w.user_data = m;
  vhpi_register_cb(&w, 0);
  /* y starts at "1111". */
  memset(&v, 0, sizeof v);
  v.format = vhpiLogicVecVal;
  v.numElems = 4;
  v.bufSize = sizeof ones;
  v.value.enumvs = ones;
  if (vhpi_schedule_transaction(m->y_drv, &v, 1, &t, vhpiInertial, NULL)) vhpi_printf("exec: y failed\n");
}

/* Creating a process or driver during the simulation is misuse. */
static void late(const vhpiCbDataT *cb) {
  vhpiErrorInfoT e;
  (void)cb;
  if (vhpi_create(vhpiProcessStmtK, vhpi_handle(vhpiRootInst, NULL), NULL)) vhpi_printf("late: created\n");
  if (vhpi_check_error(&e)) vhpi_printf("late: %s\n", e.message);
}

PLUGIN_EXPORT void farch_models(void) {
  vhpiForeignDataT d;
  vhpiCbDataT cb;
  memset(&d, 0, sizeof d);
  d.kind = vhpiArchF;
  d.libraryName = "plug";
  d.modelName = "inv";
  d.elabf = elab;
  d.execf = exec;
  if (!vhpi_register_foreignf(&d)) vhpi_printf("register failed\n");
  memset(&cb, 0, sizeof cb);
  cb.reason = vhpiCbStartOfSimulation;
  cb.cb_rtn = late;
  vhpi_register_cb(&cb, 0);
}

PLUGIN_EXPORT void (*vhpi_startup_routines[])(void) = {farch_models, NULL};
