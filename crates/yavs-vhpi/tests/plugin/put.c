/* Test VHPI plugin for vhpi_put_value and value-change callbacks (#271, #272), run on
 * tests/put/design.vhd by `yavs run --vhpi=LIB:ENTRY`.
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

static void check(const char *what) {
  vhpiErrorInfoT e;
  if (vhpi_check_error(&e))
    vhpi_printf("vhpi: %s: error: %s\n", what, e.message);
  else
    vhpi_printf("vhpi: %s: ok\n", what);
}

static vhpiHandleT reg(int reason, unsigned delay, void (*rtn)(const vhpiCbDataT *),
                       vhpiHandleT obj, vhpiValueT *value, const char *name) {
  vhpiCbDataT cb;
  vhpiTimeT t = {0, delay};
  memset(&cb, 0, sizeof cb);
  cb.reason = reason;
  cb.cb_rtn = rtn;
  cb.obj = obj;
  cb.time = &t;
  cb.value = value;
  cb.user_data = (void *)name;
  return vhpi_register_cb(&cb, vhpiReturnCb);
}

static vhpiHandleT h(const char *name) {
  char full[64];
  snprintf(full, sizeof full, ":top:%s", name);
  vhpiHandleT x = vhpi_handle_by_name(full, NULL);
  if (!x)
    vhpi_printf("vhpi: no handle %s\n", name);
  return x;
}

static int put_int(vhpiHandleT x, int v, vhpiPutValueModeT mode) {
  vhpiValueT val;
  memset(&val, 0, sizeof val);
  val.format = vhpiIntVal;
  val.value.intg = v;
  return vhpi_put_value(x, &val, mode);
}

static int put_str(vhpiHandleT x, vhpiFormatT f, const char *s, vhpiPutValueModeT mode) {
  vhpiValueT val;
  memset(&val, 0, sizeof val);
  val.format = f;
  val.value.str = (vhpiCharT *)s;
  val.bufSize = strlen(s) + 1;
  return vhpi_put_value(x, &val, mode);
}

/* ---- clocked: the clock and the reset of the counter come from VHPI ---------------- */

static vhpiHandleT clk, rst, d, pa;

/* Value-change callbacks print the value yavs filled in, in the format they asked for. */
static char buf[5][64];
static vhpiValueT vals[5];

static void changed(const vhpiCbDataT *cb) {
  long c;
  unsigned t = now(&c);
  if (cb->value)
    vhpi_printf("vhpi: %s=%s t=%u d=%ld\n", (const char *)cb->user_data, cb->value->value.str,
                t, c);
  else
    vhpi_printf("vhpi: %s changed t=%u d=%ld\n", (const char *)cb->user_data, t, c);
}

static void watch(const char *name, int i) {
  vals[i].format = vhpiBinStrVal;
  vals[i].bufSize = sizeof buf[i];
  vals[i].value.str = (vhpiCharT *)buf[i];
  reg(vhpiCbValueChange, 0, changed, h(name), &vals[i], name);
}

static void tick(const vhpiCbDataT *cb) {
  (void)cb;
  /* Read the clock back and write its inverse as a logic value. */
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiLogicVal;
  vhpi_get_value(clk, &v);
  v.value.enumv = v.value.enumv == vhpi1 ? vhpi0 : vhpi1;
  vhpi_put_value(clk, &v, vhpiDepositPropagate);
}

static void at(const vhpiCbDataT *cb) {
  long c;
  (void)cb;
  unsigned t = now(&c);
  vhpi_printf("vhpi: at t=%u d=%ld\n", t, c);
  switch (t) {
  case 12:
    vhpi_printf("vhpi: deposit p.a=%d\n", put_int(pa, 7, vhpiDepositPropagate));
    break;
  case 15:
    vhpi_printf("vhpi: force d=%d\n", put_int(d, 100, vhpiForcePropagate));
    break;
  case 22:
    vhpi_printf("vhpi: rst=%d\n", put_str(rst, vhpiBinStrVal, "0", vhpiDepositPropagate));
    break;
  case 40:
    vhpi_printf("vhpi: release d=%d\n", put_int(d, 0, vhpiRelease));
    break;
  case 55:
    vhpi_printf("vhpi: deposit d=%d\n", put_int(d, 77, vhpiDepositPropagate));
    break;
  case 60:
    vhpi_printf("vhpi: force d=%d\n", put_str(d, vhpiDecStrVal, "-9", vhpiForcePropagate));
    break;
  case 65:
    vhpi_printf("vhpi: release d=%d\n", put_int(d, 0, vhpiRelease));
    break;
  case 100:
    vhpi_control(vhpiFinish);
    break;
  }
}

/* A put from the read-write phase shows up in the next delta at the same time. */
static int rw_done;
static void read_write(const vhpiCbDataT *cb) {
  long c;
  unsigned t = now(&c);
  (void)cb;
  if (t == 80 && !rw_done) {
    rw_done = 1;
    vhpi_printf("vhpi: read_write t=%u d=%ld rst=%d\n", t, c,
                put_str(rst, vhpiStrVal, "1", vhpiDepositPropagate));
  }
}

static void start(const vhpiCbDataT *cb) {
  static const unsigned times[] = {12, 15, 22, 40, 55, 60, 65, 100};
  (void)cb;
  clk = h("clk");
  rst = h("rst");
  d = h("d");
  pa = h("p.a");
  watch("q", 0);
  watch("u:q", 1);
  watch("q(0)", 2);
  watch("p.b", 3);
  reg(vhpiCbValueChange, 0, changed, h("p"), NULL, "p");
  reg(vhpiCbValueChange, 0, changed, d, NULL, "d");
  reg(vhpiCbRepAfterDelay, 5, tick, NULL, NULL, "tick");
  for (unsigned i = 0; i < sizeof times / sizeof times[0]; i++)
    reg(vhpiCbAfterDelay, times[i], at, NULL, NULL, "at");
  reg(vhpiCbRepLastKnownDeltaCycle, 0, read_write, NULL, NULL, "rw");
}

PLUGIN_EXPORT void clocked(void) {
  reg(vhpiCbStartOfSimulation, 0, start, NULL, NULL, "start");
}

/* ---- errors: what cannot be written, and when ------------------------------------- */

static void read_only(const vhpiCbDataT *cb) {
  (void)cb;
  put_int(h("d"), 1, vhpiDepositPropagate);
  check("put in read-only phase");
  vhpi_control(vhpiFinish);
}

static void errors_start(const vhpiCbDataT *cb) {
  vhpiValueT v;
  (void)cb;
  put_int(h("c"), 1, vhpiDepositPropagate);
  check("put constant");
  put_int(h("g"), 1, vhpiForcePropagate);
  check("put generic");
  put_int(h("pv:v"), 1, vhpiDepositPropagate);
  check("put variable");
  put_int(h("d"), 1, vhpiSizeConstraint);
  check("size constraint");
  put_int(h("d"), 1, vhpiDeposit);
  check("non-propagating deposit");
  put_int(h("d"), 1, vhpiForce);
  check("non-propagating force");
  put_str(h("q"), vhpiBinStrVal, "101", vhpiDepositPropagate);
  check("put too short");
  put_str(h("q"), vhpiBinStrVal, "10Q1", vhpiDepositPropagate);
  check("put bad literal");
  put_str(h("q"), vhpiHexStrVal, "1F", vhpiDepositPropagate);
  check("put too wide");
  put_int(h("p"), 1, vhpiDepositPropagate);
  check("put record");
  memset(&v, 0, sizeof v);
  v.format = vhpiRealVal;
  vhpi_put_value(h("d"), &v, vhpiDepositPropagate);
  check("put real into integer");
  vhpi_put_value(h("d"), NULL, vhpiDepositPropagate);
  check("put NULL value");
  put_str(h("q"), vhpiHexStrVal, "A", vhpiDepositPropagate);
  check("put hex");
  reg(vhpiCbValueChange, 0, read_only, h("c"), NULL, "vc");
  check("value change on constant");
  reg(vhpiCbEndOfTimeStep, 0, read_only, NULL, NULL, "ro");
}

PLUGIN_EXPORT void errors(void) {
  reg(vhpiCbStartOfSimulation, 0, errors_start, NULL, NULL, "start");
}

/* ---- strings: vhpiStrVal shorter and longer than the array (#345) -------------------- */

static int str_step;

static void str_at(const vhpiCbDataT *cb) {
  static const char *const texts[] = {"Testing", "ab", "TooLongStr"};
  char got[16];
  vhpiValueT v;
  (void)cb;
  memset(&v, 0, sizeof v);
  memset(got, 0, sizeof got);
  v.format = vhpiStrVal;
  v.bufSize = sizeof got;
  v.value.str = (vhpiCharT *)got;
  vhpi_get_value(h("str"), &v);
  char line[128];
  int len = snprintf(line, sizeof line, "vhpi: str n=%d", (int)v.numElems);
  for (int i = 0; i < v.numElems && i < 8; i++)
    len += snprintf(line + len, sizeof line - len, " %d", (unsigned char)got[i]);
  vhpi_printf("%s\n", line);
  if (str_step == 3) {
    vhpi_control(vhpiFinish);
    return;
  }
  put_str(h("str"), vhpiStrVal, texts[str_step], vhpiDepositPropagate);
  check(texts[str_step]);
  str_step++;
  reg(vhpiCbAfterDelay, 1, str_at, NULL, NULL, "str");
}

PLUGIN_EXPORT void strings(void) {
  reg(vhpiCbAfterDelay, 1, str_at, NULL, NULL, "str");
}

/* ---- edges: a value-change callback re-armed from inside itself (#346) --------------- */

static vhpiHandleT edge_h;

static void edge(const vhpiCbDataT *cb) {
  long c;
  unsigned t = now(&c);
  vhpi_printf("vhpi: e=%s t=%u d=%ld\n", cb->value->value.str, t, c);
  /* Remove this one and register a fresh one, as Python testbench frameworks do. */
  vhpi_remove_cb(edge_h);
  edge_h = reg(vhpiCbValueChange, 0, edge, h("e"), cb->value, "e");
}

PLUGIN_EXPORT void edges(void) {
  static char ebuf[16];
  static vhpiValueT ev;
  ev.format = vhpiBinStrVal;
  ev.bufSize = sizeof ebuf;
  ev.value.str = (vhpiCharT *)ebuf;
  edge_h = reg(vhpiCbValueChange, 0, edge, h("e"), &ev, "e");
}
