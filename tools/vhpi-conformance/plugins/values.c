/* Every signal of the design read in every value format.
 * SPDX-License-Identifier: MIT OR Apache-2.0 */
#include "common.h"

static const char *const names[] = {"clk", "cnt", "flag", "bits", "slv", "rl", "ch", "st",
                                    "tm", "str", "k", "u_a:r", NULL};

static void body(void) {
  for (int i = 0; names[i]; i++) {
    vhpiHandleT h = lookup(names[i]);
    T("%s: null=%d\n", names[i], h == NULL);
    if (h) all_formats(h);
    vhpi_release_handle(h);
  }
  /* A too-small buffer reports the size needed. */
  vhpiHandleT s = lookup("slv");
  char tiny[2];
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiBinStrVal;
  v.bufSize = sizeof tiny;
  v.value.str = (vhpiCharT *)tiny;
  int rc = vhpi_get_value(s, &v);
  T("small buffer rc=%d err=%d\n", rc, err());
  /* After the clock ran: values at 25 ns, read at the end of the step. */
  vhpi_control(vhpiFinish);
}

PLUGIN_START(body)
