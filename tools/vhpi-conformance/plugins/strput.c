/* vhpi_put_value(vhpiStrVal) on string signals with fewer and with more
 * characters than the array has elements (#345): accepted or not, and which
 * elements change to what. Steps run from after-delay callbacks, 10 ns apart.
 * SPDX-License-Identifier: MIT OR Apache-2.0 */
#include "common.h"

static vhpiHandleT up_h, down_h;

/* The string as character codes, so a NUL or other filler shows. */
static void codes(vhpiHandleT h, const char *tag) {
  char buf[64];
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  memset(buf, 0, sizeof buf);
  v.format = vhpiStrVal;
  v.bufSize = sizeof buf;
  v.value.str = (vhpiCharT *)buf;
  int rc = vhpi_get_value(h, &v);
  T("  %s rc=%d err=%d n=%d", tag, rc, err(), (int)v.numElems);
  for (int i = 0; !rc && i < v.numElems && i < 63; i++) L(" %d", (unsigned char)buf[i]);
  L("\n");
}

static void put(vhpiHandleT h, const char *tag, const char *s) {
  char buf[64];
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  memset(buf, 0, sizeof buf);
  snprintf(buf, sizeof buf, "%s", s);
  v.format = vhpiStrVal;
  v.bufSize = strlen(buf) + 1;
  v.numElems = (int32_t)strlen(buf);
  v.value.str = (vhpiCharT *)buf;
  int rc = vhpi_put_value(h, &v, vhpiDepositPropagate);
  T("put %s '%s' rc=%d err=%d\n", tag, s, rc, err());
}

static int step;

static void next(const vhpiCbDataT *cb) {
  (void)cb;
  vhpiTimeT t10 = {0, 0};
  t10.low = (uint32_t)(10000000 / vhpi_get_phys(vhpiResolutionLimitP, NULL).low);
  T("step %d\n", step);
  codes(up_h, "up");
  codes(down_h, "down");
  switch (step++) {
    case 0: put(up_h, "up", "Testing"); put(down_h, "down", "Testing"); break;
    case 1: put(up_h, "up", "ab"); put(down_h, "down", "ab"); break;
    case 2: put(up_h, "up", "TooLongStr"); put(down_h, "down", ""); break;
    default: vhpi_control(vhpiFinish); return;
  }
  vhpiCbDataT n;
  memset(&n, 0, sizeof n);
  n.reason = vhpiCbAfterDelay;
  n.cb_rtn = next;
  n.time = &t10;
  vhpi_register_cb(&n, 0);
}

static void body(void) {
  up_h = lookup("up");
  down_h = lookup("down");
  T("handles %d %d\n", up_h != NULL, down_h != NULL);
  vhpiTimeT t5 = {0, 0};
  t5.low = (uint32_t)(5000000 / vhpi_get_phys(vhpiResolutionLimitP, NULL).low);
  vhpiCbDataT cb;
  memset(&cb, 0, sizeof cb);
  cb.reason = vhpiCbAfterDelay;
  cb.cb_rtn = next;
  cb.time = &t5;
  vhpi_register_cb(&cb, 0);
}

PLUGIN_START(body)
