/* Property values of selected objects, looked up by name.
 * SPDX-License-Identifier: MIT OR Apache-2.0 */
#include "common.h"

static const char *const names[] = {
    ":hier", "clk", "cnt", "flag", "bits", "slv", "rl", "ch", "st", "tm", "str", "k",
    ":hier:u_a", ":hier.u_a", "hier.u_a.r", "u_a.q", ":hier.blk.inner", "u_a:d", "u_a:q", "u_a:r", "u_a:wd", "g(1):u", "no_such", NULL,
};

#define IP(p) T("  %s=%d\n", #p, (int)vhpi_get(p, h))

/* Constraints (ranges) and enumeration literals of a type. */
static void ranges(vhpiHandleT t) {
  if (!t) return;
  vhpiHandleT it = vhpi_iterator(vhpiConstraints, t);
  T("  constraints it=%d unconstrained=%d\n", it != NULL, (int)vhpi_get(vhpiIsUnconstrainedP, t));
  err();
  for (vhpiHandleT c; it && (c = vhpi_scan(it));) {
    T("  range: unconstrained=%d left=%d right=%d up=%d null=%d\n",
      (int)vhpi_get(vhpiIsUnconstrainedP, c), (int)vhpi_get(vhpiLeftBoundP, c),
      (int)vhpi_get(vhpiRightBoundP, c), (int)vhpi_get(vhpiIsUpP, c),
      (int)vhpi_get(vhpiIsNullP, c));
    err();
  }
  it = vhpi_iterator(vhpiEnumLiterals, t);
  for (vhpiHandleT l; it && (l = vhpi_scan(it));)
    T("  literal: name=%s strval=%s\n", S(vhpiNameP, l), S(vhpiStrValP, l));
  err();
}

static void typeinfo(vhpiHandleT h) {
  vhpiHandleT t = vhpi_handle(vhpiType, h);
  if (!t) {
    T("  type: null err=%d\n", err());
    return;
  }
  T("  type kind=%s name=%s\n", S(vhpiKindStrP, t), S(vhpiNameP, t));
  vhpiHandleT b = vhpi_handle(vhpiBaseType, t);
  T("  base null=%d err=%d name=%s\n", b == NULL, err(), b ? S(vhpiNameP, b) : "-");
  ranges(t);
  ranges(b);
  vhpi_release_handle(t);
}

static void one(const char *name) {
  vhpiHandleT h = lookup(name);
  if (!h) {
    T("%s: not found err=%d\n", name, err());
    return;
  }
  T("%s: kind=%s name=%s case=%s full=%s\n", name, S(vhpiKindStrP, h), S(vhpiNameP, h),
    S(vhpiCaseNameP, h), S(vhpiFullNameP, h));
  IP(vhpiKindP); IP(vhpiSizeP); IP(vhpiIsCompositeP); IP(vhpiIsScalarP);
  IP(vhpiModeP); IP(vhpiStaticnessP); IP(vhpiNumDimensionsP); IP(vhpiLeftBoundP);
  IP(vhpiRightBoundP); IP(vhpiIsUpP); IP(vhpiIsForcedP); IP(vhpiIsLocalP);
  IP(vhpiNumLiteralsP); IP(vhpiStateP); IP(vhpiIsUnconstrainedP);
  T("  defname=%s file_null=%d line=%d\n", S(vhpiDefNameP, h),
    vhpi_get_str(vhpiFileNameP, h) == NULL, (int)vhpi_get(vhpiLineNoP, h));
  err();
  typeinfo(h);
  vhpi_release_handle(h);
}

static void body(void) {
  for (int i = 0; names[i]; i++) one(names[i]);
  /* Case-insensitive and relative lookups. */
  vhpiHandleT a = lookup("CNT"), b = lookup("cnt");
  T("case compare=%d\n", vhpi_compare_handles(a, b));
  /* Index into an array signal. */
  vhpiHandleT slv = lookup("slv");
  vhpiHandleT e = vhpi_handle_by_index(vhpiIndexedNames, slv, 2);
  T("index 2 null=%d err=%d", e == NULL, err());
  if (e) L(" name=%s kind=%s", S(vhpiNameP, e), S(vhpiKindStrP, e));
  L("\n");
  vhpiHandleT bad = vhpi_handle_by_index(vhpiIndexedNames, slv, 99);
  T("index 99 null=%d err=%d\n", bad == NULL, err());
  vhpiTimeT t;
  vhpi_get_time(&t, NULL);
  T("time null cycles ok=%d\n", fs_of(&t) == 0);
  vhpiPhysT r = vhpi_get_phys(vhpiResolutionLimitP, NULL);
  T("resolution fs=%d/%u\n", r.high, r.low);
  vhpi_control(vhpiFinish);
}

PLUGIN_START(body)
