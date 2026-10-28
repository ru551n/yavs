/* Test VHPI plugin for user attributes, tool and region properties, the foreign-model
 * callback reasons and vhpi_format_value (#143), loaded by `yavs run --vhpi=LIB:ENTRY`.
 * SPDX-License-Identifier: MIT OR Apache-2.0 */

#include <stdio.h>
#include <string.h>

#include "vhpi_user.h"

#ifdef _WIN32
#define PLUGIN_EXPORT __declspec(dllexport)
#else
#define PLUGIN_EXPORT
#endif

static int err(void) {
  vhpiErrorInfoT e;
  return vhpi_check_error(&e) != 0;
}

static void show_value(vhpiHandleT h) {
  char buf[64];
  vhpiValueT v;
  memset(&v, 0, sizeof v);
  v.format = vhpiObjTypeVal;
  v.bufSize = sizeof buf;
  v.value.str = (vhpiCharT *)buf;
  if (vhpi_get_value(h, &v) != 0) {
    vhpi_printf("    value: error\n");
  } else if (v.format == vhpiIntVal) {
    vhpi_printf("    value: int %d\n", v.value.intg);
  } else if (v.format == vhpiStrVal) {
    vhpi_printf("    value: str %s\n", buf);
  } else {
    vhpi_printf("    value: format %d\n", (int)v.format);
  }
}

/* attrs: user attributes of a signal and a constant, then properties. */
PLUGIN_EXPORT void attrs(void) {
  const char *objs[] = {"top:s", "top:c"};
  for (int i = 0; i < 2; i++) {
    vhpiHandleT o = vhpi_handle_by_name(objs[i], NULL);
    vhpiHandleT it = vhpi_iterator(vhpiAttrSpecs, o);
    vhpi_printf("%s: iterator %s error=%d\n", objs[i], it ? "yes" : "NULL", err());
    for (vhpiHandleT a; it && (a = vhpi_scan(it));) {
      vhpiHandleT d = vhpi_handle(vhpiAttrDecl, a);
      vhpi_printf("  %s %s full=%s decl=%s %s\n", vhpi_get_str(vhpiKindStrP, a),
                  vhpi_get_str(vhpiCaseNameP, a), vhpi_get_str(vhpiFullCaseNameP, a),
                  vhpi_get_str(vhpiKindStrP, d), vhpi_get_str(vhpiCaseNameP, d));
      show_value(a);
    }
  }
  vhpiHandleT by = vhpi_handle_by_name("top:s'weight", NULL);
  vhpi_printf("by name s'weight: %s\n", by ? (const char *)vhpi_get_str(vhpiKindStrP, by) : "NULL");

  vhpiHandleT root = vhpi_handle(vhpiRootInst, NULL);
  vhpi_printf("root: gens=%d ports=%d\n", vhpi_get(vhpiNumGensP, root),
              vhpi_get(vhpiNumPortsP, root));
  vhpiHandleT g = vhpi_handle_by_name("top:g1(3)", NULL);
  vhpi_printf("g1(3): label=%s index=%d line=%d\n", vhpi_get_str(vhpiLabelNameP, g),
              vhpi_get(vhpiGenerateIndexP, g), vhpi_get(vhpiStartLineNoP, g));
  vhpiHandleT v = vhpi_handle_by_name("top:p:v", NULL);
  vhpi_printf("p:v: shared=%d\n", vhpi_get(vhpiIsSharedP, v));
  vhpiHandleT tool = vhpi_handle(vhpiTool, NULL);
  vhpi_printf("tool: capabilities=%d\n", vhpi_get(vhpiCapabilitiesP, tool));
}

static unsigned now(void) {
  vhpiTimeT t;
  vhpi_get_time(&t, NULL);
  return t.low;
}

static int reps;
static vhpiHandleT rep_h;

static void say(const vhpiCbDataT *cb) {
  vhpi_printf("%s t=%u\n", (const char *)cb->user_data, now());
}

static void rep(const vhpiCbDataT *cb) {
  say(cb);
  if (++reps == 2) vhpi_remove_cb(rep_h);
}

static void sens(const vhpiCbDataT *cb) {
  vhpi_printf("sensitivity t=%u s=%d\n", now(), cb->value->value.enumv);
}

static vhpiHandleT reg(int reason, unsigned delay, vhpiHandleT obj, vhpiValueT *val,
                       void (*rtn)(const vhpiCbDataT *), const char *name) {
  vhpiCbDataT cb;
  vhpiTimeT t = {0, delay};
  memset(&cb, 0, sizeof cb);
  cb.reason = reason;
  cb.cb_rtn = rtn;
  cb.time = &t;
  cb.obj = obj;
  cb.value = val;
  cb.user_data = (void *)name;
  return vhpi_register_cb(&cb, vhpiReturnCb);
}

static vhpiValueT sval;

/* timeout: the foreign-model reasons vhpiCbTimeOut, vhpiCbRepTimeOut, vhpiCbSensitivity. */
PLUGIN_EXPORT void timeout(void) {
  reg(vhpiCbTimeOut, 5, NULL, NULL, say, "timeout");
  rep_h = reg(vhpiCbRepTimeOut, 8, NULL, NULL, rep, "rep");
  sval.format = vhpiEnumVal;
  reg(vhpiCbSensitivity, 0, vhpi_handle_by_name("top:s", NULL), &sval, sens, "sens");
}

static void fmt_int(vhpiFormatT to, int x) {
  char buf[80];
  vhpiValueT in, out;
  memset(&in, 0, sizeof in);
  memset(&out, 0, sizeof out);
  in.format = vhpiIntVal;
  in.value.intg = x;
  out.format = to;
  out.bufSize = sizeof buf;
  out.value.str = (vhpiCharT *)buf;
  int rc = vhpi_format_value(&in, &out);
  if (rc == 0 && to != vhpiRealVal)
    vhpi_printf("int %d -> %d: %s\n", x, (int)to, buf);
  else if (rc == 0)
    vhpi_printf("int %d -> real: %g\n", x, out.value.real);
  else
    vhpi_printf("int %d -> %d: rc=%d error=%d\n", x, (int)to, rc, err());
}

/* format: vhpi_format_value between formats without an object. */
PLUGIN_EXPORT void format(void) {
  fmt_int(vhpiBinStrVal, 10);
  fmt_int(vhpiHexStrVal, 255);
  fmt_int(vhpiDecStrVal, -42);
  fmt_int(vhpiRealVal, 3);

  char buf[80];
  vhpiValueT in, out;
  memset(&in, 0, sizeof in);
  memset(&out, 0, sizeof out);
  in.format = vhpiBinStrVal;
  in.value.str = (vhpiCharT *)"10X1";
  vhpiEnumT logic[8];
  out.format = vhpiLogicVecVal;
  out.bufSize = sizeof logic;
  out.value.enumvs = logic;
  int rc = vhpi_format_value(&in, &out);
  vhpi_printf("bin 10X1 -> logic vec: rc=%d n=%d %d %d %d %d\n", rc, out.numElems,
              logic[0], logic[1], logic[2], logic[3]);

  out.format = vhpiHexStrVal;
  out.bufSize = sizeof buf;
  out.value.str = (vhpiCharT *)buf;
  in.value.str = (vhpiCharT *)"10100101";
  rc = vhpi_format_value(&in, &out);
  vhpi_printf("bin 10100101 -> hex: rc=%d %s\n", rc, buf);

  in.format = vhpiRealVal;
  in.value.real = 1.5;
  out.format = vhpiBinStrVal;
  rc = vhpi_format_value(&in, &out);
  vhpi_printf("real -> bin: rc=%d error=%d\n", rc, err());

  rc = vhpi_format_value(NULL, &out);
  vhpi_printf("NULL: rc=%d error=%d\n", rc, err());
}
