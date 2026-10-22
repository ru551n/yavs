/* A value-change callback registered from inside the value-change callback of another signal
 * that changes in the same cycle (#496), on two pairs: fa then fb, and fd then fc. A
 * simulator that reports its changed signals in declaration order reports fb after fa and fc
 * before fd. The second callback removes itself and registers a fresh one when it runs, as
 * Python testbench frameworks do. Each fire prints the signal, the time and the delta.
 * SPDX-License-Identifier: MIT OR Apache-2.0 */
#include "common.h"

typedef struct {
  const char *name[2];
  vhpiHandleT second;
} pair;

static pair pairs[2] = {{{"fa", "fb"}, NULL}, {{"fd", "fc"}, NULL}};

static vhpiHandleT watch(const char *name, void (*rtn)(const vhpiCbDataT *), void *data) {
  vhpiCbDataT cb;
  memset(&cb, 0, sizeof cb);
  cb.reason = vhpiCbValueChange;
  cb.cb_rtn = rtn;
  cb.obj = lookup(name);
  cb.user_data = (void *)data;
  return vhpi_register_cb(&cb, vhpiReturnCb);
}

static void fired(const char *name) {
  vhpiTimeT t;
  long cyc = 0;
  vhpi_get_time(&t, &cyc);
  T("%s time=%lldfs cycles=%ld\n", name, fs_of(&t), cyc);
}

static void on_second(const vhpiCbDataT *cb) {
  pair *p = (pair *)cb->user_data;
  fired(p->name[1]);
  vhpi_remove_cb(p->second);
  p->second = watch(p->name[1], on_second, p);
}

static void on_first(const vhpiCbDataT *cb) {
  pair *p = (pair *)cb->user_data;
  fired(p->name[0]);
  if (!p->second) p->second = watch(p->name[1], on_second, p);
}

static void body(void) {
  for (int i = 0; i < 2; i++) watch(pairs[i].name[0], on_first, &pairs[i]);
}

PLUGIN_START(body)
