/* vhpi_put_value with the non-propagating modes vhpiDeposit and vhpiForce:
 * accepted or not, the value read back, whether a value-change callback and
 * the process sensitive to the signal see an event, and the delta count.
 * Steps run from a chain of after-delay callbacks, 10 ns apart.
 * SPDX-License-Identifier: MIT OR Apache-2.0 */
#include "common.h"

static vhpiHandleT s_h, i_h, cnt_h;

static void read_all(const char *tag) {
  now(tag);
  show(s_h, vhpiBinStrVal, "s");
  show(i_h, vhpiIntVal, "i");
  show(cnt_h, vhpiIntVal, "cnt");
}

static void at(const vhpiCbDataT *cb) { read_all((const char *)cb->user_data); }

static void changed(const vhpiCbDataT *cb) {
  vhpiTimeT t;
  long cyc = 0;
  vhpi_get_time(&t, &cyc);
  T("value change %s time=%lldfs cycles=%ld\n", (const char *)cb->user_data, fs_of(&t), cyc);
}

static void on(int reason, const char *tag) {
  vhpiCbDataT cb;
  memset(&cb, 0, sizeof cb);
  cb.reason = reason;
  cb.cb_rtn = at;
  cb.user_data = (void *)tag;
  vhpi_register_cb(&cb, 0);
}

static void watch(vhpiHandleT h, const char *tag) {
  static vhpiValueT v;
  static char buf[16];
  vhpiCbDataT cb;
  memset(&cb, 0, sizeof cb);
  v.format = vhpiBinStrVal;
  v.bufSize = sizeof buf;
  v.value.str = (vhpiCharT *)buf;
  cb.reason = vhpiCbValueChange;
  cb.cb_rtn = changed;
  cb.obj = h;
  cb.value = &v;
  cb.user_data = (void *)tag;
  vhpi_register_cb(&cb, 0);
}

static void put_s(const char *bits, vhpiPutValueModeT mode) {
  char buf[8];
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  snprintf(buf, sizeof buf, "%s", bits);
  v.format = vhpiBinStrVal;
  v.bufSize = strlen(buf) + 1;
  v.numElems = (int32_t)strlen(buf);
  v.value.str = (vhpiCharT *)buf;
  int rc = vhpi_put_value(s_h, &v, mode);
  T("put s '%s' mode=%d rc=%d err=%d\n", bits, (int)mode, rc, err());
}

static void put_i(int x, vhpiPutValueModeT mode) {
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiIntVal;
  v.value.intg = x;
  int rc = vhpi_put_value(i_h, &v, mode);
  T("put i %d mode=%d rc=%d err=%d\n", x, (int)mode, rc, err());
}

static int step;

static void next(const vhpiCbDataT *cb) {
  (void)cb;
  vhpiTimeT t10 = {0, 0};
  t10.low = (uint32_t)(10000000 / vhpi_get_phys(vhpiResolutionLimitP, NULL).low);
  T("step %d\n", step);
  read_all("before");
  switch (step++) {
    case 0: put_s("1010", vhpiDeposit); break;
    case 1: put_i(5, vhpiDeposit); break;
    case 2: put_s("1111", vhpiForce); break;
    case 3: put_i(9, vhpiForce); break;
    case 4: put_s("0000", vhpiRelease); put_i(0, vhpiRelease); break;
    case 5: put_s("0101", vhpiDepositPropagate); break; /* the propagating form, for contrast */
    default:
      vhpi_control(vhpiFinish);
      return;
  }
  read_all("after put");
  on(vhpiCbLastKnownDeltaCycle, "last-known-delta");
  on(vhpiCbEndOfTimeStep, "end-of-step");
  vhpiCbDataT n;
  memset(&n, 0, sizeof n);
  n.reason = vhpiCbAfterDelay;
  n.cb_rtn = next;
  n.time = &t10;
  vhpi_register_cb(&n, 0);
}

static void body(void) {
  s_h = lookup("s");
  i_h = lookup("i");
  cnt_h = lookup("cnt");
  T("handles %d %d %d\n", s_h != NULL, i_h != NULL, cnt_h != NULL);
  watch(s_h, "s");
  watch(i_h, "i");
  vhpiCbDataT cb;
  memset(&cb, 0, sizeof cb);
  vhpiTimeT t5 = {0, 0};
  t5.low = (uint32_t)(5000000 / vhpi_get_phys(vhpiResolutionLimitP, NULL).low);
  cb.reason = vhpiCbAfterDelay;
  cb.cb_rtn = next;
  cb.time = &t5;
  vhpi_register_cb(&cb, 0);
}

PLUGIN_START(body)
