/* Hierarchy walk: every region, with its declarations and instances, in
 * iterator order. SPDX-License-Identifier: MIT OR Apache-2.0 */
#include "common.h"

static void line(const char *tag, int depth, vhpiHandleT h) {
  T("%*s%s kind=%d %s name=%s case=%s full=%s def=%s\n", depth * 2, "", tag,
    (int)vhpi_get(vhpiKindP, h), S(vhpiKindStrP, h), S(vhpiNameP, h),
    S(vhpiCaseNameP, h), S(vhpiFullNameP, h), S(vhpiDefNameP, h));
  err();
}

static void list(const char *tag, vhpiOneToManyT rel, vhpiHandleT region, int depth, int recurse) {
  vhpiHandleT it = vhpi_iterator(rel, region);
  if (!it) {
    T("%*s%s: no iterator err=%d\n", depth * 2, "", tag, err());
    return;
  }
  vhpiHandleT h;
  int n = 0;
  while ((h = vhpi_scan(it))) {
    line(tag, depth, h);
    n++;
    if (recurse) {
      list("generic", vhpiGenericDecls, h, depth + 1, 0);
      list("port", vhpiPortDecls, h, depth + 1, 0);
      list("sig", vhpiSigDecls, h, depth + 1, 0);
      list("const", vhpiConstDecls, h, depth + 1, 0);
      list("block", vhpiBlockStmts, h, depth + 1, 1);
      list("inst", vhpiCompInstStmts, h, depth + 1, 1);
      list("region", vhpiInternalRegions, h, depth + 1, 1);
    }
  }
  T("%*s%s: %d items\n", depth * 2, "", tag, n);
}

static void body(void) {
  vhpiHandleT root = vhpi_handle(vhpiRootInst, NULL);
  line("root", 0, root);
  list("generic", vhpiGenericDecls, root, 1, 0);
  list("port", vhpiPortDecls, root, 1, 0);
  list("sig", vhpiSigDecls, root, 1, 0);
  list("const", vhpiConstDecls, root, 1, 0);
  list("decl", vhpiDecls, root, 1, 0);
  list("block", vhpiBlockStmts, root, 1, 1);
  list("inst", vhpiCompInstStmts, root, 1, 1);
  list("region", vhpiInternalRegions, root, 1, 1);
  list("gen", vhpiGenerateStmts, root, 1, 1);
  list("stmt", vhpiStmts, root, 1, 0);
  /* Navigation back up. */
  vhpiHandleT s = vhpi_handle_by_name("hier:u_a:r", NULL);
  T("up: by_name null=%d err=%d\n", s == NULL, err());
  vhpiHandleT p = vhpi_handle_by_name(":hier:u_a", NULL);
  T("by_name :hier:u_a null=%d err=%d\n", p == NULL, err());
  if (p) {
    vhpiHandleT up = vhpi_handle(vhpiParent, p);
    T("parent null=%d err=%d name=%s\n", up == NULL, err(), up ? S(vhpiFullNameP, up) : "-");
    vhpiHandleT sc = vhpi_handle(vhpiUpperRegion, p);
    T("upper null=%d err=%d name=%s\n", sc == NULL, err(), sc ? S(vhpiFullNameP, sc) : "-");
    vhpiHandleT ds = vhpi_handle(vhpiDesignUnit, p);
    T("design unit null=%d err=%d kind=%s\n", ds == NULL, err(), ds ? S(vhpiKindStrP, ds) : "-");
  }
  vhpi_control(vhpiFinish);
}

PLUGIN_START(body)
