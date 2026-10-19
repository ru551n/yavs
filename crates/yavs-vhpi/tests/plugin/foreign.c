/* Test VHPI plugin: foreign functions and procedures registered with
 * vhpi_register_foreignf and called from tests/foreign/design.vhd. The execution
 * callbacks read the parameters through vhpi_iterator(vhpiParamDecls) and
 * vhpi_handle_by_index with vhpi_get_value, and return values with vhpi_put_value
 * on the call (the function result) or on out/inout parameters.
 * SPDX-License-Identifier: MIT OR Apache-2.0 */

#include <stdio.h>
#include <string.h>

#include "vhpi_user.h"

#ifdef _WIN32
#define PLUGIN_EXPORT __declspec(dllexport)
#else
#define PLUGIN_EXPORT
#endif

static vhpiHandleT param(const vhpiCbDataT *cb, int i) {
  return vhpi_handle_by_index(vhpiParamDecls, cb->obj, i);
}

static int get_int(const vhpiCbDataT *cb, int i) {
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiIntVal;
  if (vhpi_get_value(param(cb, i), &v)) vhpi_printf("get_int failed\n");
  return v.value.intg;
}

static void put_int(vhpiHandleT h, int x) {
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiIntVal;
  v.value.intg = x;
  if (vhpi_put_value(h, &v, vhpiDepositPropagate)) vhpi_printf("put_int failed\n");
}

static void add_exec(const vhpiCbDataT *cb) {
  /* The parameters, their names and modes, once. */
  static int shown;
  if (!shown++) {
    vhpiHandleT it = vhpi_iterator(vhpiParamDecls, cb->obj);
    vhpiHandleT p;
    vhpi_printf("add: call kind=%d name=%s\n", vhpi_get(vhpiKindP, cb->obj),
                (const char *)vhpi_get_str(vhpiNameP, cb->obj));
    while ((p = vhpi_scan(it)))
      vhpi_printf("add: param %s mode=%d kind=%d\n",
                  (const char *)vhpi_get_str(vhpiNameP, p), vhpi_get(vhpiModeP, p),
                  vhpi_get(vhpiKindP, p));
  }
  put_int(cb->obj, get_int(cb, 0) + get_int(cb, 1));
}

static void half_exec(const vhpiCbDataT *cb) {
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiRealVal;
  vhpi_get_value(param(cb, 0), &v);
  v.value.real /= 2;
  vhpi_put_value(cb->obj, &v, vhpiDepositPropagate);
}

static void later_exec(const vhpiCbDataT *cb) {
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiTimeVal;
  vhpi_get_value(param(cb, 0), &v);
  /* Times count in the resolution limit (femtoseconds by default). */
  long long t = ((long long)v.value.time.high << 32) | v.value.time.low;
  t += 1000000; /* 1 ns */
  v.value.time.high = (int)(t >> 32);
  v.value.time.low = (unsigned)t;
  vhpi_put_value(cb->obj, &v, vhpiDepositPropagate);
}

static void parity_exec(const vhpiCbDataT *cb) {
  vhpiEnumT bits[64];
  vhpiValueT v;
  int ones = 0, i;
  memset(&v, 0, sizeof v);
  v.format = vhpiLogicVecVal;
  v.bufSize = sizeof bits;
  v.value.enumvs = bits;
  if (vhpi_get_value(param(cb, 0), &v)) vhpi_printf("parity: get failed\n");
  for (i = 0; i < v.numElems; i++) ones += bits[i] == vhpi1;
  memset(&v, 0, sizeof v);
  v.format = vhpiLogicVal;
  v.value.enumv = (ones & 1) ? vhpi1 : vhpi0;
  vhpi_put_value(cb->obj, &v, vhpiDepositPropagate);
}

static void is_big_exec(const vhpiCbDataT *cb) {
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiEnumVal;
  v.value.enumv = get_int(cb, 0) > 10;
  vhpi_put_value(cb->obj, &v, vhpiDepositPropagate);
}

static void divmod_exec(const vhpiCbDataT *cb) {
  int a = get_int(cb, 0), b = get_int(cb, 1);
  put_int(param(cb, 2), a / b);
  put_int(param(cb, 3), a % b);
  /* Writing an in parameter is misuse. */
  {
    vhpiValueT v;
    vhpiErrorInfoT e;
    memset(&v, 0, sizeof v);
    v.format = vhpiIntVal;
    if (!vhpi_put_value(param(cb, 0), &v, vhpiDepositPropagate))
      vhpi_printf("divmod: in parameter written\n");
    if (vhpi_check_error(&e)) vhpi_printf("divmod: in parameter: %s\n", e.message);
  }
}

static void invert_exec(const vhpiCbDataT *cb) {
  vhpiEnumT bits[64];
  vhpiValueT v;
  int i;
  memset(&v, 0, sizeof v);
  v.format = vhpiLogicVecVal;
  v.bufSize = sizeof bits;
  v.value.enumvs = bits;
  vhpi_get_value(param(cb, 0), &v);
  for (i = 0; i < v.numElems; i++) bits[i] = bits[i] == vhpi1 ? vhpi0 : vhpi1;
  vhpi_put_value(param(cb, 0), &v, vhpiDepositPropagate);
}

static void bump_exec(const vhpiCbDataT *cb) { put_int(param(cb, 0), get_int(cb, 0) + 1); }

static void forgetful_exec(const vhpiCbDataT *cb) { (void)cb; }

static void elab(const vhpiCbDataT *cb) {
  vhpiForeignDataT d;
  if (vhpi_get_foreignf_info(cb->obj, &d))
    vhpi_printf("elab: no info\n");
  else
    vhpi_printf("elab: %s %s kind=%d\n", d.libraryName, d.modelName, (int)d.kind);
}

static void reg(vhpiForeignKindT kind, char *name, void (*exec)(const vhpiCbDataT *)) {
  vhpiForeignDataT d;
  vhpiHandleT h;
  memset(&d, 0, sizeof d);
  d.kind = kind;
  d.libraryName = "plug";
  d.modelName = name;
  d.elabf = elab;
  d.execf = exec;
  h = vhpi_register_foreignf(&d);
  if (!h) vhpi_printf("register %s failed\n", name);
}

PLUGIN_EXPORT void foreign_models(void) {
  reg(vhpiFuncF, "add", add_exec);
  reg(vhpiFuncF, "half", half_exec);
  reg(vhpiFuncF, "later", later_exec);
  reg(vhpiFuncF, "parity", parity_exec);
  reg(vhpiFuncF, "is_big", is_big_exec);
  reg(vhpiProcF, "divmod", divmod_exec);
  reg(vhpiProcF, "invert", invert_exec);
  reg(vhpiProcF, "bump", bump_exec);
  reg(vhpiFuncF, "forgetful", forgetful_exec);
}

/* yavs only: registration queries and misuse (other simulators crash or accept it). */
PLUGIN_EXPORT void foreign_extras(void) {
  vhpiForeignDataT d, back;
  vhpiHandleT h;
  vhpiErrorInfoT e;
  /* vhpi_get_foreignf_info returns the registration. */
  memset(&d, 0, sizeof d);
  d.kind = vhpiProcF;
  d.libraryName = "other";
  d.modelName = "info";
  d.execf = bump_exec;
  h = vhpi_register_foreignf(&d);
  memset(&back, 0, sizeof back);
  if (!h || vhpi_get_foreignf_info(h, &back))
    vhpi_printf("info: failed\n");
  else
    vhpi_printf("info: %s %s kind=%d elabf=%d execf=%d kindP=%d name=%s\n",
                back.libraryName, back.modelName, (int)back.kind, back.elabf != NULL,
                back.execf == bump_exec, vhpi_get(vhpiForeignKindP, h),
                (const char *)vhpi_get_str(vhpiNameP, h));
  /* Misuse: a duplicate, a missing execf, a NULL registration. */
  if (vhpi_register_foreignf(&d)) vhpi_printf("duplicate accepted\n");
  if (vhpi_check_error(&e)) vhpi_printf("duplicate: %s\n", e.message);
  d.modelName = "noexec";
  d.execf = NULL;
  if (vhpi_register_foreignf(&d)) vhpi_printf("noexec accepted\n");
  if (vhpi_check_error(&e)) vhpi_printf("noexec: %s\n", e.message);
  if (vhpi_register_foreignf(NULL)) vhpi_printf("NULL accepted\n");
}

PLUGIN_EXPORT void (*vhpi_startup_routines[])(void) = {foreign_models, NULL};
