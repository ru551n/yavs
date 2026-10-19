/* vhpi_put_value: deposit, force and release on a free and a driven
 * signal, each followed by reads at several points of the cycle. Steps run
 * from a chain of after-delay callbacks, 10 ns apart.
 * SPDX-License-Identifier: MIT OR Apache-2.0 */
#include "common.h"

static vhpiHandleT free_s, drv_s, int_s;

static void read_all(const char *tag) {
  now(tag);
  show(free_s, vhpiBinStrVal, "free");
  show(drv_s, vhpiBinStrVal, "drv");
  show(int_s, vhpiIntVal, "int");
  T("  forced free=%d drv=%d\n", (int)vhpi_get(vhpiIsForcedP, free_s),
    (int)vhpi_get(vhpiIsForcedP, drv_s));
  err();
}

static void ro(const vhpiCbDataT *cb) { read_all((const char *)cb->user_data); }
static void lk(const vhpiCbDataT *cb) { read_all((const char *)cb->user_data); }

static void on(int reason, const char *tag, vhpiTimeT *t) {
  vhpiCbDataT cb;
  memset(&cb, 0, sizeof cb);
  cb.reason = reason;
  cb.cb_rtn = reason == vhpiCbEndOfTimeStep ? ro : lk;
  cb.time = t;
  cb.user_data = (void *)tag;
  vhpi_register_cb(&cb, 0);
}

static void put_str(vhpiHandleT h, const char *s, vhpiPutValueModeT mode, const char *tag) {
  char buf[64];
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  snprintf(buf, sizeof buf, "%s", s);
  v.format = vhpiBinStrVal;
  v.bufSize = strlen(buf) + 1;
  v.numElems = (int32_t)strlen(buf);
  v.value.str = (vhpiCharT *)buf;
  int rc = vhpi_put_value(h, &v, mode);
  T("put %s '%s' mode=%d rc=%d err=%d\n", tag, s, (int)mode, rc, err());
}

static void put_int(vhpiHandleT h, int x, vhpiPutValueModeT mode, const char *tag) {
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiIntVal;
  v.value.intg = x;
  int rc = vhpi_put_value(h, &v, mode);
  T("put %s %d mode=%d rc=%d err=%d\n", tag, x, (int)mode, rc, err());
}

static void put_enum(vhpiHandleT h, int x, vhpiPutValueModeT mode, const char *tag) {
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiLogicVal;
  v.value.enumv = (vhpiEnumT)x;
  int rc = vhpi_put_value(h, &v, mode);
  T("put %s enum %d mode=%d rc=%d err=%d\n", tag, x, (int)mode, rc, err());
}

static int step;

static void next(const vhpiCbDataT *cb) {
  (void)cb;
  vhpiTimeT t10 = {0, 0};
  t10.low = (uint32_t)(10000000 / vhpi_get_phys(vhpiResolutionLimitP, NULL).low);
  T("step %d\n", step);
  read_all("before");
  switch (step++) {
    case 0: /* deposit with and without propagation */
      put_str(free_s, "1010", vhpiDeposit, "free");
      put_int(int_s, 5, vhpiDepositPropagate, "int");
      break;
    case 1:
      put_str(free_s, "0110", vhpiDepositPropagate, "free");
      put_enum(drv_s, 2, vhpiDepositPropagate, "drv"); /* 0 */
      break;
    case 2: /* force, then a driver event, then release */
      put_str(free_s, "1111", vhpiForce, "free");
      put_enum(drv_s, 3, vhpiForcePropagate, "drv"); /* 1 */
      break;
    case 3:
      put_str(free_s, "0000", vhpiDepositPropagate, "free (forced)");
      put_int(int_s, 9, vhpiForce, "int");
      break;
    case 4:
      put_str(free_s, "0000", vhpiRelease, "free");
      put_enum(drv_s, 0, vhpiRelease, "drv");
      put_int(int_s, 0, vhpiRelease, "int");
      break;
    case 5: /* bad arguments */
      put_str(free_s, "10", vhpiDepositPropagate, "free (wrong size)");
      put_str(free_s, "10x1", vhpiDepositPropagate, "free (bad digit)");
      put_int(free_s, 3, vhpiDepositPropagate, "free (wrong format)");
      put_str(free_s, "0101", vhpiSizeConstraint, "free (size constraint)");
      break;
    default:
      vhpi_control(vhpiFinish);
      return;
  }
  read_all("after put");
  on(vhpiCbLastKnownDeltaCycle, "last-known-delta", NULL);
  on(vhpiCbEndOfTimeStep, "end-of-step", NULL);
  vhpiCbDataT n;
  memset(&n, 0, sizeof n);
  n.reason = vhpiCbAfterDelay;
  n.cb_rtn = next;
  n.time = &t10;
  vhpi_register_cb(&n, 0);
}

static void body(void) {
  free_s = lookup("s_free");
  drv_s = lookup("s_drv");
  int_s = lookup("s_int");
  T("handles %d %d %d\n", free_s != NULL, drv_s != NULL, int_s != NULL);
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
