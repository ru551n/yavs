/* Value-change callbacks on a vector whose changed elements form several runs
 * in one cycle (#346): a repeating callback, and a one-shot style callback
 * that removes itself and registers a fresh one from inside the callback, as
 * Python testbench frameworks do. The first of those also registers a
 * callback on a scalar that changes in the same cycle. Each fire prints the
 * time, delta and the vector's value.
 * SPDX-License-Identifier: MIT OR Apache-2.0 */
#include "common.h"

static vhpiHandleT v_h, w_h, again_h;
static int w_done;

static void fired(const char *tag) {
  vhpiTimeT t;
  long cyc = 0;
  vhpi_get_time(&t, &cyc);
  char buf[16];
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  memset(buf, 0, sizeof buf);
  v.format = vhpiBinStrVal;
  v.bufSize = sizeof buf;
  v.value.str = (vhpiCharT *)buf;
  vhpi_get_value(v_h, &v);
  T("%s time=%lldfs cycles=%ld v=%s\n", tag, fs_of(&t), cyc, buf);
}

static vhpiHandleT watch(vhpiHandleT h, void (*rtn)(const vhpiCbDataT *)) {
  static vhpiValueT val;
  static char buf[16];
  vhpiCbDataT cb;
  memset(&cb, 0, sizeof cb);
  val.format = vhpiBinStrVal;
  val.bufSize = sizeof buf;
  val.value.str = (vhpiCharT *)buf;
  cb.reason = vhpiCbValueChange;
  cb.cb_rtn = rtn;
  cb.obj = h;
  cb.value = &val;
  return vhpi_register_cb(&cb, vhpiReturnCb);
}

static void on_w(const vhpiCbDataT *cb) {
  (void)cb;
  fired("w");
}

static void on_again(const vhpiCbDataT *cb) {
  (void)cb;
  fired("again");
  vhpi_remove_cb(again_h);
  again_h = watch(v_h, on_again);
  if (!w_done) {
    w_done = 1;
    watch(w_h, on_w);
  }
}

static void on_rep(const vhpiCbDataT *cb) {
  (void)cb;
  fired("rep");
}

static void body(void) {
  v_h = lookup("v");
  w_h = lookup("w");
  T("handles %d %d\n", v_h != NULL, w_h != NULL);
  watch(v_h, on_rep);
  again_h = watch(v_h, on_again);
}

PLUGIN_START(body)
