/* vhpi_put_value of an integer (vhpiIntVal) into vectors: which bit pattern
 * each simulator takes, for a few values including negative ones and ones
 * that do not fit. Steps run from a chain of after-delay callbacks, 10 ns
 * apart; each step's puts are read back at the start of the next one.
 * SPDX-License-Identifier: MIT OR Apache-2.0 */
#include "common.h"

static vhpiHandleT h[4];
static const char *names[4] = {"v4d", "v4a", "b4", "v8"};
static const int vals[] = {3, 5, 8, 15, -1, -3, 16, 255, 256, 100000};

static void read_all(const char *tag) {
  now(tag);
  for (int k = 0; k < 4; k++) show(h[k], vhpiBinStrVal, names[k]);
}

static int step;

static void next(const vhpiCbDataT *cb) {
  (void)cb;
  vhpiTimeT t10 = {0, 0};
  t10.low = (uint32_t)(10000000 / vhpi_get_phys(vhpiResolutionLimitP, NULL).low);
  read_all("read");
  if (step > (int)(sizeof vals / sizeof *vals)) {
    vhpi_control(vhpiFinish);
    return;
  }
  if (step == (int)(sizeof vals / sizeof *vals)) { /* control: a bit string put is accepted */
    step++;
    char b[] = "10100101";
    vhpiValueT v;
    memset(&v, 0, sizeof v);
    v.format = vhpiBinStrVal;
    v.bufSize = sizeof b;
    v.numElems = 8;
    v.value.str = (vhpiCharT *)b;
    int rc = vhpi_put_value(h[3], &v, vhpiDepositPropagate);
    T("put v8 bits rc=%d err=%d\n", rc, err());
    vhpiCbDataT n;
    memset(&n, 0, sizeof n);
    n.reason = vhpiCbAfterDelay;
    n.cb_rtn = next;
    n.time = &t10;
    vhpi_register_cb(&n, 0);
    return;
  }
  int x = vals[step++];
  T("step %d value %d\n", step - 1, x);
  for (int k = 0; k < 4; k++) {
    vhpiValueT v;
    memset(&v, 0, sizeof v);
    v.format = vhpiIntVal;
    v.value.intg = x;
    int rc = vhpi_put_value(h[k], &v, vhpiDepositPropagate);
    T("put %s %d rc=%d err=%d\n", names[k], x, rc, err());
  }
  vhpiCbDataT n;
  memset(&n, 0, sizeof n);
  n.reason = vhpiCbAfterDelay;
  n.cb_rtn = next;
  n.time = &t10;
  vhpi_register_cb(&n, 0);
}

static void body(void) {
  for (int k = 0; k < 4; k++) h[k] = lookup(names[k]);
  T("handles %d %d %d %d\n", h[0] != NULL, h[1] != NULL, h[2] != NULL, h[3] != NULL);
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
