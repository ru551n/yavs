/* Test VHPI plugin. Built as a shared library against include/vhpi_user.h
 * and nothing else: on Linux the vhpi_* symbols stay unresolved and bind
 * to the host executable at load time; on Windows the test links it
 * against the host's import library.
 * SPDX-License-Identifier: MIT OR Apache-2.0 */

#include <stdio.h>
#include <string.h>

#include "vhpi_user.h"

/* The IEEE header undefines its PLI_DLLESPEC helper at the end. */
#ifdef _WIN32
#define PLUGIN_EXPORT __declspec(dllexport)
#else
#define PLUGIN_EXPORT
#endif

/* Counts every symbol the simulator exports. Referencing them all makes
 * the load (RTLD_NOW on Linux, the import table on Windows) fail if any
 * is missing from the executable. The array is automatic because the
 * address of an imported data symbol is not a constant on Windows. */
static int count_exports(void) {
  void *const every_export[] = {
    (void *)vhpi_assert,         (void *)vhpi_check_error,
    (void *)vhpi_compare_handles, (void *)vhpi_control,
    (void *)vhpi_create,         (void *)vhpi_disable_cb,
    (void *)vhpi_enable_cb,      (void *)vhpi_format_value,
    (void *)vhpi_get,            (void *)vhpi_get_cb_info,
    (void *)vhpi_get_data,       (void *)vhpi_get_foreignf_info,
    (void *)vhpi_get_next_time,  (void *)vhpi_get_phys,
    (void *)vhpi_get_real,       (void *)vhpi_get_str,
    (void *)vhpi_get_time,       (void *)vhpi_get_value,
    (void *)vhpi_handle,         (void *)vhpi_handle_by_index,
    (void *)vhpi_handle_by_name, (void *)vhpi_is_printable,
    (void *)vhpi_iterator,       (void *)vhpi_printf,
    (void *)vhpi_protected_call, (void *)vhpi_put_data,
    (void *)vhpi_put_value,      (void *)vhpi_register_cb,
    (void *)vhpi_register_foreignf, (void *)vhpi_release_handle,
    (void *)vhpi_remove_cb,      (void *)vhpi_scan,
    (void *)vhpi_schedule_transaction, (void *)vhpi_vprintf,
    (void *)&vhpiFS, (void *)&vhpiPS, (void *)&vhpiNS, (void *)&vhpiUS,
    (void *)&vhpiMS, (void *)&vhpiS,  (void *)&vhpiMN, (void *)&vhpiHR,
  };
  int n = 0;
  for (size_t i = 0; i < sizeof every_export / sizeof every_export[0]; i++)
    n += every_export[i] != NULL;
  return n;
}

static void report_error(const char *what) {
  vhpiErrorInfoT info;
  if (vhpi_check_error(&info))
    vhpi_printf("plugin: %s: error severity=%d: %s\n", what, info.severity,
                info.message);
  else
    vhpi_printf("plugin: %s: no error\n", what);
}

static void end_of_sim(const vhpiCbDataT *cb) {
  vhpi_printf("plugin: end of simulation, user_data=%s\n",
              (const char *)cb->user_data);
}

static void start_of_sim(const vhpiCbDataT *cb) {
  vhpiHandleT root = vhpi_handle(vhpiRootInst, NULL);
  vhpiHandleT by_name = vhpi_handle_by_name("top", NULL);
  vhpi_printf("plugin: start of simulation, reason=%d time=%d/%u\n",
              cb->reason, cb->time->high, cb->time->low);
  vhpi_printf("plugin: root name=%s case=%s full=%s kind=%d\n",
              vhpi_get_str(vhpiNameP, root), vhpi_get_str(vhpiCaseNameP, root),
              vhpi_get_str(vhpiFullNameP, root), (int)vhpi_get(vhpiKindP, root));
  vhpi_printf("plugin: same object=%d\n", vhpi_compare_handles(root, by_name));

  /* Check errors right after the failing call: any VHPI call, including
     vhpi_printf, clears the error. */
  vhpiHandleT missing = vhpi_handle_by_name("no_such", NULL);
  report_error("missing");
  vhpi_printf("plugin: missing=%s\n", missing ? "found" : "null");

  vhpi_printf("plugin: release=%d\n", vhpi_release_handle(by_name));
  int same = vhpi_compare_handles(root, by_name);
  report_error("stale");
  vhpi_printf("plugin: stale compare=%d\n", same);
  vhpi_printf("plugin: release again=%d\n", vhpi_release_handle(by_name));

  vhpiHandleT tool = vhpi_handle(vhpiTool, NULL);
  vhpi_printf("plugin: tool=%s\n", vhpi_get_str(vhpiNameP, tool));
  {
    vhpiHandleT it = vhpi_iterator(vhpiArgvs, tool);
    vhpiHandleT a;
    vhpi_printf("plugin: argc=%d\n", (int)vhpi_get(vhpiArgcP, tool));
    while (it && (a = vhpi_scan(it)) != NULL) {
      vhpi_printf("plugin: argv %s\n", vhpi_get_str(vhpiStrValP, a));
      vhpi_release_handle(a);
    }
  }
  vhpi_iterator(vhpiGenerateStmts, root); /* not implemented yet */
  report_error("iterator");
  report_error("check twice");

  vhpiPhysT res = vhpi_get_phys(vhpiResolutionLimitP, NULL);
  vhpi_printf("plugin: resolution=%d/%u ns=%d/%u hr=%d/%u\n", res.high, res.low,
              vhpiNS.high, vhpiNS.low, vhpiHR.high, vhpiHR.low);
  vhpi_release_handle(root);
  vhpi_release_handle(tool);

  vhpi_printf("plugin: finish=%d\n", vhpi_control(vhpiFinish, 1));
}

static char bye[] = "bye";

static void register_cbs(void) {
  vhpiCbDataT cb;
  memset(&cb, 0, sizeof cb);
  cb.reason = vhpiCbStartOfSimulation;
  cb.cb_rtn = start_of_sim;
  vhpiHandleT h = vhpi_register_cb(&cb, vhpiReturnCb);
  vhpi_printf("plugin: start cb %s, state=%d reason=%d\n", h ? "registered" : "failed",
              (int)vhpi_get(vhpiStateP, h), (int)vhpi_get(vhpiReasonP, h));

  cb.reason = vhpiCbEndOfSimulation;
  cb.cb_rtn = end_of_sim;
  cb.user_data = bye;
  vhpi_register_cb(&cb, 0);

  /* Disabled callbacks do not fire. */
  vhpiHandleT off = vhpi_register_cb(&cb, vhpiReturnCb | vhpiDisableCb);
  vhpi_printf("plugin: disabled cb state=%d\n", (int)vhpi_get(vhpiStateP, off));
  /* Removed callbacks do not fire either. */
  vhpiHandleT gone = vhpi_register_cb(&cb, vhpiReturnCb);
  vhpi_printf("plugin: remove=%d\n", vhpi_remove_cb(gone));

  vhpi_printf("plugin: %d exports referenced, printable(A)=%d printable(\\n)=%d\n",
              count_exports(),
              vhpi_is_printable('A'), vhpi_is_printable('\n'));
}

PLUGIN_EXPORT void (*vhpi_startup_routines[])(void) = {register_cbs, NULL};

/* For --vhpi lib:plugin_bootstrap. */
PLUGIN_EXPORT void plugin_bootstrap(void) {
  vhpi_printf("plugin: bootstrap entry\n");
  register_cbs();
}
