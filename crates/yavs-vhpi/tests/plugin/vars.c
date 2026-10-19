/* Test VHPI plugin for vhpi_get_value and vhpi_put_value on process variables (#309), run
 * on tests/vars/design.vhd by `yavs run --vhpi=LIB:vars --resolution=ns`.
 * SPDX-License-Identifier: MIT OR Apache-2.0 */

#include <stdio.h>
#include <string.h>

#include "vhpi_user.h"

#ifdef _WIN32
#define PLUGIN_EXPORT __declspec(dllexport)
#else
#define PLUGIN_EXPORT
#endif

static void check(const char *what) {
  vhpiErrorInfoT e;
  if (vhpi_check_error(&e))
    vhpi_printf("vhpi: %s: error: %s\n", what, e.message);
  else
    vhpi_printf("vhpi: %s: ok\n", what);
}

static void reg(int reason, unsigned delay, void (*rtn)(const vhpiCbDataT *)) {
  vhpiCbDataT cb;
  vhpiTimeT t = {0, delay};
  memset(&cb, 0, sizeof cb);
  cb.reason = reason;
  cb.cb_rtn = rtn;
  cb.time = &t;
  vhpi_register_cb(&cb, 0);
}

static vhpiHandleT h(const char *name) {
  char full[64];
  snprintf(full, sizeof full, name[0] == '@' ? "%s" : ":top:%s", name);
  vhpiHandleT x = vhpi_handle_by_name(full, NULL);
  if (!x)
    vhpi_printf("vhpi: no handle %s\n", name);
  return x;
}

/* Prints variable `name` in `format`. */
static void show(const char *name, vhpiFormatT format) {
  char buf[64];
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = format;
  v.bufSize = sizeof buf;
  v.value.str = (vhpiCharT *)buf;
  if (vhpi_get_value(h(name), &v)) {
    check(name);
    return;
  }
  switch (format) {
  case vhpiIntVal:
  case vhpiEnumVal:
  case vhpiLogicVal:
    vhpi_printf("vhpi:   %s=%d\n", name, (int)v.value.intg);
    break;
  case vhpiRealVal:
    vhpi_printf("vhpi:   %s=%g\n", name, v.value.real);
    break;
  case vhpiTimeVal:
    vhpi_printf("vhpi:   %s=%u\n", name, v.value.time.low);
    break;
  default:
    vhpi_printf("vhpi:   %s=%s\n", name, buf);
  }
}

static void dump(const char *when) {
  vhpi_printf("vhpi: %s\n", when);
  show("p:n", vhpiIntVal);
  show("p:v", vhpiBinStrVal);
  show("p:v(0)", vhpiLogicVal);
  show("p:r.a", vhpiIntVal);
  show("p:r.b", vhpiBinStrVal);
  show("p:r.t", vhpiTimeVal);
  show("p:b", vhpiEnumVal);
  show("p:s", vhpiStrVal);
  show("p:x", vhpiRealVal);
  show("q:k", vhpiIntVal);
  show("sv", vhpiIntVal);
  show("un", vhpiIntVal);
  show("@work:pkg:pv", vhpiBinStrVal);
  show("@work:pkg:pv(3)", vhpiLogicVal);
}

static int put(const char *name, vhpiValueT *v, vhpiPutValueModeT mode) {
  return vhpi_put_value(h(name), v, mode);
}

static int put_int(const char *name, vhpiFormatT format, int x, vhpiPutValueModeT mode) {
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = format;
  v.value.intg = x;
  return put(name, &v, mode);
}

static int put_str(const char *name, vhpiFormatT format, const char *s) {
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = format;
  v.value.str = (vhpiCharT *)s;
  v.bufSize = strlen(s) + 1;
  return put(name, &v, vhpiDeposit);
}

static void read_only(const vhpiCbDataT *cb) {
  (void)cb;
  put_int("p:n", vhpiIntVal, 1, vhpiDeposit);
  check("deposit in a read-only phase");
}

static void at12(const vhpiCbDataT *cb) {
  vhpiValueT v;
  (void)cb;
  dump("t=12");
  put_int("p:n", vhpiIntVal, 100, vhpiDepositPropagate);
  put_str("p:v", vhpiBinStrVal, "10100101");
  put_int("p:r.a", vhpiIntVal, 42, vhpiDeposit);
  put_int("p:r.b(1)", vhpiLogicVal, vhpi1, vhpiDeposit);
  memset(&v, 0, sizeof v);
  v.format = vhpiTimeVal;
  v.value.time.low = 7;
  put("p:r.t", &v, vhpiDeposit);
  put_int("p:b", vhpiEnumVal, 1, vhpiDeposit);
  put_str("p:s", vhpiStrVal, "xyz");
  memset(&v, 0, sizeof v);
  v.format = vhpiRealVal;
  v.value.real = 2.25;
  put("p:x", &v, vhpiDeposit);
  put_int("q:k", vhpiIntVal, 10, vhpiDeposit);
  put_int("sv", vhpiIntVal, -3, vhpiDeposit);
  put_int("@work:pkg:pv(0)", vhpiLogicVal, vhpiX, vhpiDeposit);
  check("deposits");
  dump("t=12 after the deposits");
  put_int("p:n", vhpiIntVal, 1, vhpiForcePropagate);
  check("force a variable");
  put_int("p:n", vhpiIntVal, 0, vhpiRelease);
  check("release a variable");
  put_str("p:v", vhpiBinStrVal, "101");
  check("deposit too short");
  reg(vhpiCbEndOfTimeStep, 0, read_only);
}

static void end_of_init(const vhpiCbDataT *cb) {
  (void)cb;
  dump("end of initialization");
}

static void finish(const vhpiCbDataT *cb) {
  (void)cb;
  dump("t=30");
  vhpi_control(vhpiFinish);
}

static void start(const vhpiCbDataT *cb) {
  vhpiValueT v;
  (void)cb;
  memset(&v, 0, sizeof v);
  v.format = vhpiIntVal;
  dump("start of simulation");
  put_int("d:m", vhpiIntVal, 9, vhpiDeposit);
  put_int("un", vhpiIntVal, 8, vhpiDeposit);
  check("deposit before the process ran");
  show("d:m", vhpiIntVal);
  show("un", vhpiIntVal);
  vhpi_get_value(h("pc"), &v);
  check("read a protected object");
  reg(vhpiCbEndOfInitialization, 0, end_of_init);
  reg(vhpiCbAfterDelay, 12, at12);
  reg(vhpiCbAfterDelay, 30, finish);
}

PLUGIN_EXPORT void vars(void) {
  reg(vhpiCbStartOfSimulation, 0, start);
}
