/* Test VHPI plugin: walks the design hierarchy at the start of simulation
 * and prints it, one object per line, then looks objects up by name and by
 * index. The output is compared with tests/hier/expected.txt.
 * SPDX-License-Identifier: MIT OR Apache-2.0 */

#include <stdio.h>
#include <string.h>

#include "vhpi_user.h"

#ifdef _WIN32
#define PLUGIN_EXPORT __declspec(dllexport)
#else
#define PLUGIN_EXPORT
#endif

/* One output line, printed with a single vhpi_printf. */
static char line[1024];
static size_t used;

#define put(...)                                                              \
  (used += (size_t)snprintf(line + used, sizeof line - used, __VA_ARGS__),    \
   used = used < sizeof line ? used : sizeof line - 1)

static void flush(void) {
  vhpi_printf("%s\n", line);
  used = 0;
  line[0] = 0;
}

static void indent(int depth) {
  for (int i = 0; i < depth; i++)
    put("  ");
}

static const char *str(vhpiStrPropertyT p, vhpiHandleT h) {
  const vhpiCharT *s = vhpi_get_str(p, h);
  return s ? (const char *)s : "(null)";
}

/* The file name without its directory: the test runs in a scratch dir. */
static const char *file(vhpiHandleT h) {
  const char *f = str(vhpiFileNameP, h);
  for (const char *c = f; *c; c++)
    if (*c == '/' || *c == '\\')
      f = c + 1;
  return f;
}

static void object(vhpiHandleT h, int depth);

/* The sub-elements of a composite object: record fields, then array
 * elements. */
static void elements(vhpiHandleT h, int depth) {
  static const vhpiOneToManyT rels[] = {vhpiSelectedNames, vhpiIndexedNames};
  for (size_t r = 0; r < sizeof rels / sizeof rels[0]; r++) {
    vhpiHandleT it = vhpi_iterator(rels[r], h);
    if (!it)
      continue;
    for (vhpiHandleT e; (e = vhpi_scan(it)) != NULL;) {
      vhpiHandleT prefix = vhpi_handle(vhpiPrefix, e);
      if (!vhpi_compare_handles(prefix, h))
        vhpi_printf("prefix of %s differs\n", str(vhpiFullNameP, e));
      vhpi_release_handle(prefix);
      object(e, depth);
      vhpi_release_handle(e);
    }
  }
}

static void object(vhpiHandleT h, int depth) {
  indent(depth);
  put("%s %s name=%s full=%s", str(vhpiKindStrP, h), str(vhpiCaseNameP, h),
      str(vhpiNameP, h), str(vhpiFullCaseNameP, h));
  int kind = vhpi_get(vhpiKindP, h);
  if (kind == vhpiPortDeclK)
    put(" mode=%d", (int)vhpi_get(vhpiModeP, h));
  if (kind != vhpiIndexedNameK && kind != vhpiSelectedNameK)
    put(" %s:%d", file(h), (int)vhpi_get(vhpiLineNoP, h));
  int composite = vhpi_get(vhpiIsCompositeP, h);
  put(" composite=%d size=%d", composite, (int)vhpi_get(vhpiSizeP, h));
  flush();
  if (composite == 1)
    elements(h, depth + 1);
}

/* Items of a statement relation; -1 when it does not apply (#316). */
static int count(vhpiOneToManyT rel, vhpiHandleT h) {
  vhpiHandleT it = vhpi_iterator(rel, h);
  if (!it)
    return vhpi_check_error(NULL) ? -1 : 0;
  int n = 0;
  for (vhpiHandleT s; (s = vhpi_scan(it)) != NULL; n++)
    vhpi_release_handle(s);
  return n;
}

static void region(vhpiHandleT h, int depth) {
  indent(depth);
  put("%s %s name=%s full=%s %s:%d", str(vhpiKindStrP, h),
      str(vhpiCaseNameP, h), str(vhpiNameP, h), str(vhpiFullNameP, h),
      file(h), (int)vhpi_get(vhpiLineNoP, h));
  vhpiHandleT du = vhpi_handle(vhpiDesignUnit, h);
  if (du) {
    put(" unit=%s:%s", str(vhpiKindStrP, du), str(vhpiCaseNameP, du));
    vhpiHandleT primary = vhpi_handle(vhpiPrimaryUnit, du);
    if (primary) {
      put(" of %s:%s", str(vhpiKindStrP, primary), str(vhpiCaseNameP, primary));
      vhpi_release_handle(primary);
    }
    put(" def=%s", str(vhpiDefNameP, h));
    vhpi_release_handle(du);
  }
  put(" blocks=%d insts=%d stmts=%d", count(vhpiBlockStmts, h),
      count(vhpiCompInstStmts, h), count(vhpiStmts, h));
  flush();

  static const vhpiOneToManyT decls[] = {vhpiGenericDecls, vhpiPortDecls,
                                         vhpiSigDecls, vhpiConstDecls,
                                         vhpiVarDecls};
  for (size_t r = 0; r < sizeof decls / sizeof decls[0]; r++) {
    vhpiHandleT it = vhpi_iterator(decls[r], h);
    if (!it)
      continue;
    for (vhpiHandleT d; (d = vhpi_scan(it)) != NULL;) {
      vhpiHandleT up = vhpi_handle(vhpiImmRegion, d);
      if (!vhpi_compare_handles(up, h))
        vhpi_printf("region of %s differs\n", str(vhpiFullNameP, d));
      vhpi_release_handle(up);
      object(d, depth + 1);
      vhpi_release_handle(d);
    }
  }
  vhpiHandleT it = vhpi_iterator(vhpiInternalRegions, h);
  if (!it)
    return;
  for (vhpiHandleT r; (r = vhpi_scan(it)) != NULL;) {
    vhpiHandleT up = vhpi_handle(vhpiUpperRegion, r);
    if (!vhpi_compare_handles(up, h))
      vhpi_printf("upper region of %s differs\n", str(vhpiFullNameP, r));
    vhpi_release_handle(up);
    region(r, depth + 1);
    vhpi_release_handle(r);
  }
}

static void lookup(const char *name, vhpiHandleT scope) {
  vhpiHandleT h = vhpi_handle_by_name(name, scope);
  vhpi_printf("lookup %s: %s\n", name,
              h ? str(vhpiFullCaseNameP, h) : "null");
  if (h)
    vhpi_release_handle(h);
}

static void by_index(vhpiOneToManyT rel, vhpiHandleT parent, int index) {
  vhpiHandleT h = vhpi_handle_by_index(rel, parent, index);
  vhpi_printf("index %d of %s: %s\n", index, str(vhpiCaseNameP, parent),
              h ? str(vhpiFullCaseNameP, h) : "null");
  if (h)
    vhpi_release_handle(h);
}

static void start_of_sim(const vhpiCbDataT *cb) {
  (void)cb;
  vhpiHandleT root = vhpi_handle(vhpiRootInst, NULL);
  region(root, 0);
  vhpiHandleT packs = vhpi_iterator(vhpiPackInsts, NULL);
  if (packs)
    for (vhpiHandleT p; (p = vhpi_scan(packs)) != NULL;) {
      if (strcmp(str(vhpiNameP, p), "STD:STANDARD") != 0)
        region(p, 0);
      vhpi_release_handle(p);
    }

  lookup(":top:g(1):u:reg", NULL);
  lookup(":TOP:G(1):U:P_MAIN:CNT", NULL);
  lookup("top:ig:e:init(0)", NULL);
  lookup("r.b", root);
  lookup("R.B(0)", root);
  lookup("Outs(1)(0)", root);
  lookup("counts(run)", root);
  lookup("m(1, 0)", root);
  lookup("\\Ext Sig\\", root);
  lookup("\\EXT SIG\\", root);
  lookup("@work:types_pkg:pkg_c", NULL);
  /* '.' also separates the levels of the hierarchy. */
  lookup(":top.g(1).u.reg", NULL);
  lookup(":Top.B1.Inner", NULL);
  lookup("top.ig.e.init(0)", NULL);
  lookup("b1.inner", root);
  lookup("g(0).u.d.a", root);
  lookup(":top.nope.reg", NULL);
  lookup("no_such", root);
  lookup("outs(2)", root);
  lookup("r.c", root);
  vhpiHandleT g = vhpi_handle_by_name("g(0)", root);
  lookup("local", g);
  lookup("u:d.a", g);
  vhpi_release_handle(g);

  vhpiHandleT outs = vhpi_handle_by_name("outs", root);
  by_index(vhpiIndexedNames, outs, 1);
  by_index(vhpiIndexedNames, outs, 2);
  vhpiHandleT r = vhpi_handle_by_name("r", root);
  by_index(vhpiSelectedNames, r, 1);
  by_index(vhpiIndexedNames, r, 0);
  by_index(vhpiInternalRegions, root, 2);

  /* Handles compare by object; a released handle is stale. */
  vhpiHandleT a = vhpi_handle_by_name(":top:r.b", NULL);
  vhpiHandleT b = vhpi_handle_by_index(vhpiSelectedNames, r, 1);
  vhpi_printf("same element=%d different=%d\n", vhpi_compare_handles(a, b),
              vhpi_compare_handles(a, r));
  vhpi_release_handle(a);
  vhpi_printf("stale=%d\n", vhpi_compare_handles(a, b));
  vhpi_release_handle(b);
  vhpi_release_handle(r);
  vhpi_release_handle(outs);
  vhpi_release_handle(root);
}

static void register_cbs(void) {
  vhpiCbDataT cb;
  memset(&cb, 0, sizeof cb);
  cb.reason = vhpiCbStartOfSimulation;
  cb.cb_rtn = start_of_sim;
  vhpi_register_cb(&cb, 0);
}

PLUGIN_EXPORT void (*vhpi_startup_routines[])(void) = {register_cbs, NULL};
