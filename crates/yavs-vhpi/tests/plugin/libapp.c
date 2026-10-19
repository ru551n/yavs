/* Test VHPI plugin: library and application foreign models (#357). The library model of
 * library "used" runs when a foreign model of that library is bound; the one of "idle"
 * never runs; the application model runs at the start of the simulation whatever the
 * design. Each function prints what it got and the time.
 * SPDX-License-Identifier: MIT OR Apache-2.0 */

#include <stdio.h>
#include <string.h>

#include "vhpi_user.h"

#ifdef _WIN32
#define PLUGIN_EXPORT __declspec(dllexport)
#else
#define PLUGIN_EXPORT
#endif

static void say(const char *what, const vhpiCbDataT *cb) {
  vhpiForeignDataT d;
  int rc = vhpi_get_foreignf_info(cb->obj, &d);
  vhpiTimeT t;
  long cyc = 0;
  vhpi_get_time(&t, &cyc);
  vhpi_printf("%s kind=%d info=%d foreignk=%d t=%u\n", what, vhpi_get(vhpiKindP, cb->obj), rc,
              vhpi_get(vhpiForeignKindP, cb->obj), t.low);
}

static void used_elab(const vhpiCbDataT *cb) { say("used elabf", cb); }
static void used_exec(const vhpiCbDataT *cb) { say("used execf", cb); }
static void idle_elab(const vhpiCbDataT *cb) { say("idle elabf", cb); }
static void idle_exec(const vhpiCbDataT *cb) { say("idle execf", cb); }
static void app_elab(const vhpiCbDataT *cb) { say("app elabf", cb); }
static void app_exec(const vhpiCbDataT *cb) { say("app execf", cb); }

static void one_exec(const vhpiCbDataT *cb) {
  say("one execf", cb);
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiIntVal;
  v.value.intg = 1;
  vhpi_put_value(cb->obj, &v, vhpiDepositPropagate);
}

static void start(const vhpiCbDataT *cb) {
  (void)cb;
  vhpi_printf("start of simulation callback\n");
}

static void reg(int kind, const char *lib, const char *model, void (*e)(const vhpiCbDataT *),
                void (*x)(const vhpiCbDataT *)) {
  vhpiForeignDataT d;
  memset(&d, 0, sizeof d);
  d.kind = kind;
  d.libraryName = (char *)lib;
  d.modelName = (char *)model;
  d.elabf = e;
  d.execf = x;
  if (!vhpi_register_foreignf(&d)) vhpi_printf("register %s failed\n", model);
}

static void init(void) {
  reg(vhpiLibF, "used", "lib", used_elab, used_exec);
  reg(vhpiLibF, "idle", "lib", idle_elab, idle_exec);
  reg(vhpiAppF, "app", "app", app_elab, app_exec);
  reg(vhpiFuncF, "used", "one", NULL, one_exec);
  vhpiCbDataT cb;
  memset(&cb, 0, sizeof cb);
  cb.reason = vhpiCbStartOfSimulation;
  cb.cb_rtn = start;
  vhpi_register_cb(&cb, 0);
}

PLUGIN_EXPORT void (*vhpi_startup_routines[])(void) = {init, NULL};
