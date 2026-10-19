/* Variables read and deposited before their process has run (#342): at the start of
 * simulation, after initialization and later. A simulator without variable handles
 * prints "no handle" for each.
 * SPDX-License-Identifier: MIT OR Apache-2.0 */
#include "common.h"

static void var_get(const char *name) {
  vhpiHandleT h = lookup(name);
  if (!h) {
    T("  %s: no handle\n", name);
    err();
    return;
  }
  show(h, vhpiIntVal, name);
}

static void var_put(const char *name, int x) {
  vhpiHandleT h = lookup(name);
  vhpiValueT v;
  if (!h) {
    T("  put %s: no handle\n", name);
    err();
    return;
  }
  memset(&v, 0, sizeof v);
  v.format = vhpiIntVal;
  v.value.intg = x;
  int rc = vhpi_put_value(h, &v, vhpiDepositPropagate);
  T("  put %s %d rc=%d err=%d\n", name, x, rc, err());
}

static void dump(const char *tag) {
  T("%s\n", tag);
  var_get("p:n");
  var_get("unused");
  var_get("used");
  var_get("seen");
}

static void end_of_init(const vhpiCbDataT *cb) {
  (void)cb;
  dump("end of initialization");
}

static void at_1ns(const vhpiCbDataT *cb) {
  (void)cb;
  dump("1 ns");
}

static void body(void) {
  vhpiCbDataT cb;
  vhpiTimeT t = {0, 0};
  dump("start of simulation");
  var_put("p:n", 9);
  var_put("unused", 11);
  dump("after the deposits");
  memset(&cb, 0, sizeof cb);
  cb.reason = vhpiCbEndOfInitialization;
  cb.cb_rtn = end_of_init;
  vhpi_register_cb(&cb, 0);
  t.low = (uint32_t)(1000000 / vhpi_get_phys(vhpiResolutionLimitP, NULL).low);
  memset(&cb, 0, sizeof cb);
  cb.reason = vhpiCbAfterDelay;
  cb.cb_rtn = at_1ns;
  cb.time = &t;
  vhpi_register_cb(&cb, 0);
}

PLUGIN_START(body)
