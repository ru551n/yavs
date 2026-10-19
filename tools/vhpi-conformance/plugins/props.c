/* Property values of selected objects, looked up by name.
 * SPDX-License-Identifier: MIT OR Apache-2.0 */
#include "common.h"

static const char *const names[] = {
    ":hier", "clk", "cnt", "flag", "bits", "slv", "rl", "ch", "st", "tm", "str", "k",
    ":hier:u_a", "u_a:d", "u_a:q", "u_a:r", "u_a:wd", "g(1):u", "no_such", NULL,
};

#define IP(p) T("  %s=%d\n", #p, (int)vhpi_get(p, h))

static void typeinfo(vhpiHandleT h) {
  vhpiHandleT t = vhpi_handle(vhpiType, h);
  if (!t) {
    T("  type: null err=%d\n", err());
    return;
  }
  T("  type kind=%s name=%s\n", S(vhpiKindStrP, t), S(vhpiNameP, t));
  vhpiHandleT b = vhpi_handle(vhpiBaseType, t);
  T("  base null=%d err=%d name=%s\n", b == NULL, err(), b ? S(vhpiNameP, b) : "-");
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
  IP(vhpiNumLiteralsP); IP(vhpiStateP);
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
