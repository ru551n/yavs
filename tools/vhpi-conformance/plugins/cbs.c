/* Callback reasons and the order and time they fire in.
 * SPDX-License-Identifier: MIT OR Apache-2.0 */
#include "common.h"

static int limit = 400; /* trace lines, a runaway guard */

static void fired(const vhpiCbDataT *cb) {
  const char *name = (const char *)cb->user_data;
  vhpiTimeT t;
  long cyc = 0;
  vhpi_get_time(&t, &cyc);
  if (limit-- <= 0) return;
  T("cb %-14s now=%lldfs cycles=%ld", name, fs_of(&t), cyc);
  if (cb->value) {
    L(" value=%d", (int)cb->value->format);
    if (cb->value->format == vhpiBinStrVal && cb->value->value.str)
      L(" '%s'", (const char *)cb->value->value.str);
  }
  L("\n");
}

static void reg(int reason, const char *name, vhpiHandleT obj, vhpiTimeT *when, int flags) {
  vhpiCbDataT cb;
  memset(&cb, 0, sizeof cb);
  cb.reason = reason;
  cb.cb_rtn = fired;
  cb.obj = obj;
  cb.time = when;
  cb.user_data = (void *)name;
  static char bin[16];
  static vhpiValueT val;
  if (reason == vhpiCbValueChange) {
    val.format = vhpiBinStrVal;
    val.bufSize = sizeof bin;
    val.value.str = (vhpiCharT *)bin;
    cb.value = &val;
  }
  vhpiHandleT h = vhpi_register_cb(&cb, flags | vhpiReturnCb);
  T("register %-14s null=%d err=%d state=%d\n", name, h == NULL, err(),
    h ? (int)vhpi_get(vhpiStateP, h) : -1);
}

static void finish(const vhpiCbDataT *cb) {
  fired(cb);
  vhpi_control(vhpiFinish);
}

static void sim_start(const vhpiCbDataT *cb) {
  fired(cb);
  vhpiHandleT clk = lookup("clk");
  vhpiTimeT d15 = {0, 0}, d0 = {0, 0};
  d15.low = (uint32_t)(15000000 / vhpi_get_phys(vhpiResolutionLimitP, NULL).low);
  reg(vhpiCbEndOfInitialization, "EndOfInit", NULL, NULL, 0);
  reg(vhpiCbRepNextTimeStep, "NextTimeStep", NULL, NULL, 0);
  reg(vhpiCbRepStartOfNextCycle, "StartNextCycle", NULL, NULL, 0);
  reg(vhpiCbRepStartOfProcesses, "StartProcs", NULL, NULL, 0);
  reg(vhpiCbRepEndOfProcesses, "EndProcs", NULL, NULL, 0);
  reg(vhpiCbRepLastKnownDeltaCycle, "LastKnownDelta", NULL, NULL, 0);
  reg(vhpiCbRepStartOfPostponed, "StartPostponed", NULL, NULL, 0);
  reg(vhpiCbRepEndOfTimeStep, "EndOfTimeStep", NULL, NULL, 0);
  reg(vhpiCbValueChange, "ValueChange", clk, NULL, 0);
  reg(vhpiCbAfterDelay, "After15ns", NULL, &d15, 0);
  reg(vhpiCbAfterDelay, "After0", NULL, &d0, 0);
  reg(vhpiCbRepAfterDelay, "RepAfter15ns", NULL, &d15, 0);
  /* Repeating callbacks keep a run alive: end it at 100 ns. */
  vhpiTimeT d100 = {0, 0};
  d100.low = (uint32_t)(100000000 / vhpi_get_phys(vhpiResolutionLimitP, NULL).low);
  vhpiCbDataT fin;
  memset(&fin, 0, sizeof fin);
  fin.reason = vhpiCbAfterDelay;
  fin.cb_rtn = finish;
  fin.time = &d100;
  fin.user_data = (void *)"Finish100ns";
  vhpi_register_cb(&fin, 0);
}

static void init(void) {
  reg(vhpiCbStartOfInitialization, "StartOfInit", NULL, NULL, 0);
  reg(vhpiCbStartOfSimulation, "StartOfSim", NULL, NULL, 0);
  reg(vhpiCbEndOfSimulation, "EndOfSim", NULL, NULL, 0);
  /* The first start-of-simulation callback registers the rest. */
  vhpiCbDataT cb;
  memset(&cb, 0, sizeof cb);
  cb.reason = vhpiCbStartOfSimulation;
  cb.cb_rtn = sim_start;
  cb.user_data = (void *)"StartOfSim2";
  vhpi_register_cb(&cb, 0);
}

PLUGIN_INIT(init)
