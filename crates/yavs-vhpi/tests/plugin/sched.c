/* Test VHPI plugin for simulation-phase and after-delay callbacks (#272),
 * loaded by `yavs run --vhpi=LIB:ENTRY` with one entry point per scenario.
 * SPDX-License-Identifier: MIT OR Apache-2.0 */

#include <stdio.h>
#include <string.h>

#include "vhpi_user.h"

#ifdef _WIN32
#define PLUGIN_EXPORT __declspec(dllexport)
#else
#define PLUGIN_EXPORT
#endif

static unsigned now(long *cycles) {
  vhpiTimeT t;
  vhpi_get_time(&t, cycles);
  return t.low;
}

static void say(const vhpiCbDataT *cb) {
  long d;
  unsigned t = now(&d);
  vhpi_printf("vhpi: %s t=%u d=%ld cb_time=%u\n", (const char *)cb->user_data, t,
              d, cb->time->low);
}

static vhpiHandleT reg(int reason, unsigned delay, void (*rtn)(const vhpiCbDataT *),
                       const char *name, int flags) {
  vhpiCbDataT cb;
  vhpiTimeT t = {0, delay};
  memset(&cb, 0, sizeof cb);
  cb.reason = reason;
  cb.cb_rtn = rtn;
  cb.time = &t;
  cb.user_data = (void *)name;
  return vhpi_register_cb(&cb, flags);
}

static void finish(const vhpiCbDataT *cb) {
  say(cb);
  vhpi_control(vhpiFinish);
}

static void end_of_sim(const vhpiCbDataT *cb) { say(cb); }

/* timed_finish: an after-delay callback registered before the simulation
 * starts ends it at T (in units of the resolution limit). */
PLUGIN_EXPORT void timed_finish(void) {
  vhpiPhysT res = vhpi_get_phys(vhpiResolutionLimitP, NULL);
  vhpi_printf("vhpi: resolution=%u\n", res.low);
  reg(vhpiCbAfterDelay, 30, finish, "finish", 0);
  reg(vhpiCbEndOfSimulation, 0, end_of_sim, "end", 0);
}

/* chain: each after-delay callback registers the next; a repeating one is
 * removed after three firings. The run ends idle after the last one. */
static int links, reps;
static vhpiHandleT rep_h;

static void chain_link(const vhpiCbDataT *cb) {
  say(cb);
  if (++links < 4)
    reg(vhpiCbAfterDelay, 10, chain_link, "link", 0);
}

static void rep(const vhpiCbDataT *cb) {
  say(cb);
  if (++reps == 3)
    vhpi_printf("vhpi: remove rep=%d\n", vhpi_remove_cb(rep_h));
}

static void chain_start(const vhpiCbDataT *cb) {
  say(cb);
  reg(vhpiCbAfterDelay, 10, chain_link, "link", 0);
  rep_h = reg(vhpiCbRepAfterDelay, 7, rep, "rep", vhpiReturnCb);
  /* Removing a pending one-shot cancels it: it never fires and does not
     keep the run alive. */
  vhpiHandleT h = reg(vhpiCbAfterDelay, 1000, chain_link, "never", vhpiReturnCb);
  vhpi_printf("vhpi: remove pending=%d\n", vhpi_remove_cb(h));
}

PLUGIN_EXPORT void chain(void) {
  reg(vhpiCbStartOfSimulation, 0, chain_start, "start", 0);
  reg(vhpiCbEndOfSimulation, 0, end_of_sim, "end", 0);
}

/* order: every phase callback once per occurrence, in cycle order. The
 * read-write phase (last known delta) creates a delta once with an
 * after-delay of 0; a callback registered in the read-only phase (end of
 * time step) for the same phase fires at the next one. */
static int lkd_done, eots_done;

static void lkd(const vhpiCbDataT *cb) {
  say(cb);
  if (!lkd_done++)
    reg(vhpiCbAfterDelay, 0, say, "after0", 0);
}

static void eots(const vhpiCbDataT *cb) {
  say(cb);
  if (!eots_done++)
    reg(vhpiCbEndOfTimeStep, 0, say, "late_eots", 0);
}

PLUGIN_EXPORT void order(void) {
  reg(vhpiCbStartOfSimulation, 0, say, "start_of_sim", 0);
  reg(vhpiCbStartOfInitialization, 0, say, "start_of_init", 0);
  reg(vhpiCbEndOfInitialization, 0, say, "end_of_init", 0);
  reg(vhpiCbNextTimeStep, 0, say, "next_time_step_once", 0);
  reg(vhpiCbRepNextTimeStep, 0, say, "next_time_step", vhpiReturnCb);
  reg(vhpiCbRepStartOfNextCycle, 0, say, "next_cycle", 0);
  reg(vhpiCbRepStartOfProcesses, 0, say, "start_procs", 0);
  reg(vhpiCbRepEndOfProcesses, 0, say, "end_procs", 0);
  reg(vhpiCbRepLastKnownDeltaCycle, 0, lkd, "read_write", 0);
  reg(vhpiCbRepStartOfPostponed, 0, say, "start_postponed", 0);
  reg(vhpiCbRepEndOfTimeStep, 0, eots, "read_only", 0);
  reg(vhpiCbEndOfSimulation, 0, end_of_sim, "end", 0);
}

/* toggle: a disabled repeating callback is enabled at 20 and disabled at 40
 * by after-delay callbacks; 100000 disabled callbacks that are never
 * removed ride along. */
static vhpiHandleT nts_h, som_h;
static int ticks;

static void state(const char *what) {
  vhpi_printf("vhpi: %s state=%d\n", what, (int)vhpi_get(vhpiStateP, nts_h));
}

static void tick(const vhpiCbDataT *cb) { (void)cb; ticks++; }

static void enable(const vhpiCbDataT *cb) {
  say(cb);
  vhpi_printf("vhpi: enable=%d\n", vhpi_enable_cb(nts_h));
  state("enabled");
  /* A matured one-shot can still be removed. */
  int st = (int)vhpi_get(vhpiStateP, som_h);
  vhpi_printf("vhpi: matured state=%d remove=%d\n", st, vhpi_remove_cb(som_h));
}

static void disable(const vhpiCbDataT *cb) {
  say(cb);
  vhpi_printf("vhpi: disable=%d\n", vhpi_disable_cb(nts_h));
  state("disabled");
}

static void stop(const vhpiCbDataT *cb) {
  say(cb);
  vhpi_printf("vhpi: ticks=%d\n", ticks);
  vhpi_control(vhpiFinish);
}

PLUGIN_EXPORT void toggle(void) {
  nts_h = reg(vhpiCbRepNextTimeStep, 0, say, "next_time_step",
              vhpiReturnCb | vhpiDisableCb);
  state("registered");
  vhpiCbDataT info;
  vhpi_printf("vhpi: get_cb_info=%d\n", vhpi_get_cb_info(nts_h, &info));
  vhpi_printf("vhpi: info reason=%d user_data=%s\n", info.reason,
              (const char *)info.user_data);
  for (int i = 0; i < 100000; i++)
    reg(vhpiCbRepStartOfNextCycle, 0, say, "never", vhpiReturnCb | vhpiDisableCb);
  reg(vhpiCbRepAfterDelay, 5, tick, "tick", 0);
  reg(vhpiCbAfterDelay, 20, enable, "enable", 0);
  reg(vhpiCbAfterDelay, 40, disable, "disable", 0);
  reg(vhpiCbAfterDelay, 60, stop, "stop", 0);
  som_h = reg(vhpiCbStartOfSimulation, 0, say, "start_of_sim", vhpiReturnCb);
}

/* errors: misuse is reported, not fatal. */
PLUGIN_EXPORT void errors(void) {
  vhpiErrorInfoT e;
  vhpiCbDataT cb;
  memset(&cb, 0, sizeof cb);
  cb.reason = vhpiCbAfterDelay;
  cb.cb_rtn = say;
  vhpiHandleT none = vhpi_register_cb(&cb, vhpiReturnCb);
  /* Read the error before any other VHPI call clears it. */
  if (vhpi_check_error(&e))
    vhpi_printf("vhpi: error: %s\n", e.message);
  vhpi_printf("vhpi: no time=%s\n", none ? "handle" : "null");
  vhpiHandleT h = reg(vhpiCbStartOfSimulation, 0, say, "start_of_sim", vhpiReturnCb);
  vhpi_printf("vhpi: remove=%d\n", vhpi_remove_cb(h));
  vhpi_printf("vhpi: remove again=%d\n", vhpi_remove_cb(h));
}
