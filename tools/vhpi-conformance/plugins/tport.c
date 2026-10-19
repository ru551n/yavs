/* Value-change callbacks on an integer output port driven by a transport
 * delay (#347): the input port is written from a read-write (last known
 * delta) callback registered at the start of simulation, in a design with
 * no delta cycle at time 0, as Python testbench frameworks write, and again
 * at 20 ns; each change of the ports and of a counter is printed.
 * SPDX-License-Identifier: MIT OR Apache-2.0 */
#include "common.h"

static vhpiHandleT i_h, o_h, s_h;

static void put_i(int x) {
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiIntVal;
  v.value.intg = x;
  int rc = vhpi_put_value(i_h, &v, vhpiDepositPropagate);
  T("put i_int %d rc=%d err=%d\n", x, rc, err());
}

static void changed(const vhpiCbDataT *cb) {
  vhpiTimeT t;
  long cyc = 0;
  vhpi_get_time(&t, &cyc);
  T("value change %s=%d time=%lldfs cycles=%ld\n", (const char *)cb->user_data,
    cb->value->value.intg, fs_of(&t), cyc);
}

static void watch(vhpiHandleT h, const char *tag) {
  static vhpiValueT v[3];
  static int n;
  vhpiCbDataT cb;
  memset(&cb, 0, sizeof cb);
  v[n].format = vhpiIntVal;
  cb.reason = vhpiCbValueChange;
  cb.cb_rtn = changed;
  cb.obj = h;
  cb.value = &v[n++];
  cb.user_data = (void *)tag;
  vhpi_register_cb(&cb, 0);
}

static void later(const vhpiCbDataT *cb) {
  (void)cb;
  if (vhpi_get_phys(vhpiResolutionLimitP, NULL).low * (long long)cb->time->low >= 30000000) {
    vhpi_control(vhpiFinish);
    return;
  }
  put_i(7);
}

static void after(int ns) {
  static vhpiTimeT t[2];
  static int n;
  vhpiCbDataT cb;
  memset(&cb, 0, sizeof cb);
  t[n].low = (uint32_t)(ns * 1000000LL / vhpi_get_phys(vhpiResolutionLimitP, NULL).low);
  cb.reason = vhpiCbAfterDelay;
  cb.cb_rtn = later;
  cb.time = &t[n++];
  vhpi_register_cb(&cb, 0);
}

static void rw(const vhpiCbDataT *cb) {
  vhpiTimeT t;
  long cyc = 0;
  (void)cb;
  vhpi_get_time(&t, &cyc);
  T("read-write time=%lldfs cycles=%ld\n", fs_of(&t), cyc);
  put_i(0);
}

static void ro(const vhpiCbDataT *cb) {
  vhpiTimeT t;
  long cyc = 0;
  (void)cb;
  vhpi_get_time(&t, &cyc);
  T("read-only time=%lldfs cycles=%ld\n", fs_of(&t), cyc);
}

static void body(void) {
  i_h = lookup("i_int");
  o_h = lookup("o_int");
  s_h = lookup("s_int");
  T("handles %d %d %d\n", i_h != NULL, o_h != NULL, s_h != NULL);
  watch(i_h, "i_int");
  watch(o_h, "o_int");
  watch(s_h, "s_int");
  vhpiCbDataT cb;
  memset(&cb, 0, sizeof cb);
  cb.reason = vhpiCbLastKnownDeltaCycle;
  cb.cb_rtn = rw;
  vhpi_register_cb(&cb, 0);
  cb.reason = vhpiCbEndOfTimeStep;
  cb.cb_rtn = ro;
  vhpi_register_cb(&cb, 0);
  after(20);
  after(45);
}

PLUGIN_START(body)
